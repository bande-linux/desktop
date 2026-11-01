use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};

use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::{self, Message};
use zbus::names::BusName;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{MatchRule, OwnedMatchRule};

use crate::{
    Client, Details, Error, ErrorCode, Failure, Info, Package, PackageKitProxyBlocking, Progress,
    SERVICE, Status,
};

const BUS: &str = "org.freedesktop.DBus";
const TRANSACTION: &str = "org.freedesktop.PackageKit.Transaction";
const UNKNOWN_PERCENTAGE: u32 = 101;
const RESTART_SYSTEM: u32 = 4;
const RESTART_SECURITY_SYSTEM: u32 = 6;
const EXIT_SUCCESS: u32 = 1;
const EXIT_CANCELLED: u32 = 3;

#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub(crate) packages: Vec<Package>,
    pub(crate) details: Vec<Details>,
    pub(crate) files: Vec<String>,
    pub(crate) restart: bool,
    pub(crate) failure: Option<Failure>,
}

type Running = Option<(Connection, OwnedObjectPath)>;

#[derive(Clone, Default)]
pub struct Canceller(Arc<Mutex<Running>>);

impl Canceller {
    pub fn cancel(&self) -> zbus::Result<()> {
        let running = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some((system, path)) = running else {
            return Ok(());
        };
        Proxy::new(&system, SERVICE, &path, TRANSACTION)?.call("Cancel", &())
    }

    fn set(&self, running: Running) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = running;
    }
}

fn left(message: &Message) -> bool {
    let header = message.header();
    header.interface().is_some_and(|interface| interface == BUS)
        && header
            .member()
            .is_some_and(|member| member == "NameOwnerChanged")
        && message
            .body()
            .deserialize::<(String, String, String)>()
            .is_ok_and(|(name, old, _)| name == SERVICE && !old.is_empty())
}

fn from_transaction(message: &Message, path: &OwnedObjectPath) -> bool {
    let header = message.header();
    header.message_type() == message::Type::Signal
        && header
            .path()
            .is_some_and(|from| from.as_str() == path.as_str())
}

fn finished(message: &Message) -> bool {
    message
        .header()
        .member()
        .is_some_and(|member| member == "Finished")
}

fn relay(messages: MessageIterator, path: OwnedObjectPath, relayed: Sender<Message>) {
    for message in messages.flatten() {
        let gone = left(&message);
        let wanted = gone || from_transaction(&message, &path);
        if !wanted {
            continue;
        }
        let last = gone || finished(&message);
        if relayed.send(message).is_err() || last {
            break;
        }
    }
}

fn track(progress: &mut Progress, changed: &HashMap<String, OwnedValue>) {
    if let Some(percent) = changed
        .get("Percentage")
        .and_then(|value| u32::try_from(value).ok())
    {
        progress.percent = (percent < UNKNOWN_PERCENTAGE).then_some(percent);
    }
    if let Some(status) = changed
        .get("Status")
        .and_then(|value| u32::try_from(value).ok())
    {
        progress.status = Status::from_raw(status);
    }
    if let Some(allow) = changed
        .get("AllowCancel")
        .and_then(|value| bool::try_from(value).ok())
    {
        progress.allow_cancel = allow;
    }
}

fn listen(relayed: &Receiver<Message>, report: &dyn Fn(Progress)) -> Result<Outcome, Error> {
    let mut outcome = Outcome::default();
    let mut progress = Progress::default();
    for message in relayed.iter() {
        if left(&message) {
            return Err(Error::Vanished);
        }
        let header = message.header();
        let body = message.body();
        match header.member().map(|member| member.as_str()) {
            Some("Package") => {
                if let Ok((info, id, summary)) = body.deserialize::<(u32, String, String)>() {
                    outcome
                        .packages
                        .push(Package::new(Info::from_raw(info), &id, &summary));
                }
            }
            Some("Details") => {
                if let Ok(details) = body.deserialize::<HashMap<String, OwnedValue>>() {
                    outcome.details.push(Details::from_map(&details));
                }
            }
            Some("Files") => {
                if let Ok((_, files)) = body.deserialize::<(String, Vec<String>)>() {
                    outcome.files.extend(files);
                }
            }
            Some("RequireRestart") => {
                if let Ok((kind, _)) = body.deserialize::<(u32, String)>() {
                    outcome.restart |= matches!(kind, RESTART_SYSTEM | RESTART_SECURITY_SYSTEM);
                }
            }
            Some("ErrorCode") => {
                if let Ok((code, details)) = body.deserialize::<(u32, String)>() {
                    outcome.failure = Some(Failure {
                        code: ErrorCode::from_raw(code),
                        details,
                    });
                }
            }
            Some("PropertiesChanged") => {
                if let Ok((_, changed, _)) =
                    body.deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                {
                    track(&mut progress, &changed);
                    report(progress);
                }
            }
            Some("Finished") => {
                if let Ok((exit, _)) = body.deserialize::<(u32, u32)>()
                    && exit != EXIT_SUCCESS
                    && outcome.failure.is_none()
                {
                    let code = match exit {
                        EXIT_CANCELLED => ErrorCode::Cancelled,
                        other => ErrorCode::Other(other),
                    };
                    outcome.failure = Some(Failure {
                        code,
                        details: String::new(),
                    });
                }
                return Ok(outcome);
            }
            _ => {}
        }
    }
    Err(Error::Vanished)
}

fn rules(path: &OwnedObjectPath) -> zbus::Result<[OwnedMatchRule; 2]> {
    Ok([
        MatchRule::builder()
            .msg_type(message::Type::Signal)
            .path(path.clone())?
            .build()
            .into(),
        MatchRule::builder()
            .msg_type(message::Type::Signal)
            .sender(BUS)?
            .interface(BUS)?
            .member("NameOwnerChanged")?
            .arg(0, SERVICE)?
            .build()
            .into(),
    ])
}

pub(crate) fn run<B>(
    client: &Client,
    method: &str,
    body: &B,
    progress: &dyn Fn(Progress),
) -> Result<Outcome, Error>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let system = &client.system;
    let path = PackageKitProxyBlocking::new(system)?.create_transaction()?;
    let transaction = Proxy::new(system, SERVICE, &path, TRANSACTION)?;
    let _: () = transaction.call("SetHints", &(&client.hints,))?;
    let bus = DBusProxy::new(system)?;
    let rules = rules(&path)?;
    let (relayed, received) = mpsc::channel();
    let messages = MessageIterator::from(system);
    let watched = path.clone();
    std::thread::Builder::new()
        .name("packagekit-signals".to_owned())
        .spawn(move || relay(messages, watched, relayed))
        .map_err(|error| Error::Bus(zbus::Error::InputOutput(Arc::new(error))))?;
    client.canceller.set(Some((system.clone(), path.clone())));
    let outcome = (|| -> Result<Outcome, Error> {
        for rule in &rules {
            bus.add_match_rule(rule.inner().clone())?;
        }
        if !bus.name_has_owner(BusName::try_from(SERVICE)?)? {
            return Err(Error::Vanished);
        }
        let _: () = transaction.call(method, body)?;
        listen(&received, progress)
    })();
    client.canceller.set(None);
    for rule in rules {
        if let Err(error) = bus.remove_match_rule(rule.into_inner()) {
            tracing::warn!("cannot remove a PackageKit match rule: {error}");
        }
    }
    outcome
}
