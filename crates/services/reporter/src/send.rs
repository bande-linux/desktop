use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::{Map, Value, json};

use crate::config::Config;
use crate::http::{Answer, Request, Session, parse_endpoint};
use crate::identity::{self, Identity};
use crate::spool::{MANIFEST, STATE, Spool, Status, Stored};

const REPORTS: &str = "/v1/reports";
const CHECKINS: &str = "/v1/checkins";
const ENROLL: &str = "/v1/enroll";
const ROTATE: &str = "/v1/rotate";
const ME: &str = "/v1/me";
const JSON: &str = "Content-Type: application/json\r\n";
const ATTEMPTS: usize = 2;

pub enum Outcome {
    Stored,
    NotSupported,
    BadFormat,
    Unauthorized,
    UnknownToken,
    Revoked,
    TooLarge,
    OverBudget,
    Later(Option<Duration>),
    NoRoute(io::Error),
}

fn outcome_of(answer: &Answer) -> Outcome {
    let code = answer
        .json()
        .and_then(|body| body.get("code").and_then(Value::as_str).map(str::to_owned));
    match (answer.status, code.as_deref()) {
        (200 | 201, _) => Outcome::Stored,
        (400, _) => {
            tracing::warn!(
                "the server says a request is malformed: {}",
                String::from_utf8_lossy(&answer.body)
            );
            Outcome::BadFormat
        }
        (401, Some("unknown_token")) => Outcome::UnknownToken,
        (401, Some("revoked")) => Outcome::Revoked,
        (401 | 403, _) => Outcome::Unauthorized,
        (404, _) => Outcome::NotSupported,
        (413, _) => Outcome::TooLarge,
        _ => Outcome::Later(answer.retry_after),
    }
}

fn boundary(id: &str) -> String {
    format!("bande-report-{id}")
}

fn is_attachment(name: &str) -> bool {
    name != MANIFEST && name != STATE && !name.starts_with('.')
}

struct Body {
    bytes: Vec<u8>,
    attachments: Vec<String>,
}

fn body_of(
    report: &Stored,
    machine: Option<&str>,
    counts_only: bool,
    without: &[String],
) -> io::Result<Body> {
    let mut manifest = report.manifest.clone();
    manifest.insert("occurrences".to_owned(), json!(report.occurrences));
    manifest.remove("owner_uid");
    manifest.remove("machine");
    if let Some(machine) = machine {
        manifest.insert("machine".to_owned(), json!(machine));
    }
    let mut attachments: Vec<(String, Vec<u8>)> = Vec::new();
    if report.status != Status::Sent && !counts_only {
        for entry in fs::read_dir(&report.path)?.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if is_attachment(&name) && !without.contains(&name) {
                attachments.push((name, fs::read(entry.path())?));
            }
        }
    }
    if !without.is_empty() {
        let mut dropped: Vec<Value> = manifest
            .get("attachments_dropped")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        dropped.extend(without.iter().map(|name| json!(name)));
        manifest.insert("attachments_dropped".to_owned(), Value::Array(dropped));
    }
    attachments.sort_by_key(|(_, bytes)| bytes.len());
    let names = attachments.iter().map(|(name, _)| name.clone()).collect();
    let boundary = boundary(&report.id);
    let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
    write!(
        gzip,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"manifest\"\r\nContent-Type: application/json\r\n\r\n"
    )?;
    gzip.write_all(&serde_json::to_vec(&manifest)?)?;
    gzip.write_all(b"\r\n")?;
    for (name, bytes) in &attachments {
        write!(
            gzip,
            "--{boundary}\r\nContent-Disposition: form-data; name=\"attachment\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )?;
        gzip.write_all(bytes)?;
        gzip.write_all(b"\r\n")?;
    }
    write!(gzip, "--{boundary}--\r\n")?;
    Ok(Body {
        bytes: gzip.finish()?,
        attachments: names,
    })
}

pub fn attachment_bytes(report: &Stored) -> u64 {
    fs::read_dir(&report.path).map_or(0, |entries| {
        entries
            .flatten()
            .filter(|entry| entry.file_name().to_str().is_some_and(is_attachment))
            .filter_map(|entry| entry.metadata().ok())
            .map(|metadata| metadata.len())
            .sum()
    })
}

enum Credential {
    Token(String),
    SharedKey(String, String),
}

pub struct Sender<'a> {
    session: Session,
    spool: &'a Spool,
    shared_key: Option<String>,
    pub identity: Identity,
    pub contacted: bool,
}

impl<'a> Sender<'a> {
    pub fn new(config: &Config, spool: &'a Spool) -> Option<Self> {
        let endpoint = config.endpoint.as_deref()?;
        let Some(target) = parse_endpoint(endpoint) else {
            tracing::warn!("the report server's address is not https://host:port");
            return None;
        };
        let identity = match Identity::load(spool.root()) {
            Some(identity) if identity.endpoint == endpoint => identity,
            Some(_) => {
                if let Err(error) = forget_here(spool) {
                    tracing::warn!("cannot forget the identity for the old server: {error}");
                }
                Identity::new(endpoint)
            }
            None => Identity::new(endpoint),
        };
        Some(Self {
            session: Session::new(target, config.ca.as_deref()),
            spool,
            shared_key: config.key.clone(),
            identity,
            contacted: false,
        })
    }

