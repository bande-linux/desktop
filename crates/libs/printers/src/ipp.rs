use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

pub(crate) const OPERATION: u8 = 0x01;
pub(crate) const JOB: u8 = 0x02;
pub(crate) const PRINTER: u8 = 0x04;
const END: u8 = 0x03;
const DELIMITERS_END: u8 = 0x0F;
pub(crate) const INTEGER: u8 = 0x21;
const BOOLEAN: u8 = 0x22;
const ENUM: u8 = 0x23;
const TEXT_WITH_LANGUAGE: u8 = 0x35;
const NAME_WITH_LANGUAGE: u8 = 0x36;
const NAME: u8 = 0x42;
pub(crate) const KEYWORD: u8 = 0x44;
pub(crate) const URI: u8 = 0x45;
const CHARSET: u8 = 0x47;
const LANGUAGE: u8 = 0x48;
const SUCCESS_LIMIT: u16 = 0x00FF;
const HTTP_UNAUTHORIZED: u16 = 401;
const HTTP_FORBIDDEN: u16 = 403;
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Integer(i32),
    Boolean(bool),
    Text(String),
    Other,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    NotAllowed,
    Refused(u16, String),
    Malformed,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "cannot reach CUPS: {error}"),
            Self::NotAllowed => f.write_str("this user may not change printers"),
            Self::Refused(_, message) if !message.is_empty() => f.write_str(message),
            Self::Refused(status, _) => write!(f, "CUPS refused the request ({status:#06x})"),
            Self::Malformed => f.write_str("CUPS sent an answer that cannot be read"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub(crate) struct Attribute {
    pub name: String,
    pub values: Vec<Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct Group {
    pub tag: u8,
    pub attributes: Vec<Attribute>,
}

impl Group {
    pub(crate) fn get(&self, name: &str) -> Option<&[Value]> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.values.as_slice())
    }

    pub(crate) fn text(&self, name: &str) -> Option<String> {
        match self.get(name)?.first()? {
            Value::Text(text) => Some(text.clone()),
            _ => None,
        }
    }

    pub(crate) fn texts(&self, name: &str) -> Vec<String> {
        self.get(name)
            .unwrap_or_default()
            .iter()
            .filter_map(|value| match value {
                Value::Text(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn integer(&self, name: &str) -> Option<i32> {
        match self.get(name)?.first()? {
            Value::Integer(number) => Some(*number),
            _ => None,
        }
    }

    pub(crate) fn boolean(&self, name: &str) -> Option<bool> {
        match self.get(name)?.first()? {
            Value::Boolean(flag) => Some(*flag),
            _ => None,
        }
    }
}

pub(crate) struct Request {
    body: Vec<u8>,
}

impl Request {
    pub(crate) fn new(operation: u16, user: &str) -> Self {
        let mut request = Self {
            body: [0x02, 0x00].to_vec(),
        };
        request.body.extend(operation.to_be_bytes());
        request.body.extend(1u32.to_be_bytes());
        request.body.push(OPERATION);
        request.string(CHARSET, "attributes-charset", "utf-8");
        request.string(LANGUAGE, "attributes-natural-language", "en");
        request.string(NAME, "requesting-user-name", user);
        request
    }

    fn value(&mut self, tag: u8, name: &str, value: &[u8]) {
        self.body.push(tag);
        self.body
            .extend(u16::try_from(name.len()).unwrap_or(0).to_be_bytes());
        self.body.extend(name.as_bytes());
        self.body
            .extend(u16::try_from(value.len()).unwrap_or(0).to_be_bytes());
        self.body.extend(value);
    }

    pub(crate) fn string(&mut self, tag: u8, name: &str, value: &str) -> &mut Self {
        self.value(tag, name, value.as_bytes());
        self
    }

    pub(crate) fn strings(&mut self, tag: u8, name: &str, values: &[&str]) -> &mut Self {
        for (index, value) in values.iter().enumerate() {
            self.value(tag, if index == 0 { name } else { "" }, value.as_bytes());
        }
        self
    }

    pub(crate) fn integer(&mut self, tag: u8, name: &str, value: i32) -> &mut Self {
        self.value(tag, name, &value.to_be_bytes());
        self
    }

    pub(crate) fn send(&mut self, socket: &Path, resource: &str, user: &str) -> Result<Vec<Group>> {
        self.body.push(END);
        let mut stream = UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        let head = format!(
            "POST {resource} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/ipp\r\n\
             Content-Length: {}\r\nAuthorization: PeerCred {user}\r\nConnection: close\r\n\r\n",
            self.body.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(&self.body)?;
        let body = read_http(BufReader::new(stream))?;
        parse(&body)
    }
}

fn read_http(mut reader: impl BufRead) -> Result<Vec<u8>> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or(Error::Malformed)?;
    let mut length = None;
    let mut chunked = false;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(Error::Malformed);
        }
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            length = value.parse::<usize>().ok();
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.eq_ignore_ascii_case("chunked");
        }
    }
    match status {
        HTTP_UNAUTHORIZED | HTTP_FORBIDDEN => return Err(Error::NotAllowed),
        200 => {}
        other => return Err(Error::Refused(other, format!("HTTP {other}"))),
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            line.clear();
            reader.read_line(&mut line)?;
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16)
                .map_err(|_| Error::Malformed)?;
            if size == 0 {
                break;
            }
            let start = body.len();
            body.resize(start + size, 0);
            reader.read_exact(&mut body[start..])?;
            line.clear();
            reader.read_line(&mut line)?;
        }
    } else if let Some(length) = length {
        body.resize(length, 0);
        reader.read_exact(&mut body)?;
    } else {
        reader.read_to_end(&mut body)?;
    }
    Ok(body)
}

