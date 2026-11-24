use std::fs;
use std::io::{self, Write};
use std::time::Duration;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::{Map, Value, json};

use crate::http::{Answer, Request, Session};
use crate::identity::{self, Identity};
use crate::spool::{MANIFEST, STATE, Spool, Status, Stored};

const REPORTS: &str = "/v1/reports";
const CHECKINS: &str = "/v1/checkins";
const ENROLL: &str = "/v1/enroll";
const ROTATE: &str = "/v1/rotate";
const ME: &str = "/v1/me";
const JSON: &[(&str, &str)] = &[("Content-Type", "application/json")];
const ATTEMPTS: usize = 2;

pub enum Outcome {
    Stored,
    BadFormat,
    Unauthorized,
    UnknownToken,
    Revoked,
    TooLarge,
    OverBudget,
    Withdrawn,
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

fn body_of(report: &Stored, counts_only: bool, without: &[String]) -> io::Result<Body> {
    let mut manifest = report.manifest.clone();
    manifest.insert("occurrences".to_owned(), json!(report.occurrences));
    manifest.remove("owner_uid");
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

pub struct Sender<'a> {
    session: Session,
    spool: &'a Spool,
    pub identity: Identity,
    pub contacted: bool,
    kept: bool,
}

impl<'a> Sender<'a> {
    pub fn new(session: Session, spool: &'a Spool) -> Self {
        let identity = Identity::load(spool.root());
        Self {
            session,
            spool,
            kept: identity.is_some(),
            identity: identity.unwrap_or_else(Identity::unenrolled),
            contacted: false,
        }
    }

    fn forgotten(&self) -> bool {
        if self.kept {
            Identity::load(self.spool.root()).is_none()
        } else {
            self.spool.is_withdrawn()
        }
    }

    fn save(&mut self) {
        let saved = if self.forgotten() {
            queue_forget(self.spool, &self.identity)
        } else {
            self.identity
                .save(self.spool.root())
                .inspect(|()| self.kept = true)
        };
        if let Err(error) = saved {
            tracing::warn!("cannot keep this computer's report identity: {error}");
        }
    }

    fn exchange(&mut self, request: &Request) -> Result<Answer, Outcome> {
        if self.spool.is_withdrawn() {
            return Err(Outcome::Withdrawn);
        }
        let answer = self.session.send(request).map_err(Outcome::NoRoute)?;
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

    fn token(&self) -> Option<String> {
        if self.identity.revoked {
            return None;
        }
        self.identity
            .credentials
            .as_ref()
            .map(|credentials| credentials.token.clone())
    }

    fn enroll(&mut self) -> Outcome {
        for _ in 0..ATTEMPTS {
            let challenge = match self.exchange(&Request {
                method: "GET",
                path: ENROLL,
                token: None,
                headers: &[],
                body: b"",
            }) {
                Ok(answer) if answer.status == 200 => answer.json().unwrap_or_default(),
                Ok(answer) => return outcome_of(&answer),
                Err(outcome) => return outcome,
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
                Err(outcome) => return outcome,
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
            headers: &[],
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
            Err(Outcome::NoRoute(error)) => {
                tracing::debug!("cannot rotate the report token: {error}");
            }
            Err(_) => {}
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
        if self.identity.credentials.is_some() {
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
        headers: &[(&str, &str)],
        body: impl Fn() -> io::Result<Vec<u8>>,
    ) -> (Outcome, u64) {
        for _ in 0..ATTEMPTS {
            let Some(token) = self.token() else {
                return (Outcome::Unauthorized, 0);
            };
            let bytes = match body() {
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
                Err(outcome) => outcome,
            };
            self.lost(&outcome);
            if matches!(outcome, Outcome::UnknownToken) && self.prepare().is_ok() {
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
        let content_type = format!("multipart/form-data; boundary={}", boundary(&report.id));
        let headers = [
            ("Content-Type", content_type.as_str()),
            ("Content-Encoding", "gzip"),
        ];
        loop {
            let names = match body_of(report, counts_only, &without) {
                Ok(body) if !allowed(body.bytes.len() as u64) => return (Outcome::OverBudget, 0),
                Ok(body) => body.attachments,
                Err(error) => {
                    tracing::warn!("cannot read report {}: {error}", report.id);
                    return (Outcome::BadFormat, 0);
                }
            };
            let dropped = without.clone();
            let (outcome, bytes) = self.authorised(REPORTS, &headers, || {
                body_of(report, counts_only, &dropped).map(|body| body.bytes)
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
        self.authorised(CHECKINS, JSON, || Ok(serde_json::to_vec(payload)?))
            .0
    }
}

pub fn forget(session: &Session, request: &Map<String, Value>) -> Outcome {
    let Some(token) = request.get("token").and_then(Value::as_str) else {
        return Outcome::Stored;
    };
    let answer = session.send(&Request {
        method: "DELETE",
        path: ME,
        token: Some(token),
        headers: &[],
        body: b"",
    });
    match answer.map(|answer| outcome_of(&answer)) {
        Ok(
            Outcome::Stored
            | Outcome::UnknownToken
            | Outcome::Revoked
            | Outcome::Unauthorized
            | Outcome::BadFormat,
        ) => Outcome::Stored,
        Ok(outcome) => outcome,
        Err(error) => Outcome::NoRoute(error),
    }
}

fn queue_forget(spool: &Spool, identity: &Identity) -> io::Result<()> {
    match &identity.credentials {
        Some(credentials) => spool.queue_forget(&json!({"token": credentials.token})),
        None => Ok(()),
    }
}

pub fn forget_here(spool: &Spool) -> io::Result<()> {
    if let Some(identity) = Identity::load(spool.root()) {
        queue_forget(spool, &identity)?;
    }
    identity::forget(spool.root())
}