    fn save(&self) {
        if let Err(error) = self.identity.save(self.spool.root()) {
            tracing::warn!("cannot keep this computer's report identity: {error}");
        }
    }

    fn exchange(&mut self, request: &Request) -> io::Result<Answer> {
        let answer = self.session.send(request)?;
        self.contacted = true;
        if let Some(body) = answer.json() {
            let before = self.identity.clone();
            self.identity.learn(&body);
            if self.identity != before {
                self.save();
            }
        }
        Ok(answer)
    }

    fn credential(&self) -> io::Result<Option<Credential>> {
        if self.identity.revoked {
            return Ok(None);
        }
        if let Some(credentials) = &self.identity.credentials {
            return Ok(Some(Credential::Token(credentials.token.clone())));
        }
        match &self.shared_key {
            Some(key) => Ok(Some(Credential::SharedKey(
                key.clone(),
                self.spool.install_id()?,
            ))),
            None => Ok(None),
        }
    }

    fn enroll(&mut self) -> Outcome {
        for _ in 0..ATTEMPTS {
            let challenge = match self.exchange(&Request {
                method: "GET",
                path: ENROLL,
                token: None,
                headers: "",
                body: b"",
            }) {
                Ok(answer) if answer.status == 200 => answer.json().unwrap_or_default(),
                Ok(answer) => return outcome_of(&answer),
                Err(error) => return Outcome::NoRoute(error),
            };
            let text = challenge.get("challenge").and_then(Value::as_str);
            let difficulty = challenge
                .get("difficulty")
                .and_then(Value::as_u64)
                .and_then(|bits| u32::try_from(bits).ok());
            let (Some(text), Some(difficulty)) = (text, difficulty) else {
                return Outcome::BadFormat;
            };
            let Some(nonce) = identity::solve(text, difficulty) else {
                tracing::warn!("the report server asks for too much work to enroll");
                return Outcome::Later(None);
            };
            let body = json!({"challenge": text, "nonce": nonce}).to_string();
            let answer = match self.exchange(&Request {
                method: "POST",
                path: ENROLL,
                token: None,
                headers: JSON,
                body: body.as_bytes(),
            }) {
                Ok(answer) => answer,
                Err(error) => return Outcome::NoRoute(error),
            };
            match answer.status {
                403 => continue,
                201 if self
                    .identity
                    .take_credentials(&answer.json().unwrap_or_default()) =>
                {
                    self.save();
                    return Outcome::Stored;
                }
                201 => return Outcome::BadFormat,
                _ => return outcome_of(&answer),
            }
        }
        Outcome::Later(None)
    }

    fn rotate(&mut self) {
        let Some(token) = self
            .identity
            .credentials
            .as_ref()
            .map(|credentials| credentials.token.clone())
        else {
            return;
        };
        let answer = self.exchange(&Request {
            method: "POST",
            path: ROTATE,
            token: Some(&token),
            headers: "",
            body: b"",
        });
        match answer {
            Ok(answer) if answer.status == 201 => {
                if self
                    .identity
                    .take_credentials(&answer.json().unwrap_or_default())
                {
                    self.save();
                }
            }
            Ok(answer) => self.lost(&outcome_of(&answer)),
            Err(error) => tracing::debug!("cannot rotate the report token: {error}"),
        }
    }

    fn lost(&mut self, outcome: &Outcome) {
        match outcome {
            Outcome::UnknownToken => self.identity.credentials = None,
            Outcome::Revoked => {
                tracing::warn!("the report server has revoked this computer");
                self.identity.credentials = None;
                self.identity.revoked = true;
            }
            _ => return,
        }
        self.save();
    }

    pub fn prepare(&mut self) -> Result<(), Outcome> {
        if self.identity.revoked {
            return Err(Outcome::Revoked);
        }
        if self.identity.credentials.is_some() {
            if self.identity.rotation_due() {
                self.rotate();
            }
            return Ok(());
        }
        let enrolled = self.enroll();
        if self.identity.credentials.is_some() || self.shared_key.is_some() {
            return Ok(());
        }
        Err(match enrolled {
            Outcome::Stored | Outcome::BadFormat => Outcome::Later(None),
            other => other,
        })
    }

