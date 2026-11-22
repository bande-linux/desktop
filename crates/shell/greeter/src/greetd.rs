use std::io::{self, Read, Write};

use bande_lock::{Prompt, Refusal, clip_message};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const SESSION: &str = "bande-session";
const MESSAGE_LIMIT: u32 = 64 << 10;
pub const SESSION_ENVIRONMENT: [&str; 3] = [
    "XDG_SESSION_TYPE=wayland",
    "XDG_CURRENT_DESKTOP=X-Bande",
    "XDG_SESSION_DESKTOP=bande",
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
    let length = u32::from_ne_bytes(length);
    if length > MESSAGE_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("greetd sent a message of {length} bytes"),
        ));
    }
    let mut body = Zeroizing::new(vec![0u8; length as usize]);
    stream.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Send(Request),
    Ask(Prompt),
    Show(String),
    Refused(Refusal),
    Authenticated,
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
    phase: Phase,
}

fn refusal_of(error_type: ErrorType, description: &str) -> Refusal {
    let code = description.rsplit(": ").next().unwrap_or_default();
    match (Refusal::from_pam_name(code), error_type) {
        (Some(refusal), _) => refusal,
        (None, ErrorType::AuthError) => Refusal::WrongAnswer,
        (None, ErrorType::Error) => Refusal::Unavailable,
    }
}

impl Login {
    pub fn begin(username: &str) -> (Self, Request) {
        (
            Self {
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

    pub fn is_starting(&self) -> bool {
        self.phase == Phase::Starting
    }

    pub fn is_cancelling(&self) -> bool {
        self.phase == Phase::Cancelling
    }

    pub fn answer(&self, text: Zeroizing<String>) -> Request {
        Request::PostAuthMessageResponse {
            response: Some(text),
        }
    }

    pub fn start(&mut self, environment: &[String]) -> Request {
        self.phase = Phase::Starting;
        let mut env = SESSION_ENVIRONMENT.map(str::to_owned).to_vec();
        env.extend_from_slice(environment);
        Request::StartSession {
            cmd: vec![SESSION.to_owned()],
            env,
        }
    }

    pub fn cancel(&mut self) -> Request {
        self.phase = Phase::Cancelling;
        Request::CancelSession
    }

    pub fn respond(&mut self, response: Response) -> Vec<Step> {
        match (self.phase, response) {
            (Phase::Cancelling, Response::Error { description, .. }) => {
                tracing::info!("greetd: {description}");
                self.phase = Phase::Idle;
                Vec::new()
            }
            (Phase::Cancelling, Response::Success) => {
                self.phase = Phase::Idle;
                Vec::new()
            }
            (Phase::Cancelling, Response::AuthMessage { .. }) => vec![Step::Send(self.cancel())],
            (
                _,
                Response::AuthMessage {
                    auth_message_type: kind @ (AuthMessageType::Secret | AuthMessageType::Visible),
                    auth_message,
                },
            ) => vec![Step::Ask(Prompt {
                text: clip_message(&auth_message),
                visible: kind == AuthMessageType::Visible,
            })],
            (
                _,
                Response::AuthMessage {
                    auth_message_type: AuthMessageType::Info | AuthMessageType::Error,
                    auth_message,
                },
            ) => vec![
                Step::Show(clip_message(&auth_message)),
                Step::Send(Request::PostAuthMessageResponse { response: None }),
            ],
            (Phase::Authenticating, Response::Success) => vec![Step::Authenticated],
            (Phase::Starting, Response::Success) => {
                self.phase = Phase::Idle;
                vec![Step::Started]
            }
            (Phase::Idle, Response::Success) => Vec::new(),
            (
                _,
                Response::Error {
                    error_type,
                    description,
                },
            ) => {
                tracing::info!("not signed in: {description}");
                vec![
                    Step::Refused(refusal_of(error_type, &description)),
                    Step::Send(self.cancel()),
                ]
            }
        }
    }
}
