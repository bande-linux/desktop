use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

pub const HELPER: &str = "/usr/lib/polkit-1/polkit-agent-helper-1";

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

#[derive(Clone)]
pub struct Canceller(Arc<Mutex<Child>>);

impl Canceller {
    pub fn cancel(&self) {
        let _ = self.0.lock().unwrap_or_else(PoisonError::into_inner).kill();
    }
}

pub struct HelperSession {
    child: Arc<Mutex<Child>>,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl HelperSession {
    pub fn start(helper: &Path, user: &str, cookie: &str) -> io::Result<Self> {
        let mut child = Command::new(helper)
            .arg(user)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err(io::Error::other("the helper has no pipes"));
        };
        stdin.write_all(format!("{cookie}\n").as_bytes())?;
        Ok(Self {
            child: Arc::new(Mutex::new(child)),
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    pub fn canceller(&self) -> Canceller {
        Canceller(Arc::clone(&self.child))
    }

    pub fn read_line(&mut self) -> io::Result<Option<Line>> {
        let mut line = Vec::new();
        if self.stdout.read_until(b'\n', &mut line)? == 0 {
            return Ok(None);
        }
        parse(&line)
            .map(Some)
            .ok_or_else(|| io::Error::other("the helper sent an unknown line"))
    }

    pub fn respond(&mut self, response: &str) -> io::Result<()> {
        let mut line = response.as_bytes().to_vec();
        if line.last() != Some(&b'\n') {
            line.push(b'\n');
        }
        self.stdin.write_all(&line)
    }
}

impl Drop for HelperSession {
    fn drop(&mut self) {
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(child.try_wait(), Ok(None)) {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
}
