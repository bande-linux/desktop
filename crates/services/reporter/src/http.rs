use std::io;
use std::time::Duration;

use bande_https::ureq::{self, Agent};
use jiff::Timestamp;

use crate::config::SERVER;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(120);
const APP: &str = "reporter";
const BODY_LIMIT: u64 = 64 * 1024;
const LONGEST_RETRY_AFTER: u64 = 24 * 60 * 60;

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
    agent: Agent,
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

impl Session {
    pub fn with_report_server() -> Self {
        Self::with(SERVER, true)
    }

    #[cfg(test)]
    pub fn unencrypted(server: &str) -> Self {
        Self::with(server, false)
    }

    fn with(server: &str, https_only: bool) -> Self {
        Self {
            base: server.to_owned(),
            agent: bande_https::config(APP)
                .https_only(https_only)
                .http_status_as_error(false)
                .allow_non_standard_methods(true)
                .max_redirects(0)
                .timeout_connect(Some(CONNECT_TIMEOUT))
                .timeout_global(Some(TRANSFER_TIMEOUT))
                .build()
                .into(),
        }
    }

    fn exchange(&self, request: &Request) -> Result<Answer, ureq::Error> {
        let mut builder = ureq::http::Request::builder()
            .method(request.method)
            .uri(format!("{}{}", self.base, request.path));
        if let Some(token) = request.token {
            builder = builder.header("Authorization", format!("Bearer {token}"));
        }
        for (name, value) in request.headers {
            builder = builder.header(*name, *value);
        }
        let mut response = self.agent.run(builder.body(request.body.to_vec())?)?;
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

    pub fn send(&self, request: &Request) -> io::Result<Answer> {
        self.exchange(request).map_err(ureq::Error::into_io)
    }
}
