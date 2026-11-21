use std::io;
use std::path::Path;
use std::time::Duration;

use bande_https::ureq::config::ConfigBuilder;
use bande_https::ureq::typestate::AgentScope;
use bande_https::ureq::{self, Agent};
use jiff::Timestamp;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(120);
const APP: &str = "reporter";
const BODY_LIMIT: u64 = 64 * 1024;
const LONGEST_RETRY_AFTER: u64 = 24 * 60 * 60;

pub struct Target {
    host: String,
    port: u16,
}

pub fn parse_endpoint(endpoint: &str) -> Option<Target> {
    let authority = endpoint.strip_prefix("https://")?;
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse().ok()?),
        None => (authority, 443),
    };
    (!host.is_empty() && !host.contains('/')).then(|| Target {
        host: host.to_owned(),
        port,
    })
}

pub struct Answer {
    pub status: u16,
    pub retry_after: Option<Duration>,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

pub struct Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub token: Option<&'a str>,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: &'a [u8],
}

pub struct Session {
    base: String,
    pinned: Option<Agent>,
    system: Agent,
}

fn retry_after(value: &str) -> Option<Duration> {
    let value = value.trim();
    let seconds = match value.parse::<u64>() {
        Ok(seconds) => seconds,
        Err(_) => {
            let time: Timestamp = jiff::fmt::rfc2822::parse(value).ok()?.timestamp();
            time.as_second()
                .saturating_sub(Timestamp::now().as_second())
                .max(0)
                .unsigned_abs()
        }
    };
    Some(Duration::from_secs(seconds.min(LONGEST_RETRY_AFTER)))
}

fn agent(config: ConfigBuilder<AgentScope>) -> Agent {
    config
        .https_only(true)
        .http_status_as_error(false)
        .allow_non_standard_methods(true)
        .max_redirects(0)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(TRANSFER_TIMEOUT))
        .build()
        .into()
}

impl Session {
    pub fn new(target: Target, ca: Option<&Path>) -> Self {
        let pinned = ca.and_then(|ca| {
            bande_https::pinned_config(APP, ca)
                .inspect_err(|error| {
                    tracing::info!(
                        "cannot read the pinned roots {}, so the system's roots decide: {error}",
                        ca.display()
                    );
                })
                .ok()
                .map(agent)
        });
        Self {
            base: format!("https://{}:{}", target.host, target.port),
            pinned,
            system: agent(bande_https::config(APP)),
        }
    }

    fn exchange(&self, agent: &Agent, request: &Request) -> Result<Answer, ureq::Error> {
        let mut builder = ureq::http::Request::builder()
            .method(request.method)
            .uri(format!("{}{}", self.base, request.path));
        if let Some(token) = request.token {
            builder = builder.header("Authorization", format!("Bearer {token}"));
        }
        for (name, value) in request.headers {
            builder = builder.header(*name, *value);
        }
        let mut response = agent.run(builder.body(request.body.to_vec())?)?;
        let retry_after = response
            .headers()
            .get("Retry-After")
            .and_then(|value| value.to_str().ok())
            .and_then(retry_after);
        let body = response
            .body_mut()
            .with_config()
            .limit(BODY_LIMIT)
            .read_to_vec()?;
        Ok(Answer {
            status: response.status().as_u16(),
            retry_after,
            body,
        })
    }

    pub fn send(&mut self, request: &Request) -> io::Result<Answer> {
        let answer = match &self.pinned {
            Some(pinned) => match self.exchange(pinned, request) {
                Err(error) if bande_https::untrusted(&error) => {
                    tracing::info!(
                        "the report server's certificate is not from the pinned roots, so the system's roots decide: {error}"
                    );
                    self.pinned = None;
                    self.exchange(&self.system, request)
                }
                answered => answered,
            },
            None => self.exchange(&self.system, request),
        };
        answer.map_err(ureq::Error::into_io)
    }
}