struct Cursor<'a> {
    bytes: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        if self.bytes.len() < count {
            return Err(Error::Malformed);
        }
        let (taken, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(taken)
    }

    fn short(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn sized(&mut self) -> Result<&'a [u8]> {
        let length = usize::from(self.short()?);
        self.take(length)
    }
}

fn value(tag: u8, bytes: &[u8]) -> Value {
    let text = |bytes: &[u8]| Value::Text(String::from_utf8_lossy(bytes).into_owned());
    match tag {
        INTEGER | ENUM => bytes
            .try_into()
            .map_or(Value::Other, |raw| Value::Integer(i32::from_be_bytes(raw))),
        BOOLEAN => Value::Boolean(bytes.first().is_some_and(|byte| *byte != 0)),
        TEXT_WITH_LANGUAGE | NAME_WITH_LANGUAGE => {
            let mut cursor = Cursor { bytes };
            match cursor.sized().and_then(|_| cursor.sized()) {
                Ok(inner) => text(inner),
                Err(_) => Value::Other,
            }
        }
        0x40..=0x5F => text(bytes),
        _ => Value::Other,
    }
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Vec<Group>> {
    let mut cursor = Cursor { bytes };
    cursor.take(2)?;
    let status = cursor.short()?;
    cursor.take(4)?;
    let mut groups: Vec<Group> = Vec::new();
    loop {
        let tag = *cursor.take(1)?.first().ok_or(Error::Malformed)?;
        if tag == END {
            break;
        }
        if tag <= DELIMITERS_END {
            groups.push(Group {
                tag,
                attributes: Vec::new(),
            });
            continue;
        }
        let name = String::from_utf8_lossy(cursor.sized()?).into_owned();
        let value = value(tag, cursor.sized()?);
        let group = groups.last_mut().ok_or(Error::Malformed)?;
        match group.attributes.last_mut() {
            Some(attribute) if name.is_empty() => attribute.values.push(value),
            _ => group.attributes.push(Attribute {
                name,
                values: vec![value],
            }),
        }
    }
    if status > SUCCESS_LIMIT {
        let message = groups
            .iter()
            .find(|group| group.tag == OPERATION)
            .and_then(|group| group.text("status-message"))
            .unwrap_or_default();
        return Err(Error::Refused(status, message));
    }
    Ok(groups)
}
