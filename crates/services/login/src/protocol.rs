use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// The largest message either side of a connection accepts.
pub const MESSAGE_LIMIT: u32 = 64 << 10;
/// The variable that tells the login window where to reach `bande-login`.
pub const SOCKET_VARIABLE: &str = "BANDE_LOGIN_SOCK";
/// The prefix of the variables the login window may hand to a session.
pub const CARRIED_PREFIX: &str = "BANDE_LOGIN_";
const CARRIED_LIMIT: usize = 4096;

/// What the login window asks of `bande-login`; each request gets one [`Response`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    CreateSession { username: String },
    PostAuthMessageResponse { response: Option<Zeroizing<String>> },
    StartSession { env: Vec<String> },
    CancelSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    AuthError,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMessageType {
    Visible,
    Secret,
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Success,
    Error {
        error_type: ErrorType,
        description: String,
    },
    AuthMessage {
        auth_message_type: AuthMessageType,
        auth_message: String,
    },
}

impl Response {
    pub fn error(error_type: ErrorType, description: impl Into<String>) -> Self {
        Self::Error {
            error_type,
            description: description.into(),
        }
    }
}

/// Whose session a worker opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Greeter,
    User,
}

/// What `bande-login` tells one of its session workers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToWorker {
    Begin { class: Class, username: String },
    Answer { response: Option<Zeroizing<String>> },
    Launch { env: Vec<String> },
}

/// What a session worker tells `bande-login`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromWorker {
    Ask {
        text: String,
        visible: bool,
    },
    Show {
        text: String,
        error: bool,
    },
    Authenticated,
    Failed {
        error_type: ErrorType,
        description: String,
    },
    Shown,
}

/// Keeps the variables the login window may pass on: `BANDE_LOGIN_…` assignments of a sane size.
pub fn carried_environment(env: Vec<String>) -> Vec<String> {
    env.into_iter()
        .filter(|assignment| {
            let carried = assignment.len() <= CARRIED_LIMIT
                && !assignment.contains('\0')
                && assignment
                    .split_once('=')
                    .and_then(|(name, _)| name.strip_prefix(CARRIED_PREFIX))
                    .is_some_and(|rest| {
                        !rest.is_empty()
                            && rest
                                .bytes()
                                .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
                    });
            if !carried {
                tracing::warn!("the login window passed a variable it may not set");
            }
            carried
        })
        .collect()
}

pub fn write_message<T: Serialize>(stream: &mut impl Write, message: &T) -> io::Result<()> {
    let mut counter = ByteCounter(0);
    serde_json::to_writer(&mut counter, message)?;
    let mut body = Zeroizing::new(Vec::with_capacity(counter.0));
    serde_json::to_writer(&mut *body, message)?;
    let length = u32::try_from(body.len())
        .ok()
        .filter(|length| *length <= MESSAGE_LIMIT)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the message is too long"))?;
    let mut framed = Zeroizing::new(Vec::with_capacity(body.len() + 4));
    framed.extend_from_slice(&length.to_ne_bytes());
    framed.extend_from_slice(&body);
    stream.write_all(&framed)?;
    stream.flush()
}

struct ByteCounter(usize);

impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn read_message<T: for<'de> Deserialize<'de>>(stream: &mut impl Read) -> io::Result<T> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_ne_bytes(length);
    if length > MESSAGE_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a message of {length} bytes is over the limit"),
        ));
    }
    let mut body = Zeroizing::new(vec![0u8; length as usize]);
    stream.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

/// A non-blocking stream that gives up once `until` has passed, however slowly the other side
/// sends or takes each byte, so one message cannot hold its reader for longer than that.
pub struct Deadline<'a> {
    stream: &'a UnixStream,
    until: Instant,
}

impl<'a> Deadline<'a> {
    pub fn within(stream: &'a UnixStream, patience: Duration) -> Self {
        Self {
            stream,
            until: Instant::now() + patience,
        }
    }

    fn retry<T>(
        &self,
        flags: PollFlags,
        mut attempt: impl FnMut() -> io::Result<T>,
    ) -> io::Result<T> {
        loop {
            match attempt() {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                done => return done,
            }
            let left = self.until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the message did not arrive in time",
                ));
            }
            let timeout = Timespec::try_from(left).map_err(io::Error::other)?;
            let mut fds = [PollFd::new(self.stream, flags)];
            match poll(&mut fds, Some(&timeout)) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl Read for Deadline<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let stream = self.stream;
        self.retry(PollFlags::IN, || (&*stream).read(buffer))
    }
}

impl Write for Deadline<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let stream = self.stream;
        self.retry(PollFlags::OUT, || (&*stream).write(bytes))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
