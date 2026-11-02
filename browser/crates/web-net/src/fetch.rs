use std::cell::RefCell;
use std::future::{Future, poll_fn};
use std::io::{self, Write};
use std::pin::{Pin, pin};
use std::rc::Rc;
use std::sync::Arc;

use brotli_decompressor::DecompressorWriter;
use flate2::write::{GzDecoder, ZlibDecoder};
use hyper::body::{Body, Incoming};
use hyper::header::{
    ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE, COOKIE, HOST, HeaderMap, HeaderName,
    IF_MODIFIED_SINCE, IF_NONE_MATCH, LOCATION, SET_COOKIE, STRICT_TRANSPORT_SECURITY, USER_AGENT,
};
use hyper::{StatusCode, Uri};
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use url::{Host, Position, Url};

use crate::cache::Lookup;
use crate::cookies;
use crate::reactor::Reactor;
use crate::site::Site;
use crate::storage::{self, State};
use crate::stream::{Stream, dial};
use crate::{ContentType, FetchError, Request, Response};

pub const MAX_BODY: usize = 64 * 1024 * 1024;
const MAX_REDIRECTS: usize = 20;
const BROTLI_BUFFER: usize = 4096;
const AGENT: &str = concat!(
    "Mozilla/5.0 (Linux aarch64) Bande/",
    env!("CARGO_PKG_VERSION"),
    " (like Gecko)"
);
const ACCEPTED: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
const ENCODINGS: &str = "gzip, deflate, br";

pub(crate) struct Network {
    pub(crate) reactor: Rc<Reactor>,
    pub(crate) tls: Arc<ClientConfig>,
    pub(crate) state: RefCell<State>,
}

struct Head {
    status: StatusCode,
    headers: HeaderMap,
    body: Option<Vec<u8>>,
}

fn same_site(a: &Url, b: &Url) -> bool {
    Site::of(a).is_some() && Site::of(a) == Site::of(b)
}

pub(crate) async fn fetch(network: Rc<Network>, request: Request) -> Result<Response, FetchError> {
    let result = fetch_following(&network, request).await;
    network.state.borrow_mut().persist();
    result
}

async fn fetch_following(network: &Network, request: Request) -> Result<Response, FetchError> {
    let Request {
        mut url,
        top_level,
        initiator,
    } = request;
    if url.scheme() == "data" {
        return crate::data::fetch(url);
    }
    for _ in 0..=MAX_REDIRECTS {
        let now = storage::now();
        network.state.borrow_mut().hsts.upgrade(&mut url, now);
        let navigation = top_level.is_none();
        let top = top_level.clone().unwrap_or_else(|| url.clone());
        let site = Site::of(&top).ok_or(FetchError::InvalidUrl)?;
        let partition = site.as_str();
        let first_party = same_site(&url, &top);
        let context = cookies::Context {
            top_level: &site,
            first_party,
            same_site: first_party
                && initiator
                    .as_ref()
                    .is_none_or(|initiator| same_site(initiator, &url)),
            navigation,
        };
        let mut extra = Vec::new();
        let stale = {
            let mut state = network.state.borrow_mut();
            match state.cache.lookup(&partition, &url, now) {
                Lookup::Fresh(response) => return Ok(response),
                Lookup::Stale {
                    etag,
                    last_modified,
                } => {
                    extra.extend(etag.map(|etag| (IF_NONE_MATCH, etag)));
                    extra.extend(last_modified.map(|date| (IF_MODIFIED_SINCE, date)));
                    true
                }
                Lookup::Miss => false,
            }
        };
        let cookie = network
            .state
            .borrow_mut()
            .cookies
            .header(&url, context, now);
        let cookie_sent = cookie.is_some();
        extra.extend(cookie.map(|cookie| (COOKIE, cookie)));
        let head = exchange(network, &url, extra, |status, _| {
            !(redirect_status(status) || (stale && status == StatusCode::NOT_MODIFIED))
        })
        .await?;
        let now = storage::now();
        {
            let mut state = network.state.borrow_mut();
            for value in head.headers.get_all(SET_COOKIE) {
                if let Ok(value) = value.to_str() {
                    state.cookies.store(&url, value, context, now);
                }
            }
            if let Some(policy) = head
                .headers
                .get(STRICT_TRANSPORT_SECURITY)
                .and_then(|value| value.to_str().ok())
            {
                state.hsts.observe(&url, policy, now);
            }
        }
        if stale && head.status == StatusCode::NOT_MODIFIED {
            let revalidated =
                network
                    .state
                    .borrow_mut()
                    .cache
                    .revalidated(&partition, &url, &head.headers, now);
            match revalidated {
                Some(response) => return Ok(response),
                None => continue,
            }
        }
        if let Some(target) = redirect(&url, head.status, &head.headers)? {
            url = target;
            continue;
        }
        let response = Response {
            content_type: head
                .headers
                .get(CONTENT_TYPE)
                .and_then(|value| ContentType::parse(value.as_bytes())),
            body: head.body.unwrap_or_default(),
            status: head.status.as_u16(),
            url,
        };
        network.state.borrow_mut().cache.store(
            &partition,
            &response,
            &head.headers,
            cookie_sent,
            now,
        );
        return Ok(response);
    }
    Err(FetchError::TooManyRedirects)
}

