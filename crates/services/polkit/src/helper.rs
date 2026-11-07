use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;

use zeroize::Zeroizing;

pub const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Prompt { text: String, echo: bool },
    Error(String),
    Info(String),
    Success,
    Failure,
}

pub fn unescape(line: &[u8]) -> Vec<u8> {
    let mut unescaped = Vec::with_capacity(line.len());
    let mut bytes = line.iter().copied().peekable();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            unescaped.push(byte);
            continue;
        }
        let Some(escaped) = bytes.next() else {
            break;
        };
        unescaped.push(match escaped {
            b'0'..=b'7' => {
                let mut value = u32::from(escaped - b'0');
                for _ in 0..2 {
                    match bytes.peek() {
                        Some(digit @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(digit - b'0');
                            bytes.next();
                        }
                        _ => break,
                    }
                }
                u8::try_from(value & 0xff).unwrap_or_default()
            }
            b'b' => 0x08,
            b'f' => 0x0c,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => 0x0b,
            other => other,
        });
    }
    unescaped
}

pub fn parse(line: &[u8]) -> Option<Line> {
    let line =
        String::from_utf8_lossy(&unescape(line.strip_suffix(b"\n").unwrap_or(line))).into_owned();
    let field = |prefix: &str| line.strip_prefix(prefix).map(str::to_owned);
    if let Some(text) = field("PAM_PROMPT_ECHO_OFF ") {
        Some(Line::Prompt { text, echo: false })
    } else if let Some(text) = field("PAM_PROMPT_ECHO_ON ") {
        Some(Line::Prompt { text, echo: true })
    } else if let Some(text) = field("PAM_ERROR_MSG ") {
        Some(Line::Error(text))
    } else if let Some(text) = field("PAM_TEXT_INFO ") {
        Some(Line::Info(text))
    } else if line.starts_with("SUCCESS") {
        Some(Line::Success)
    } else if line.starts_with("FAILURE") {
        Some(Line::Failure)
    } else {
        None
    }
}

pub struct Canceller(UnixStream);

impl Canceller {
    pub fn cancel(&self) {
        if let Err(error) = self.0.shutdown(Shutdown::Both) {
            tracing::debug!("the helper had already gone: {error}");
        }
    }
}

pub struct HelperSession {
    socket: UnixStream,
    replies: BufReader<UnixStream>,
}

impl HelperSession {
    pub fn start(helper_socket: &Path, user: &str, cookie: &str) -> io::Result<Self> {
        let mut socket = UnixStream::connect(helper_socket)?;
        socket.write_all(format!("{user}\n{cookie}\n").as_bytes())?;
        let replies = BufReader::new(socket.try_clone()?);
        Ok(Self { socket, replies })
    }

    pub fn canceller(&self) -> io::Result<Canceller> {
        self.socket.try_clone().map(Canceller)
    }

    pub fn read_line(&mut self) -> io::Result<Option<Line>> {
        let mut line = Vec::new();
        if self.replies.read_until(b'\n', &mut line)? == 0 {
            return Ok(None);
        }
        parse(&line)
            .map(Some)
            .ok_or_else(|| io::Error::other("the helper sent an unknown line"))
    }

    pub fn respond(&mut self, response: &str) -> io::Result<()> {
        let mut line = Zeroizing::new(Vec::with_capacity(response.len() + 1));
        line.extend_from_slice(response.as_bytes());
        if line.last() != Some(&b'\n') {
            line.push(b'\n');
        }
        self.socket.write_all(&line)
    }
}

impl Drop for HelperSession {
    fn drop(&mut self) {
        if let Err(error) = self.socket.shutdown(Shutdown::Both) {
            tracing::debug!("the helper had already gone: {error}");
        }
    }
}
