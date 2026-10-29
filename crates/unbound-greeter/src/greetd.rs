use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    CreateSession { username: String },
    PostAuthMessageResponse { response: Option<String> },
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
    let body = serde_json::to_vec(message)?;
    let length = u32::try_from(body.len()).map_err(io::Error::other)?;
    stream.write_all(&length.to_ne_bytes())?;
    stream.write_all(&body)?;
    stream.flush()
}

pub fn read_message<T: for<'de> Deserialize<'de>>(stream: &mut impl Read) -> io::Result<T> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let mut body = vec![0u8; u32::from_ne_bytes(length) as usize];
    stream.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Send(Request),
    Ask { prompt: String, secret: bool },
    Show(String),
    Rejected(String),
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
    password: Option<String>,
    session: Vec<String>,
    phase: Phase,
}

impl Login {
    pub fn begin(username: &str, password: String, session: Vec<String>) -> (Self, Request) {
        (
            Self {
                password: Some(password),
                session,
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

    pub fn answer(&mut self, text: String) -> Request {
        Request::PostAuthMessageResponse {
            response: Some(text),
        }
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
                    auth_message_type: kind @ (AuthMessageType::Secret | AuthMessageType::Visible),
                    auth_message,
                },
            ) if self.password.is_none() => vec![Step::Ask {
                prompt: auth_message,
                secret: kind == AuthMessageType::Secret,
            }],
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
                    cmd: self.session.clone(),
                    env: Vec::new(),
                })]
            }
            (Phase::Starting, Response::Success) => {
                self.phase = Phase::Idle;
                vec![Step::Started]
            }
            (Phase::Idle, Response::Success) => Vec::new(),
            (_, Response::Error { description, .. }) => {
                self.password = None;
                self.phase = Phase::Cancelling;
                vec![
                    Step::Rejected(description),
                    Step::Send(Request::CancelSession),
                ]
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Started,
    Rejected(String),
}

pub fn log_in(
    greetd: &mut (impl Read + Write),
    username: &str,
    password: String,
    session: Vec<String>,
) -> io::Result<Outcome> {
    let (mut login, first) = Login::begin(username, password, session);
    write_message(greetd, &first)?;
    let mut rejection = None;
    loop {
        let response = read_message(greetd)?;
        for step in login.respond(response) {
            match step {
                Step::Send(request) => write_message(greetd, &request)?,
                Step::Started => return Ok(Outcome::Started),
                Step::Rejected(reason) => rejection = Some(reason),
                Step::Ask { prompt, .. } => {
                    rejection = Some(prompt);
                    login.phase = Phase::Cancelling;
                    write_message(greetd, &Request::CancelSession)?;
                }
                Step::Show(_) => {}
            }
        }
        if login.is_idle()
            && let Some(reason) = rejection.take()
        {
            return Ok(Outcome::Rejected(reason));
        }
    }
}