fn redirect_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

async fn exchange(
    network: &Network,
    url: &Url,
    extra: Vec<(HeaderName, String)>,
    wants_body: impl FnOnce(StatusCode, &HeaderMap) -> bool,
) -> Result<Head, FetchError> {
    let secure = match url.scheme() {
        "https" => true,
        "http" => false,
        _ => return Err(FetchError::UnsupportedScheme),
    };
    let host = url.host().ok_or(FetchError::InvalidUrl)?.to_owned();
    let port = url.port_or_known_default().ok_or(FetchError::InvalidUrl)?;
    let mut builder = hyper::Request::get(
        url[Position::BeforePath..Position::AfterQuery]
            .parse::<Uri>()
            .map_err(|_| FetchError::InvalidUrl)?,
    )
    .header(HOST, &url[Position::BeforeHost..Position::AfterPort])
    .header(USER_AGENT, AGENT)
    .header(ACCEPT, ACCEPTED)
    .header(ACCEPT_ENCODING, ENCODINGS);
    for (name, value) in extra {
        builder = builder.header(name, value);
    }
    let request = builder
        .body(String::new())
        .map_err(|_| FetchError::InvalidUrl)?;
    let server_name = server_name(&host)?;
    let tcp = dial(host, port).await.map_err(FetchError::Connect)?;
    let stream = if secure {
        let session = rustls::ClientConnection::new(Arc::clone(&network.tls), server_name)
            .map_err(|error| FetchError::Connect(io::Error::other(error)))?;
        Stream::tls(tcp, &network.reactor, session)
    } else {
        Stream::plain(tcp, &network.reactor)
    }
    .map_err(FetchError::Connect)?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(stream)
        .await
        .map_err(FetchError::Http)?;
    let response = async move {
        let response = sender
            .send_request(request)
            .await
            .map_err(FetchError::Http)?;
        let status = response.status();
        let headers = response.headers().clone();
        if !wants_body(status, &headers) {
            return Ok(Head {
                status,
                headers,
                body: None,
            });
        }
        let decoder = Decoder::for_headers(&headers)?;
        let body = read_body(response.into_body(), decoder).await?;
        Ok(Head {
            status,
            headers,
            body: Some(body),
        })
    };
    let mut connection = pin!(connection);
    let mut response = pin!(response);
    let mut connection_done = false;
    poll_fn(|cx| {
        if !connection_done && connection.as_mut().poll(cx).is_ready() {
            connection_done = true;
        }
        response.as_mut().poll(cx)
    })
    .await
}

fn server_name(host: &Host<String>) -> Result<ServerName<'static>, FetchError> {
    match host {
        Host::Domain(domain) => {
            ServerName::try_from(domain.clone()).map_err(|_| FetchError::InvalidUrl)
        }
        Host::Ipv4(address) => Ok(ServerName::from(std::net::IpAddr::from(*address))),
        Host::Ipv6(address) => Ok(ServerName::from(std::net::IpAddr::from(*address))),
    }
}

