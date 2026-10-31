use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use zbus::blocking::Connection;
use zbus::fdo::DBusProxy;
use zbus::message::Header;
use zbus::names::{BusName, UniqueName};
use zbus::zvariant::{OwnedValue, Value};

use crate::helper::{Canceller, HelperSession, Line};
use zeroize::Zeroizing;

pub const OBJECT_PATH: &str = "/org/unbound/PolicyKit1/AuthenticationAgent";
const AUTHORITY: &str = "org.freedesktop.PolicyKit1";
const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const AUTHORITY_INTERFACE: &str = "org.freedesktop.PolicyKit1.Authority";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub action_id: String,
    pub message: String,
    pub icon_name: String,
    pub details: HashMap<String, String>,
    pub user: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Info(String),
    Error(String),
    WrongPassword,
}

pub trait Conversation: Send + Sync {
    fn respond(&self, request: &Request, prompt: &str, echo: bool) -> Option<Zeroizing<String>>;
    fn notice(&self, request: &Request, notice: Notice);
    fn close(&self, request: &Request);
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
pub enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Failed(String),
    Cancelled(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    Session(String),
    Process { pid: u32, start_time: u64 },
}

impl Subject {
    pub fn process(pid: u32) -> std::io::Result<Self> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let after_name = stat
            .rsplit_once(')')
            .map(|(_, fields)| fields)
            .ok_or_else(|| std::io::Error::other("malformed /proc stat"))?;
        let start_time = after_name
            .split_whitespace()
            .nth(19)
            .and_then(|field| field.parse().ok())
            .ok_or_else(|| std::io::Error::other("no start time in /proc stat"))?;
        Ok(Self::Process { pid, start_time })
    }

    fn to_variant(&self) -> (&'static str, HashMap<&'static str, Value<'_>>) {
        match self {
            Self::Session(id) => (
                "unix-session",
                HashMap::from([("session-id", Value::from(id.as_str()))]),
            ),
            Self::Process { pid, start_time } => (
                "unix-process",
                HashMap::from([
                    ("pid", Value::from(*pid)),
                    ("start-time", Value::from(*start_time)),
                ]),
            ),
        }
    }
}

pub struct Agent {
    connection: Connection,
    subject: Subject,
}

impl Agent {
    pub fn register(
        system: &Connection,
        subject: Subject,
        locale: &str,
        helper: PathBuf,
        conversation: Arc<dyn Conversation>,
    ) -> zbus::Result<Self> {
        system.object_server().at(
            OBJECT_PATH,
            AuthenticationAgent {
                conversation,
                helper,
                running: Arc::default(),
            },
        )?;
        system.call_method(
            Some(AUTHORITY),
            AUTHORITY_PATH,
            Some(AUTHORITY_INTERFACE),
            "RegisterAuthenticationAgent",
            &(subject.to_variant(), locale, OBJECT_PATH),
        )?;
        Ok(Self {
            connection: system.clone(),
            subject,
        })
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.connection.call_method(
            Some(AUTHORITY),
            AUTHORITY_PATH,
            Some(AUTHORITY_INTERFACE),
            "UnregisterAuthenticationAgent",
            &(self.subject.to_variant(), OBJECT_PATH),
        );
        let _ = self
            .connection
            .object_server()
            .remove::<AuthenticationAgent, _>(OBJECT_PATH);
    }
}

type Identity = (String, HashMap<String, OwnedValue>);

fn user_name(uid: u32) -> Option<String> {
    let output = Command::new("getent")
        .args(["passwd", &uid.to_string()])
        .output()
        .ok()?;
    let entry = String::from_utf8(output.stdout).ok()?;
    entry
        .split(':')
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn choose_user(identities: &[Identity]) -> Option<String> {
    let uids: Vec<u32> = identities
        .iter()
        .filter(|(kind, _)| kind == "unix-user")
        .filter_map(|(_, details)| u32::try_from(details.get("uid")?).ok())
        .collect();
    let own = rustix::process::getuid().as_raw();
    let uid = uids
        .iter()
        .copied()
        .find(|uid| *uid == own)
        .or_else(|| uids.first().copied())?;
    user_name(uid)
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Authorized,
    Cancelled,
}

#[derive(Default)]
struct Running {
    sessions: HashMap<String, (Request, Option<Canceller>)>,
}

type Shared = Arc<Mutex<Running>>;

fn lock(running: &Shared) -> MutexGuard<'_, Running> {
    running.lock().unwrap_or_else(PoisonError::into_inner)
}

