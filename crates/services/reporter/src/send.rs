use std::fs;
use std::io::{self, Read, Write};
use std::time::Duration;

use bande_https::Timeouts;
use flate2::Compression;
use flate2::write::GzEncoder;
use jiff::Timestamp;
use serde_json::{Value, json};

use crate::config::Config;
use crate::spool::{MANIFEST, Stored};

const PATH: &str = "/v1/reports";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(120);
const ANSWER_LIMIT: u64 = 64 * 1024;
const LONGEST_RETRY_AFTER: u64 = 24 * 60 * 60;

pub enum Outcome {
    Stored,
    BadFormat,
    Unauthorized,
    TooLarge,
    Later(Option<Duration>),
    NoRoute(io::Error),
}

struct Target {
    host: String,
    port: u16,
}

fn parse_endpoint(endpoint: &str) -> Option<Target> {
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

fn boundary(id: &str) -> String {
    format!("bande-report-{id}")
}

fn body_of(report: &Stored, without: &[String]) -> io::Result<(Vec<u8>, Vec<String>)> {
    let mut manifest = report.manifest.clone();
    manifest.insert("occurrences".to_owned(), json!(report.occurrences));
    let mut attachments: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in fs::read_dir(&report.path)?.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name != MANIFEST
            && !name.starts_with('.')
            && name != "state.json"
            && !without.contains(&name)
        {
            attachments.push((name.clone(), fs::read(entry.path())?));
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
    Ok((gzip.finish()?, names))
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

fn outcome_of(answer: &str) -> io::Result<Outcome> {
    let mut lines = answer.lines();
    let status: u16 = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an HTTP answer"))?;
    let wait = lines
        .take_while(|line| !line.is_empty())
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("retry-after").then_some(value)
        })
        .and_then(retry_after);
    Ok(match status {
        200 | 201 => Outcome::Stored,
        400 => Outcome::BadFormat,
        401 | 403 => Outcome::Unauthorized,
        413 => Outcome::TooLarge,
        _ => Outcome::Later(wait),
    })
}

fn post(
    config: &Config,
    target: &Target,
    key: &str,
    boundary: &str,
    body: &[u8],
) -> io::Result<String> {
    let timeouts = Timeouts {
        connect: CONNECT_TIMEOUT,
        transfer: TRANSFER_TIMEOUT,
    };
    let mut stream =
        bande_https::connect(&target.host, target.port, config.ca.as_deref(), timeouts)?;
    write!(
        stream,
        "POST {PATH} HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: Bearer {key}\r\nContent-Type: multipart/form-data; boundary={boundary}\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        target.host,
        target.port,
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()?;
    let mut answer = Vec::new();
    let read = Read::take(&mut stream, ANSWER_LIMIT).read_to_end(&mut answer);
    match read {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof && !answer.is_empty() => {}
        Err(error) => return Err(error),
    }
    Ok(String::from_utf8_lossy(&answer).into_owned())
}

pub fn send(config: &Config, report: &Stored) -> Outcome {
    let Some((endpoint, key)) = config.server() else {
        return Outcome::Unauthorized;
    };
    let Some(target) = parse_endpoint(endpoint) else {
        tracing::warn!("the report server's address is not https://host:port");
        return Outcome::Unauthorized;
    };
    let mut without = Vec::new();
    loop {
        let (body, names) = match body_of(report, &without) {
            Ok(built) => built,
            Err(error) => {
                tracing::warn!("cannot read report {}: {error}", report.id);
                return Outcome::BadFormat;
            }
        };
        let outcome = post(config, &target, key, &boundary(&report.id), &body)
            .and_then(|answer| outcome_of(&answer));
        match outcome {
            Ok(Outcome::TooLarge) => match names.last() {
                Some(largest) => without.push(largest.clone()),
                None => return Outcome::TooLarge,
            },
            Ok(outcome) => return outcome,
            Err(error) => return Outcome::NoRoute(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Outcome, outcome_of};
    use std::time::Duration;

    #[test]
    fn the_server_decides_what_happens_next() {
        let wait = |text| match outcome_of(text) {
            Ok(Outcome::Later(wait)) => wait,
            _ => panic!("expected a retry: {text}"),
        };
        assert!(matches!(
            outcome_of("HTTP/1.1 201 Created\r\n\r\n"),
            Ok(Outcome::Stored)
        ));
        assert!(matches!(
            outcome_of("HTTP/1.1 400 Bad Request\r\n\r\n"),
            Ok(Outcome::BadFormat)
        ));
        assert_eq!(
            wait("HTTP/1.1 429 Too Many\r\nretry-after: 120\r\n\r\n"),
            Some(Duration::from_secs(120))
        );
        assert_eq!(wait("HTTP/1.1 507 Full\r\n\r\n"), None);
        assert!(outcome_of("garbage").is_err());
    }
}
