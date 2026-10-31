use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const SESSION: &str = "unbound-session";
pub const SESSION_ENVIRONMENT: [&str; 3] = [
    "XDG_SESSION_TYPE=wayland",
    "XDG_CURRENT_DESKTOP=Unbound",
    "XDG_SESSION_DESKTOP=unbound",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    CreateSession { username: String },
    PostAuthMessageResponse { response: Option<Zeroizing<String>> },
    StartSession { cmd: Vec<String>, env: Vec<String> },
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

pub fn write_message<T: Serialize>(stream: &mut impl Write, message: &T) -> io::Result<()> {
    let mut counter = ByteCounter(0);
    serde_json::to_writer(&mut counter, message)?;
    let mut body = Zeroizing::new(Vec::with_capacity(counter.0));
    serde_json::to_writer(&mut *body, message)?;
    let length = u32::try_from(body.len()).map_err(io::Error::other)?;
    stream.write_all(&length.to_ne_bytes())?;
    stream.write_all(&body)?;
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
    let mut body = vec![0u8; u32::from_ne_bytes(length) as usize];
    stream.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    WrongPassword,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Send(Request),
    Ask(String),
    Show(String),
    Rejected(Rejection),
    Started,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Authenticating,
    Starting,
    Cancelling,
    Idle,
}

#[derive(Debug)]
pub struct Login {
    password: Option<Zeroizing<String>>,
    asked: bool,
    phase: Phase,
}

impl Login {
    pub fn begin(username: &str, password: Zeroizing<String>) -> (Self, Request) {
        (
            Self {
                password: Some(password),
                asked: false,
                phase: Phase::Authenticating,
            },
            Request::CreateSession {
                username: username.to_owned(),
            },
        )
    }

    pub fn is_idle(&self) -> bool {
        self.phase == Phase::Idle
    }

    pub fn answer(&self, text: Zeroizing<String>) -> Request {
        Request::PostAuthMessageResponse {
            response: Some(text),
        }
    }

    pub fn cancel(&mut self) -> Request {
        self.password = None;
        self.phase = Phase::Cancelling;
        Request::CancelSession
    }

    pub fn respond(&mut self, response: Response) -> Vec<Step> {
        match (self.phase, response) {
            (Phase::Cancelling, _) => {
                self.phase = Phase::Idle;
                Vec::new()
            }
            (
                _,
                Response::AuthMessage {
                    auth_message_type: AuthMessageType::Secret | AuthMessageType::Visible,
                    auth_message,
                },
            ) if self.password.is_none() => {
                self.asked = true;
                vec![Step::Ask(auth_message)]
            }
            (
                _,
                Response::AuthMessage {
                    auth_message_type: AuthMessageType::Secret | AuthMessageType::Visible,
                    ..
                },
            ) => vec![Step::Send(Request::PostAuthMessageResponse {
                response: self.password.take(),
            })],
            (
                _,
                Response::AuthMessage {
                    auth_message_type: AuthMessageType::Info | AuthMessageType::Error,
                    auth_message,
                },
            ) => vec![
                Step::Show(auth_message),
                Step::Send(Request::PostAuthMessageResponse { response: None }),
            ],
            (Phase::Authenticating, Response::Success) => {
                self.phase = Phase::Starting;
                vec![Step::Send(Request::StartSession {
                    cmd: vec![SESSION.to_owned()],
                    env: SESSION_ENVIRONMENT.map(str::to_owned).to_vec(),
                })]
            }
            (Phase::Starting, Response::Success) => {
                self.phase = Phase::Idle;
                vec![Step::Started]
            }
            (Phase::Idle, Response::Success) => Vec::new(),
            (
                phase,
                Response::Error {
                    error_type,
                    description,
                },
            ) => {
                let wrong_password = error_type == ErrorType::AuthError
                    && phase == Phase::Authenticating
                    && !self.asked;
                let rejection = if wrong_password {
                    Rejection::WrongPassword
                } else {
                    Rejection::Failed(description)
                };
                vec![Step::Rejected(rejection), Step::Send(self.cancel())]
            }
        }
    }
}
