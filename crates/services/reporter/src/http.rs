use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::time::Duration;

use bande_https::{Stream, Timeouts};
use jiff::Timestamp;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(120);
const HEAD_LIMIT: u64 = 16 * 1024;
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
    pub headers: &'a str,
    pub body: &'a [u8],
}

pub struct Session {
    target: Target,
    ca: Option<std::path::PathBuf>,
    stream: Option<BufReader<Stream>>,
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

fn is_untrusted(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
        || error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
            .is_some_and(|inner| matches!(inner, rustls::Error::InvalidCertificate(_)))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

fn read_line(reader: &mut impl BufRead) -> io::Result<String> {
    let mut line = String::new();
    reader.take(HEAD_LIMIT).read_line(&mut line)?;
    if !line.ends_with('\n') {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the answer ended early",
        ));
    }
    Ok(line.trim_end().to_owned())
}

fn read_chunked(reader: &mut impl BufRead) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line = read_line(reader)?;
        let size = usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| invalid("a bad chunk size"))?;
        if size == 0 {
            while !read_line(reader)?.is_empty() {}
            return Ok(body);
        }
        if body.len() + size > BODY_LIMIT as usize {
            return Err(invalid("the answer is too long"));
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..])?;
        read_line(reader)?;
    }
}

pub fn read_answer(reader: &mut impl BufRead) -> io::Result<(Answer, bool)> {
    let status_line = read_line(reader)?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| invalid("not an HTTP answer"))?;
    let mut length = None;
    let mut chunked = false;
    let mut closes = false;
    let mut wait = None;
    loop {
        let line = read_line(reader)?;
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse::<u64>().ok(),
            "transfer-encoding" => chunked = value.eq_ignore_ascii_case("chunked"),
            "connection" => closes = value.eq_ignore_ascii_case("close"),
            "retry-after" => wait = retry_after(value),
            _ => {}
        }
    }
    let body = if chunked {
        read_chunked(reader)?
    } else if let Some(length) = length {
        if length > BODY_LIMIT {
            return Err(invalid("the answer is too long"));
        }
        let mut body = vec![0; length as usize];
        reader.read_exact(&mut body)?;
        body
    } else {
        closes = true;
        let mut body = Vec::new();
        match reader.take(BODY_LIMIT).read_to_end(&mut body) {
            Err(error) if error.kind() != io::ErrorKind::UnexpectedEof => return Err(error),
            _ => {}
        }
        body
    };
    Ok((
        Answer {
            status,
            retry_after: wait,
            body,
        },
        closes,
    ))
}

impl Session {
    pub fn new(target: Target, ca: Option<&Path>) -> Self {
        Self {
            target,
            ca: ca.map(Path::to_path_buf),
            stream: None,
        }
    }

    fn open(&self, ca: Option<&Path>) -> io::Result<BufReader<Stream>> {
        let timeouts = Timeouts {
            connect: CONNECT_TIMEOUT,
            transfer: TRANSFER_TIMEOUT,
        };
        let mut stream = bande_https::connect(&self.target.host, self.target.port, ca, timeouts)?;
        stream.conn.complete_io(&mut stream.sock)?;
        Ok(BufReader::new(stream))
    }

    fn connect(&self) -> io::Result<BufReader<Stream>> {
        match self.open(self.ca.as_deref()) {
            Err(error) if self.ca.is_some() && is_untrusted(&error) => {
                tracing::info!(
                    "the report server's certificate is not from the pinned roots, so the system's roots decide: {error}"
                );
                self.open(None)
            }
            opened => opened,
        }
    }

    fn exchange(
        &self,
        stream: &mut BufReader<Stream>,
        request: &Request,
    ) -> io::Result<(Answer, bool)> {
        let writer = stream.get_mut();
        let authorization = request
            .token
            .map(|token| format!("Authorization: Bearer {token}\r\n"))
            .unwrap_or_default();
        write!(
            writer,
            "{} {} HTTP/1.1\r\nHost: {}:{}\r\n{authorization}{}Content-Length: {}\r\n\r\n",
            request.method,
            request.path,
            self.target.host,
            self.target.port,
            request.headers,
            request.body.len()
        )?;
        writer.write_all(request.body)?;
        writer.flush()?;
        read_answer(stream)
    }

    pub fn send(&mut self, request: &Request) -> io::Result<Answer> {
        let reused = self.stream.is_some();
        let mut stream = match self.stream.take() {
            Some(stream) => stream,
            None => self.connect()?,
        };
        let exchanged = match self.exchange(&mut stream, request) {
            Err(_) if reused => {
                stream = self.connect()?;
                self.exchange(&mut stream, request)
            }
            other => other,
        };
        let (answer, closes) = exchanged?;
        if !closes {
            self.stream = Some(stream);
        }
        Ok(answer)
    }
}