fn redirect(url: &Url, status: StatusCode, headers: &HeaderMap) -> Result<Option<Url>, FetchError> {
    if !redirect_status(status) {
        return Ok(None);
    }
    let Some(location) = headers.get(LOCATION) else {
        return Ok(None);
    };
    let location = location.to_str().map_err(|_| FetchError::InvalidRedirect)?;
    let mut target = url
        .join(location)
        .map_err(|_| FetchError::InvalidRedirect)?;
    if !matches!(target.scheme(), "http" | "https") {
        return Err(FetchError::InvalidRedirect);
    }
    if target.fragment().is_none() {
        target.set_fragment(url.fragment());
    }
    Ok(Some(target))
}

async fn read_body(mut body: Incoming, mut decoder: Decoder) -> Result<Vec<u8>, FetchError> {
    if body
        .size_hint()
        .exact()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        return Err(FetchError::TooLarge);
    }
    let mut received = 0;
    while let Some(frame) = poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await {
        let Ok(data) = frame.map_err(FetchError::Http)?.into_data() else {
            continue;
        };
        received += data.len();
        if received > MAX_BODY {
            return Err(FetchError::TooLarge);
        }
        decoder.write_all(&data).map_err(decode_error)?;
    }
    decoder.finish().map_err(decode_error)
}

fn decode_error(error: io::Error) -> FetchError {
    if error.kind() == io::ErrorKind::FileTooLarge {
        FetchError::TooLarge
    } else {
        FetchError::BadEncoding
    }
}

struct Limited(Vec<u8>);

impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len() + bytes.len() > MAX_BODY {
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

enum Decoder {
    Identity(Limited),
    Gzip(Box<GzDecoder<Limited>>),
    Deflate(Box<ZlibDecoder<Limited>>),
    Brotli(Box<DecompressorWriter<Limited>>),
}

impl Decoder {
    fn for_headers(headers: &HeaderMap) -> Result<Self, FetchError> {
        let mut codings = headers
            .get_all(CONTENT_ENCODING)
            .iter()
            .flat_map(|value| value.as_bytes().split(|&byte| byte == b','))
            .map(|coding| coding.trim_ascii())
            .filter(|coding| !coding.is_empty() && !coding.eq_ignore_ascii_case(b"identity"));
        let Some(coding) = codings.next() else {
            return Ok(Self::Identity(Limited(Vec::new())));
        };
        if codings.next().is_some() {
            return Err(FetchError::BadEncoding);
        }
        let output = Limited(Vec::new());
        match coding.to_ascii_lowercase().as_slice() {
            b"gzip" | b"x-gzip" => Ok(Self::Gzip(Box::new(GzDecoder::new(output)))),
            b"deflate" => Ok(Self::Deflate(Box::new(ZlibDecoder::new(output)))),
            b"br" => Ok(Self::Brotli(Box::new(DecompressorWriter::new(
                output,
                BROTLI_BUFFER,
            )))),
            _ => Err(FetchError::BadEncoding),
        }
    }

    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        match self {
            Self::Identity(output) => output.write_all(bytes),
            Self::Gzip(decoder) => decoder.write_all(bytes),
            Self::Deflate(decoder) => decoder.write_all(bytes),
            Self::Brotli(decoder) => decoder.write_all(bytes),
        }
    }

    fn finish(self) -> io::Result<Vec<u8>> {
        match self {
            Self::Identity(output) => Ok(output.0),
            Self::Gzip(decoder) => decoder.finish().map(|output| output.0),
            Self::Deflate(decoder) => decoder.finish().map(|output| output.0),
            Self::Brotli(mut decoder) => {
                decoder.close()?;
                decoder
                    .into_inner()
                    .map(|output| output.0)
                    .map_err(|_| io::ErrorKind::InvalidData.into())
            }
        }
    }
}
