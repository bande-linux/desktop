use std::hash::{BuildHasher, RandomState};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bande_https::ureq;
use base64::Engine;
use md5::Md5;
use sha2::Sha256;

const TIMEOUT: Duration = Duration::from_secs(30);
const BODY_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub user: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Network(String),
    Certificate(String),
    Unauthorized,
    Unsupported(String),
    Status { status: u16, url: String },
    PreconditionFailed,
    Xml(String),
    NotFound(String),
    Insecure(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(error) => write!(f, "cannot reach the server: {error}"),
            Self::Certificate(url) => write!(f, "the certificate of {url} is not valid"),
            Self::Unauthorized => write!(f, "the user name or password is wrong"),
            Self::Unsupported(schemes) => write!(f, "the server asks for {schemes} sign-in"),
            Self::Status { status, url } => write!(f, "{url} answered {status}"),
            Self::PreconditionFailed => write!(f, "the event changed on the server"),
            Self::Xml(error) => write!(f, "the server's answer is not valid: {error}"),
            Self::NotFound(what) => write!(f, "no calendar server found for {what}"),
            Self::Insecure(url) => write!(f, "{url} is not encrypted, so the password is not sent"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub body: String,
    pub etag: Option<String>,
    pub location: Option<String>,
    challenges: Vec<String>,
}

impl Response {
    pub fn success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn challenged(&self) -> bool {
        self.status == 401
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Algorithm {
    Md5,
    Sha256,
}

impl Algorithm {
    fn name(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha256 => "SHA-256",
        }
    }

    fn hash(self, text: &str) -> String {
        let hex = |bytes: &[u8]| bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        match self {
            Self::Md5 => {
                use md5::Digest;
                hex(&Md5::digest(text.as_bytes()))
            }
            Self::Sha256 => {
                use sha2::Digest;
                hex(&Sha256::digest(text.as_bytes()))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Digest {
    realm: String,
    nonce: String,
    opaque: Option<String>,
    algorithm: Algorithm,
    qop: bool,
    count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Scheme {
    Basic,
    Digest(Digest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Challenge {
    scheme: String,
    params: Vec<(String, String)>,
}

impl Challenge {
    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

fn split_outside_quotes(header: &str) -> Vec<String> {
    let mut items = vec![String::new()];
    let (mut quoted, mut escaped) = (false, false);
    for character in header.chars() {
        match character {
            ',' if !quoted => items.push(String::new()),
            _ => {
                if let Some(item) = items.last_mut() {
                    item.push(character);
                }
                match (character, escaped) {
                    ('\\', false) if quoted => escaped = true,
                    ('"', false) => quoted = !quoted,
                    _ => escaped = false,
                }
            }
        }
    }
    items
}

fn parameter(text: &str) -> Option<(String, String)> {
    let (key, value) = text.split_once('=')?;
    let value = value.trim();
    let value = match value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        Some(quoted) => quoted.replace("\\\"", "\"").replace("\\\\", "\\"),
        None => value.to_owned(),
    };
    Some((key.trim().to_ascii_lowercase(), value))
}

fn challenges(headers: &[String]) -> Vec<Challenge> {
    let mut found: Vec<Challenge> = Vec::new();
    for item in headers
        .iter()
        .flat_map(|header| split_outside_quotes(header))
    {
        let item = item.trim();
        let starts_scheme = item
            .split_once(char::is_whitespace)
            .map_or(!item.contains('='), |(first, _)| !first.contains('='));
        if item.is_empty() {
            continue;
        }
        if starts_scheme {
            let (scheme, rest) = item.split_once(char::is_whitespace).unwrap_or((item, ""));
            found.push(Challenge {
                scheme: scheme.to_ascii_lowercase(),
                params: parameter(rest).into_iter().collect(),
            });
        } else if let (Some(challenge), Some(param)) = (found.last_mut(), parameter(item)) {
            challenge.params.push(param);
        }
    }
    found
}

fn chosen(challenges: &[Challenge]) -> Option<Scheme> {
    let digest = |wanted: Algorithm| {
        challenges.iter().find_map(|challenge| {
            let algorithm = match challenge.param("algorithm").unwrap_or("MD5") {
                name if name.eq_ignore_ascii_case("MD5") => Algorithm::Md5,
                name if name.eq_ignore_ascii_case("SHA-256") => Algorithm::Sha256,
                _ => return None,
            };
            let qop = challenge.param("qop");
            let auth = qop.is_some_and(|qop| qop.split(',').any(|qop| qop.trim() == "auth"));
            (challenge.scheme == "digest" && algorithm == wanted && (qop.is_none() || auth))
                .then(|| {
                    Some(Scheme::Digest(Digest {
                        realm: challenge.param("realm")?.to_owned(),
                        nonce: challenge.param("nonce")?.to_owned(),
                        opaque: challenge.param("opaque").map(str::to_owned),
                        algorithm,
                        qop: auth,
                        count: 0,
                    }))
                })
                .flatten()
        })
    };
    digest(Algorithm::Sha256)
        .or_else(|| digest(Algorithm::Md5))
        .or_else(|| {
            challenges
                .iter()
                .any(|challenge| challenge.scheme == "basic")
                .then_some(Scheme::Basic)
        })
}

fn stale(challenges: &[Challenge]) -> bool {
    challenges.iter().any(|challenge| {
        challenge.scheme == "digest"
            && challenge
                .param("stale")
                .is_some_and(|stale| stale.eq_ignore_ascii_case("true"))
    })
}

#[derive(Debug, Clone)]
struct Session {
    origin: String,
    scheme: Scheme,
}

#[derive(Clone)]
pub struct Http {
    agent: ureq::Agent,
    credentials: Option<Credentials>,
    session: Arc<Mutex<Option<Session>>>,
}

fn classify(url: &str, error: &ureq::Error) -> Error {
    match bande_https::untrusted(error) {
        true => Error::Certificate(origin(url).to_owned()),
        false => Error::Network(error.to_string()),
    }
}

impl Http {
    pub fn new(credentials: Option<&Credentials>) -> Self {
        let agent = bande_https::config("calendar")
            .http_status_as_error(false)
            .allow_non_standard_methods(true)
            .max_redirects(0)
            .timeout_global(Some(TIMEOUT))
            .build()
            .new_agent();
        Self {
            agent,
            credentials: credentials.cloned(),
            session: Arc::new(Mutex::new(None)),
        }
    }

    fn authorization(&self, method: &str, url: &str) -> Option<String> {
        let credentials = self.credentials.as_ref()?;
        let mut session = self.session.lock().ok()?;
        let session = session
            .as_mut()
            .filter(|session| session.origin == origin(url))?;
        match &mut session.scheme {
            Scheme::Basic => {
                let pair = format!("{}:{}", credentials.user, credentials.password);
                Some(format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(pair)
                ))
            }
            Scheme::Digest(digest) => {
                digest.count += 1;
                Some(digest_header(digest, credentials, method, url))
            }
        }
    }

    pub fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<Response, Error> {
        let sent = self.authorization(method, url);
        let response = self.run(method, url, headers, body, sent.as_deref())?;
        if !response.challenged() {
            return Ok(response);
        }
        let Some(_) = &self.credentials else {
            return Err(Error::Unauthorized);
        };
        let offered = challenges(&response.challenges);
        if sent.is_some() && !stale(&offered) {
            return Err(Error::Unauthorized);
        }
        let Some(scheme) = chosen(&offered) else {
            let schemes = offered
                .iter()
                .map(|challenge| challenge.scheme.clone())
                .collect::<Vec<_>>();
            return Err(Error::Unsupported(schemes.join(", ")));
        };
        if !url.starts_with("https://") && !loopback(url) {
            return Err(Error::Insecure(origin(url).to_owned()));
        }
        if let Ok(mut session) = self.session.lock() {
            *session = Some(Session {
                origin: origin(url).to_owned(),
                scheme,
            });
        }
        let authorization = self.authorization(method, url);
        let response = self.run(method, url, headers, body, authorization.as_deref())?;
        match response.challenged() {
            true => Err(Error::Unauthorized),
            false => Ok(response),
        }
    }

    fn run(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
        authorization: Option<&str>,
    ) -> Result<Response, Error> {
        let mut builder = ureq::http::Request::builder().method(method).uri(url);
        if let Some(authorization) = authorization {
            builder = builder.header("Authorization", authorization);
        }
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        if body.is_some()
            && !headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("Content-Type"))
        {
            builder = builder.header("Content-Type", "application/xml; charset=utf-8");
        }
        let request = builder
            .body(body.unwrap_or_default().to_owned())
            .map_err(|error| Error::Network(error.to_string()))?;
        let mut response = match self.agent.run(request.clone()) {
            Err(ureq::Error::Io(_)) => self.agent.run(request),
            other => other,
        }
        .map_err(|error| classify(url, &error))?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let status = response.status().as_u16();
        let etag = header("ETag");
        let location = header("Location");
        let challenges = response
            .headers()
            .get_all("WWW-Authenticate")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .map(str::to_owned)
            .collect();
        let body = response
            .body_mut()
            .with_config()
            .limit(BODY_LIMIT)
            .read_to_string()
            .map_err(|error| classify(url, &error))?;
        Ok(Response {
            status,
            body,
            etag,
            location,
            challenges,
        })
    }
}

fn digest_header(digest: &Digest, credentials: &Credentials, method: &str, url: &str) -> String {
    let uri = match &url[origin(url).len()..] {
        "" => "/",
        path => path,
    };
    let hash = |text: String| digest.algorithm.hash(&text);
    let secret = hash(format!(
        "{}:{}:{}",
        credentials.user, digest.realm, credentials.password
    ));
    let request = hash(format!("{method}:{uri}"));
    let quote = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
    let mut header = format!(
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", algorithm={}",
        quote(&credentials.user),
        quote(&digest.realm),
        quote(&digest.nonce),
        quote(uri),
        digest.algorithm.name()
    );
    let response = match digest.qop {
        true => {
            let count = format!("{:08x}", digest.count);
            let cnonce = format!("{:016x}", RandomState::new().hash_one(&digest.nonce));
            header.push_str(&format!(", qop=auth, nc={count}, cnonce=\"{cnonce}\""));
            hash(format!(
                "{secret}:{}:{count}:{cnonce}:auth:{request}",
                digest.nonce
            ))
        }
        false => hash(format!("{secret}:{}:{request}", digest.nonce)),
    };
    header.push_str(&format!(", response=\"{response}\""));
    if let Some(opaque) = &digest.opaque {
        header.push_str(&format!(", opaque=\"{}\"", quote(opaque)));
    }
    header
}

fn loopback(url: &str) -> bool {
    let host = origin(url).split_once("://").map_or("", |(_, host)| host);
    let host = match host.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

pub fn origin(url: &str) -> &str {
    let after_scheme = url.find("://").map_or(0, |index| index + 3);
    match url[after_scheme..].find('/') {
        Some(index) => &url[..after_scheme + index],
        None => url,
    }
}

pub fn join(base: &str, href: &str) -> String {
    let href = href.trim();
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_owned();
    }
    if href.starts_with('/') {
        return format!("{}{href}", origin(base));
    }
    let directory = match base.rfind('/') {
        Some(index) if index >= origin(base).len() => &base[..=index],
        _ => return format!("{}/{href}", origin(base)),
    };
    format!("{directory}{href}")
}

pub fn same_resource(a: &str, b: &str) -> bool {
    let normal = |url: &str| {
        let decoded = percent_decode(url);
        decoded.trim_end_matches('/').to_owned()
    };
    normal(a) == normal(b)
}

pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let decoded = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match decoded {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
