pub mod space;

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_l10n::Localizer;
use bande_notify::Urgency;
use bande_notify::client::{self, Request};
use zbus::blocking::fdo::ObjectManagerProxy;
use zbus::blocking::{Connection, MessageIterator};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{MatchRule, Message, message};

const ENGLISH: &str = include_str!("../locales/en-US/bande-automount.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

const UDISKS: &str = "org.freedesktop.UDisks2";
const UDISKS_PATH: &str = "/org/freedesktop/UDisks2";
pub const BLOCK: &str = "org.freedesktop.UDisks2.Block";
pub const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const LOGIN1: &str = "org.freedesktop.login1";
const LOGIN1_SESSION: &str = "org.freedesktop.login1.Session";

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    fn get_session(&self, session_id: &str) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait Session {
    #[zbus(property)]
    fn active(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;
}

fn unlocked(session: &SessionProxyBlocking<'_>) -> bool {
    match (session.active(), session.locked_hint()) {
        (Ok(active), Ok(locked)) => active && !locked,
        (Err(error), _) | (_, Err(error)) => {
            tracing::warn!("cannot read the session state: {error}");
            false
        }
    }
}

fn follow_session(
    system: &Connection,
) -> zbus::Result<(SessionProxyBlocking<'static>, MessageIterator)> {
    let path = ManagerProxyBlocking::new(system)?.get_session("auto")?;
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .sender(LOGIN1)?
        .path(path.clone())?
        .interface(PROPERTIES)?
        .member("PropertiesChanged")?
        .build();
    let changes = MessageIterator::for_match_rule(rule, system, None)?;
    let session = SessionProxyBlocking::builder(system)
        .path(path)?
        .cache_properties(CacheProperties::No)
        .build()?;
    Ok((session, changes))
}

type Properties = HashMap<String, OwnedValue>;
type Interfaces = HashMap<String, Properties>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Volume {
    label: String,
    automount: bool,
    mounted: bool,
}

fn property<T: TryFrom<OwnedValue>>(properties: &Properties, key: &str) -> Option<T> {
    properties
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| T::try_from(value).ok())
}

fn mounted(filesystem: &Properties) -> bool {
    property::<Vec<Vec<u8>>>(filesystem, "MountPoints").is_some_and(|points| !points.is_empty())
}

impl Volume {
    fn from_interfaces(interfaces: &Interfaces) -> Option<Self> {
        let block = interfaces.get(BLOCK)?;
        let filesystem = interfaces.get(FILESYSTEM)?;
        let flag = |key: &str| property::<bool>(block, key).unwrap_or(false);
        let label = [
            property::<String>(block, "IdLabel"),
            property::<String>(block, "HintName"),
        ]
        .into_iter()
        .flatten()
        .find(|label| !label.is_empty())
        .unwrap_or_else(|| L10N.tr("untitled"));
        Some(Self {
            label,
            automount: flag("HintAuto") && !flag("HintIgnore") && !flag("HintSystem"),
            mounted: mounted(filesystem),
        })
    }
}

#[derive(Default)]
struct State {
    objects: HashMap<OwnedObjectPath, Interfaces>,
    volumes: HashMap<OwnedObjectPath, Volume>,
    unlocked: bool,
    waiting: HashSet<OwnedObjectPath>,
}

type Shared = Arc<Mutex<State>>;

fn lock(state: &Shared) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct Automounter {
    system: Connection,
    session: Connection,
    state: Shared,
}

impl Automounter {
    pub fn start(system: &Connection, session: &Connection) -> zbus::Result<Arc<Self>> {
        let manager = ObjectManagerProxy::builder(system)
            .destination(UDISKS)?
            .path(UDISKS_PATH)?
            .build()?;
        let rule = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .sender(UDISKS)?
            .path_namespace(UDISKS_PATH)?
            .build();
        let signals = MessageIterator::for_match_rule(rule, system, None)?;
        let automounter = Arc::new(Self {
            system: system.clone(),
            session: session.clone(),
            state: Shared::default(),
        });
        match follow_session(system) {
            Ok((login, changes)) => {
                lock(&automounter.state).unlocked = unlocked(&login);
                let follower = Arc::clone(&automounter);
                thread::spawn(move || {
                    for change in changes.flatten() {
                        if let Ok((LOGIN1_SESSION, _, _)) =
                            change
                                .body()
                                .deserialize::<(&str, Properties, Vec<String>)>()
                        {
                            follower.session_changed(unlocked(&login));
                        }
                    }
                });
            }
            Err(error) => tracing::warn!("cannot follow the login session: {error}"),
        }
        for (path, interfaces) in manager.get_managed_objects()? {
            let interfaces: Interfaces = interfaces
                .into_iter()
                .map(|(name, properties)| (name.to_string(), properties))
                .collect();
            automounter.appeared(path, &interfaces);
        }
        let watcher = Arc::clone(&automounter);
        thread::spawn(move || {
            for signal in signals.flatten() {
                watcher.handle(&signal);
            }
        });
        Ok(automounter)
    }

    fn handle(&self, signal: &Message) {
        let header = signal.header();
        let (Some(interface), Some(member)) = (header.interface(), header.member()) else {
            return;
        };
        let body = signal.body();
        match (interface.as_str(), member.as_str()) {
            (OBJECT_MANAGER, "InterfacesAdded") => {
                if let Ok((path, interfaces)) = body.deserialize::<(OwnedObjectPath, Interfaces)>()
                {
                    self.appeared(path, &interfaces);
                }
            }
            (OBJECT_MANAGER, "InterfacesRemoved") => {
                if let Ok((path, gone)) = body.deserialize::<(OwnedObjectPath, Vec<String>)>() {
                    self.disappeared(&path, &gone);
                }
            }
            (PROPERTIES, "PropertiesChanged") => {
                if let (Some(path), Ok((FILESYSTEM, changed, _))) = (
                    header.path(),
                    body.deserialize::<(&str, Properties, Vec<String>)>(),
                ) && changed.contains_key("MountPoints")
                    && let Some(volume) = lock(&self.state)
                        .volumes
                        .get_mut(&OwnedObjectPath::from(path.to_owned()))
                {
                    volume.mounted = mounted(&changed);
                }
            }
            _ => {}
        }
    }

    fn appeared(&self, path: OwnedObjectPath, interfaces: &Interfaces) {
        let mount = {
            let mut state = lock(&self.state);
            let object = state.objects.entry(path.clone()).or_default();
            object.extend(interfaces.clone());
            let Some(volume) = Volume::from_interfaces(object) else {
                return;
            };
            if state.volumes.contains_key(&path) {
                return;
            }
            let wanted = volume.automount && !volume.mounted;
            state.volumes.insert(path.clone(), volume);
            if wanted && !state.unlocked {
                state.waiting.insert(path.clone());
                return;
            }
            wanted
        };
        if mount {
            self.mount(&path);
        }
    }

    fn mount(&self, path: &OwnedObjectPath) {
        let options: HashMap<&str, OwnedValue> = HashMap::new();
        if let Err(error) =
            self.system
                .call_method(Some(UDISKS), path, Some(FILESYSTEM), "Mount", &(options,))
        {
            tracing::warn!("cannot mount {path}: {error}");
        }
    }

    fn session_changed(&self, unlocked: bool) {
        let ready: Vec<OwnedObjectPath> = {
            let mut state = lock(&self.state);
            state.unlocked = unlocked;
            if !unlocked {
                return;
            }
            let waiting = std::mem::take(&mut state.waiting);
            waiting
                .into_iter()
                .filter(|path| {
                    state
                        .volumes
                        .get(path)
                        .is_some_and(|volume| volume.automount && !volume.mounted)
                })
                .collect()
        };
        for path in &ready {
            self.mount(path);
        }
    }

    fn disappeared(&self, path: &OwnedObjectPath, interfaces: &[String]) {
        let volume = {
            let mut state = lock(&self.state);
            if let Some(object) = state.objects.get_mut(path) {
                object.retain(|name, _| !interfaces.contains(name));
                if object.is_empty() {
                    state.objects.remove(path);
                }
            }
            if !interfaces
                .iter()
                .any(|name| name == FILESYSTEM || name == BLOCK)
            {
                return;
            }
            state.waiting.remove(path);
            state.volumes.remove(path)
        };
        let Some(volume) = volume else {
            return;
        };
        if !volume.mounted {
            return;
        }
        let body = L10N.tr_args(
            "not-ejected-body",
            [("label", volume.label.as_str().into())],
        );
        let app_name = L10N.tr("app-name");
        let summary = L10N.tr("not-ejected");
        let posted = client::notify(
            &self.session,
            &Request {
                app_name: &app_name,
                replaces_id: 0,
                app_icon: "drive-removable-media",
                summary: &summary,
                body: &body,
                actions: &[],
                urgency: Urgency::Normal,
                sound: None,
            },
        );
        if let Err(error) = posted {
            tracing::warn!("cannot post the notification: {error}");
        }
    }
}