    fn authorised(
        &mut self,
        path: &str,
        headers: &str,
        body: impl Fn(Option<&str>) -> io::Result<Vec<u8>>,
    ) -> (Outcome, u64) {
        for _ in 0..ATTEMPTS {
            let (token, machine) = match self.credential() {
                Ok(Some(Credential::Token(token))) => (token, None),
                Ok(Some(Credential::SharedKey(key, machine))) => (key, Some(machine)),
                Ok(None) => return (Outcome::Unauthorized, 0),
                Err(error) => return (Outcome::NoRoute(error), 0),
            };
            let bytes = match body(machine.as_deref()) {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::warn!("cannot build a request: {error}");
                    return (Outcome::BadFormat, 0);
                }
            };
            let outcome = match self.exchange(&Request {
                method: "POST",
                path,
                token: Some(&token),
                headers,
                body: &bytes,
            }) {
                Ok(answer) => outcome_of(&answer),
                Err(error) => Outcome::NoRoute(error),
            };
            self.lost(&outcome);
            if matches!(outcome, Outcome::UnknownToken)
                && machine.is_none()
                && self.prepare().is_ok()
            {
                continue;
            }
            return (outcome, bytes.len() as u64);
        }
        (Outcome::Unauthorized, 0)
    }

    pub fn send(
        &mut self,
        report: &Stored,
        counts_only: bool,
        mut allowed: impl FnMut(u64) -> bool,
    ) -> (Outcome, u64) {
        let mut without = Vec::new();
        let headers = format!(
            "Content-Type: multipart/form-data; boundary={}\r\nContent-Encoding: gzip\r\n",
            boundary(&report.id)
        );
        loop {
            let names = match body_of(report, None, counts_only, &without) {
                Ok(body) if !allowed(body.bytes.len() as u64) => return (Outcome::OverBudget, 0),
                Ok(body) => body.attachments,
                Err(error) => {
                    tracing::warn!("cannot read report {}: {error}", report.id);
                    return (Outcome::BadFormat, 0);
                }
            };
            let dropped = without.clone();
            let (outcome, bytes) = self.authorised(REPORTS, &headers, |machine| {
                body_of(report, machine, counts_only, &dropped).map(|body| body.bytes)
            });
            match outcome {
                Outcome::TooLarge => match names.last() {
                    Some(largest) => without.push(largest.clone()),
                    None => return (Outcome::TooLarge, bytes),
                },
                outcome => return (outcome, bytes),
            }
        }
    }

    pub fn checkin(&mut self, payload: &Value) -> Outcome {
        self.authorised(CHECKINS, JSON, |machine| {
            let mut payload = payload.clone();
            if let (Some(machine), Some(fields)) = (machine, payload.as_object_mut()) {
                fields.insert("machine".to_owned(), json!(machine));
            }
            Ok(serde_json::to_vec(&payload)?)
        })
        .0
    }
}

pub fn forget(request: &Map<String, Value>, ca: Option<&Path>) -> Outcome {
    let text = |name| request.get(name).and_then(Value::as_str);
    let (Some(target), Some(token)) = (text("endpoint").and_then(parse_endpoint), text("token"))
    else {
        return Outcome::Stored;
    };
    let answer = Session::new(target, ca).send(&Request {
        method: "DELETE",
        path: ME,
        token: Some(token),
        headers: "",
        body: b"",
    });
    match answer.map(|answer| outcome_of(&answer)) {
        Ok(
            Outcome::Stored
            | Outcome::UnknownToken
            | Outcome::Revoked
            | Outcome::Unauthorized
            | Outcome::NotSupported
            | Outcome::BadFormat,
        ) => Outcome::Stored,
        Ok(outcome) => outcome,
        Err(error) => Outcome::NoRoute(error),
    }
}

pub fn forget_here(spool: &Spool) -> io::Result<()> {
    if let Some(identity) = Identity::load(spool.root())
        && let Some(credentials) = identity.credentials
    {
        spool.queue_forget(&json!({"endpoint": identity.endpoint, "token": credentials.token}))?;
    }
    identity::forget(spool.root())
}

#[cfg(test)]
mod tests {
    use super::{Outcome, outcome_of};
    use crate::http::read_answer;
    use std::io::BufReader;
    use std::time::Duration;

    fn outcome(text: &str) -> Outcome {
        match read_answer(&mut BufReader::new(text.as_bytes())) {
            Ok((answer, _)) => outcome_of(&answer),
            Err(error) => Outcome::NoRoute(error),
        }
    }

    #[test]
    fn the_server_decides_what_happens_next() {
        assert!(matches!(
            outcome("HTTP/1.1 201 Created\r\ncontent-length: 2\r\n\r\n{}"),
            Outcome::Stored
        ));
        assert!(matches!(
            outcome("HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n"),
            Outcome::BadFormat
        ));
        let body = r#"{"error":"x","code":"unknown_token"}"#;
        assert!(matches!(
            outcome(&format!(
                "HTTP/1.1 401 Unauthorized\r\ntransfer-encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
                body.len()
            )),
            Outcome::UnknownToken
        ));
        assert!(matches!(
            outcome("HTTP/1.1 429 Too Many\r\nretry-after: 120\r\ncontent-length: 0\r\n\r\n"),
            Outcome::Later(Some(wait)) if wait == Duration::from_secs(120)
        ));
        assert!(matches!(
            outcome("HTTP/1.1 507 Full\r\n\r\n"),
            Outcome::Later(None)
        ));
        assert!(matches!(outcome("garbage"), Outcome::NoRoute(_)));
    }
}