fn converse(
    conversation: &dyn Conversation,
    request: &Request,
    helper: &std::path::Path,
    cookie: &str,
    running: &Shared,
) -> Result<Outcome, AgentError> {
    let failed = |error: std::io::Error| AgentError::Failed(error.to_string());
    loop {
        let mut session = HelperSession::start(helper, &request.user, cookie).map_err(failed)?;
        {
            let mut running = lock(running);
            match running.sessions.get_mut(cookie) {
                Some((_, slot)) => *slot = Some(session.canceller()),
                None => return Ok(Outcome::Cancelled),
            }
        }
        let mut answered = false;
        loop {
            match session.read_line().map_err(failed)? {
                Some(Line::Prompt { text, echo }) => {
                    let response = conversation.respond(request, &text, echo);
                    match response {
                        Some(response) if lock(running).sessions.contains_key(cookie) => {
                            session.respond(&response).map_err(failed)?;
                            answered = true;
                        }
                        _ => return Ok(Outcome::Cancelled),
                    }
                }
                Some(Line::Info(text)) => conversation.notice(request, Notice::Info(text)),
                Some(Line::Error(text)) => conversation.notice(request, Notice::Error(text)),
                Some(Line::Success) => return Ok(Outcome::Authorized),
                Some(Line::Failure) => {
                    if !lock(running).sessions.contains_key(cookie) {
                        return Ok(Outcome::Cancelled);
                    }
                    if !answered {
                        return Err(AgentError::Failed(
                            "the helper refused without asking for a password".to_owned(),
                        ));
                    }
                    conversation.notice(request, Notice::WrongPassword);
                    break;
                }
                None => return Ok(Outcome::Cancelled),
            }
        }
    }
}

async fn from_polkitd(
    connection: &zbus::Connection,
    header: &Header<'_>,
) -> Result<(), AgentError> {
    let owner = DBusProxy::new(connection)
        .await?
        .get_name_owner(BusName::from_static_str(AUTHORITY).map_err(zbus::Error::from)?)
        .await
        .map_err(zbus::Error::from)?;
    if header.sender().map(UniqueName::as_str) == Some(owner.as_str()) {
        Ok(())
    } else {
        Err(AgentError::Failed(
            "only polkitd may ask for authentication".to_owned(),
        ))
    }
}

struct AuthenticationAgent {
    conversation: Arc<dyn Conversation>,
    helper: PathBuf,
    running: Shared,
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl AuthenticationAgent {
    #[allow(clippy::too_many_arguments)]
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<Identity>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), AgentError> {
        from_polkitd(connection, &header).await?;
        let user = choose_user(&identities)
            .ok_or_else(|| AgentError::Failed("no user to authenticate as".to_owned()))?;
        let request = Request {
            action_id,
            message,
            icon_name,
            details,
            user,
        };
        lock(&self.running)
            .sessions
            .insert(cookie.clone(), (request.clone(), None));
        let conversation = Arc::clone(&self.conversation);
        let helper = self.helper.clone();
        let running = Arc::clone(&self.running);
        let outcome = blocking::unblock(move || {
            let outcome = converse(conversation.as_ref(), &request, &helper, &cookie, &running);
            if lock(&running).sessions.remove(&cookie).is_some() {
                conversation.close(&request);
            }
            outcome
        })
        .await?;
        match outcome {
            Outcome::Authorized => Ok(()),
            Outcome::Cancelled => Err(AgentError::Cancelled(
                "the authentication was cancelled".to_owned(),
            )),
        }
    }

    async fn cancel_authentication(
        &self,
        cookie: String,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), AgentError> {
        from_polkitd(connection, &header).await?;
        let session = lock(&self.running).sessions.remove(&cookie);
        match session {
            Some((request, canceller)) => {
                if let Some(canceller) = canceller {
                    canceller.cancel();
                }
                self.conversation.close(&request);
                Ok(())
            }
            None => Err(AgentError::Failed(format!(
                "no authentication with cookie {cookie}"
            ))),
        }
    }
}
