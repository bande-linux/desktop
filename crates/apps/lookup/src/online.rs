use std::io::{self, BufRead, Read, Write};
use std::time::Duration;

use bande_dict::{Entry, Hit, Line, Match, Pack};
use bande_https::Timeouts;

use crate::import;

const PORT: u16 = 443;
const ANSWER_LIMIT: u64 = 256 * 1024;
const REDIRECTS: usize = 3;
const TIMEOUTS: Timeouts = Timeouts {
    connect: Duration::from_secs(5),
    transfer: Duration::from_secs(10),
};
const WIKIMEDIA_SUMMARY: &str = "wikimedia-summary";
const DICT: &str = "dict";

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

pub fn look_up(source: &Pack, word: &str, language: &str) -> Option<Hit> {
    let online = source.online.as_ref()?;
    match online.protocol.as_str() {
        WIKIMEDIA_SUMMARY => wikimedia_summary(source, &online.url, word, language),
        DICT => dict(source, &online.url, word)
            .inspect_err(|error| tracing::warn!("cannot ask {}: {error}", source.name))
            .ok()
            .flatten(),
        other => {
            tracing::warn!("{}: unknown protocol {other}", source.id);
            None
        }
    }
}

fn wikimedia_summary(source: &Pack, url: &str, word: &str, language: &str) -> Option<Hit> {
    if language.is_empty()
        || !language
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    let address = url
        .replace("{language}", language)
        .replace("{word}", &encoded(word.trim()));
    let rest = address.strip_prefix("https://")?;
    let (host, first_path) = rest.split_at(rest.find('/')?);
    let base = &first_path[..=first_path.rfind('/')?];
    let mut path = first_path.to_owned();
    for _ in 0..REDIRECTS {
        let (status, location, body) = get(host, &path)
            .inspect_err(|error| tracing::warn!("cannot reach {}: {error}", source.name))
            .ok()?;
        match status {
            200 => return summary(source, &body, language),
            301 | 302 | 303 | 307 | 308
                if location.starts_with('/') || !location.contains("://") =>
            {
                path = match location.starts_with('/') {
                    true => location,
                    false => format!("{base}{location}"),
                };
            }
            _ => return None,
        }
    }
    None
}

fn summary(source: &Pack, body: &[u8], language: &str) -> Option<Hit> {
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
        place: 0,
        id: source.id.clone(),
        source: source.name.clone(),
        attribution: source.attribution.clone(),
        language: language.to_owned(),
        matched: Match::Headword,
        kind: source.kind_rank(language),
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

fn dict(source: &Pack, url: &str, word: &str) -> io::Result<Option<Hit>> {
    let word = word.trim();
    if word.is_empty()
        || word
            .chars()
            .any(|c| c == '"' || c == '\\' || c.is_control())
    {
        return Ok(None);
    }
    let (host, port, database) =
        import::parse_dict_url(url).ok_or_else(|| io::Error::other("a bad address"))?;
    let mut stream = import::connect_dict(&host, port)?;
    stream
        .get_mut()
        .write_all(format!("DEFINE {database} \"{word}\"\r\nQUIT\r\n").as_bytes())?;
    let mut entries = Vec::new();
    let mut current: Option<Entry> = None;
    let mut line = String::new();
    loop {
        line.clear();
        if stream.read_line(&mut line)? == 0 {
            break;
        }
        let text = line.trim_end_matches(['\r', '\n']);
        if text == "." && current.is_some() {
            entries.extend(current.take());
            continue;
        }
        match &mut current {
            Some(entry) => {
                let text = text
                    .strip_prefix('.')
                    .filter(|_| text.starts_with(".."))
                    .unwrap_or(text);
                let text = text.trim();
                if !text.is_empty() && !(entry.lines.is_empty() && text.eq_ignore_ascii_case(word))
                {
                    entry.lines.push(Line {
                        kind: 'D',
                        text: text.to_owned(),
                    });
                }
            }
            None if text.starts_with("151 ") => {
                current = Some(Entry {
                    word: word.to_owned(),
                    pos: String::new(),
                    lines: Vec::new(),
                });
            }
            None if text.starts_with("250")
                || text.starts_with("552")
                || text.starts_with("221") =>
            {
                if !text.starts_with("250") {
                    break;
                }
            }
            None if text.starts_with('5') || text.starts_with('4') => {
                return Err(io::Error::other(text.to_owned()));
            }
            None => {}
        }
    }
    Ok((!entries.is_empty()).then(|| Hit {
        place: 0,
        id: source.id.clone(),
        source: source.name.clone(),
        attribution: source.attribution.clone(),
        language: String::new(),
        matched: Match::Headword,
        kind: source.kind_rank(""),
        entries,
        derived: None,
    }))
}
