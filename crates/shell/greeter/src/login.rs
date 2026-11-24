use bande_lock::{Prompt, Refusal, clip_message};
use bande_login::protocol::{AuthMessageType, ErrorType, Request, Response};
use zeroize::Zeroizing;

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

    pub fn start(&mut self, env: Vec<String>) -> Request {
        self.phase = Phase::Starting;
        Request::StartSession { env }
    }

    pub fn cancel(&mut self) -> Request {
        self.phase = Phase::Cancelling;
        Request::CancelSession
    }

    pub fn respond(&mut self, response: Response) -> Vec<Step> {
        match (self.phase, response) {
            (Phase::Cancelling, Response::Error { description, .. }) => {
                tracing::info!("bande-login: {description}");
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
