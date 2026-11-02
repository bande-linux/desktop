use std::future::{Future, poll_fn};
use std::io::{self, Write};
use std::pin::{Pin, pin};
use std::rc::Rc;
use std::sync::Arc;

use brotli_decompressor::DecompressorWriter;
use flate2::write::{GzDecoder, ZlibDecoder};
use hyper::body::{Body, Incoming};
use hyper::header::{
    ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE, HOST, HeaderMap, LOCATION, USER_AGENT,
};
use hyper::{StatusCode, Uri};
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use url::{Host, Position, Url};

use crate::reactor::Reactor;
use crate::stream::{Stream, dial};
use crate::{ContentType, FetchError, Response};

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
}

enum Outcome {
    Done(Response),
    Redirect(Url),
}

pub(crate) async fn fetch(network: Rc<Network>, mut url: Url) -> Result<Response, FetchError> {
    for _ in 0..=MAX_REDIRECTS {
        match exchange(&network, url).await? {
            Outcome::Done(response) => return Ok(response),
            Outcome::Redirect(next) => url = next,
        }
    }
    Err(FetchError::TooManyRedirects)
}

async fn exchange(network: &Network, url: Url) -> Result<Outcome, FetchError> {
    let secure = match url.scheme() {
        "https" => true,
        "http" => false,
        _ => return Err(FetchError::UnsupportedScheme),
    };
    let host = url.host().ok_or(FetchError::InvalidUrl)?.to_owned();
    let port = url.port_or_known_default().ok_or(FetchError::InvalidUrl)?;
    let request = hyper::Request::get(
        url[Position::BeforePath..Position::AfterQuery]
            .parse::<Uri>()
            .map_err(|_| FetchError::InvalidUrl)?,
    )
    .header(HOST, &url[Position::BeforeHost..Position::AfterPort])
    .header(USER_AGENT, AGENT)
    .header(ACCEPT, ACCEPTED)
    .header(ACCEPT_ENCODING, ENCODINGS)
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
        if let Some(target) = redirect(&url, status, response.headers())? {
            return Ok(Outcome::Redirect(target));
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| ContentType::parse(value.as_bytes()));
        let decoder = Decoder::for_headers(response.headers())?;
        let body = read_body(response.into_body(), decoder).await?;
        Ok(Outcome::Done(Response {
            url,
            status: status.as_u16(),
            content_type,
            body,
        }))
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
    if !matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
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
