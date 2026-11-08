use std::io::{self, Read, Write};
use std::time::Duration;

use bande_dict::{Entry, Hit, Line, Match};
use bande_https::Timeouts;

use crate::L10N;

const PORT: u16 = 443;
const ANSWER_LIMIT: u64 = 256 * 1024;
const REDIRECTS: usize = 3;
const SUMMARY: &str = "/api/rest_v1/page/summary/";
const TIMEOUTS: Timeouts = Timeouts {
    connect: Duration::from_secs(5),
    transfer: Duration::from_secs(10),
};
const OTHER_KIND: u8 = 3;

fn encoded(word: &str) -> String {
    word.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            b' ' => "_".to_owned(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn get(host: &str, path: &str) -> io::Result<(u16, String, Vec<u8>)> {
    let mut stream = bande_https::connect(host, PORT, None, TIMEOUTS)?;
    write!(
        stream,
        "GET {path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: BandeLookUp/{} (https://github.com/bande-linux/desktop)\r\nAccept: application/json\r\n\r\n",
        env!("CARGO_PKG_VERSION")
    )?;
    stream.flush()?;
    let mut answer = Vec::new();
    match Read::take(&mut stream, ANSWER_LIMIT).read_to_end(&mut answer) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof && !answer.is_empty() => {}
        Err(error) => return Err(error),
    }
    let split = answer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| io::Error::other("no HTTP header"))?;
    let head = String::from_utf8_lossy(&answer[..split]).into_owned();
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| io::Error::other("no HTTP status"))?;
    let location = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("location")
                .then(|| value.trim().to_owned())
        })
        .unwrap_or_default();
    Ok((status, location, answer[split + 4..].to_vec()))
}

pub fn wikipedia(word: &str, language: &str) -> Option<Hit> {
    if language.is_empty()
        || !language
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    let host = format!("{language}.wikipedia.org");
    let mut path = format!("{SUMMARY}{}", encoded(word.trim()));
    for _ in 0..REDIRECTS {
        let (status, location, body) = get(&host, &path)
            .inspect_err(|error| tracing::warn!("cannot reach Wikipedia: {error}"))
            .ok()?;
        match status {
            200 => return summary(&body, language),
            301 | 302 | 303 | 307 | 308
                if location.starts_with('/') || !location.contains("://") =>
            {
                path = match location.starts_with('/') {
                    true => location,
                    false => format!("{SUMMARY}{location}"),
                };
            }
            _ => return None,
        }
    }
    None
}

fn summary(body: &[u8], language: &str) -> Option<Hit> {
    let page: serde_json::Value = serde_json::from_slice(body).ok()?;
    if page.get("type").and_then(serde_json::Value::as_str) == Some("disambiguation") {
        return None;
    }
    let title = page.get("title")?.as_str()?.to_owned();
    let extract = page.get("extract")?.as_str()?.trim().to_owned();
    if extract.is_empty() {
        return None;
    }
    Some(Hit {
        id: "wikipedia".to_owned(),
        source: L10N.tr("wikipedia"),
        attribution: L10N.tr("wikipedia-attribution"),
        language: language.to_owned(),
        matched: Match::Headword,
        kind: OTHER_KIND,
        entries: vec![Entry {
            word: title,
            pos: String::new(),
            lines: vec![Line {
                kind: 'D',
                text: extract,
            }],
        }],
        derived: None,
    })
}
