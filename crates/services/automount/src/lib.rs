pub mod askpass;
pub mod devices;
pub mod discovery;
pub mod health;
pub mod mounts;
mod mounts_bus;
pub use bande_volumes::server;
pub mod space;

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_l10n::Localizer;
use bande_notify::Urgency;
use bande_notify::client::{self, Request};
use bande_volumes::udisks::{BLOCK, Change, FILESYSTEM, Objects, UDISKS, Udisks};
use devices::Devices;
use zbus::blocking::{Connection, MessageIterator};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{MatchRule, message};

pub static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-automount")));

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct Volume {
    label: String,
    automount: bool,
    mounted: bool,
}

impl Volume {
    fn from_objects(objects: &Objects, path: &str) -> Option<Self> {
        if !objects.has(path, BLOCK) || !objects.has(path, FILESYSTEM) {
            return None;
        }
        let flag = |key: &str| objects.flag(path, BLOCK, key);
        let label = ["IdLabel", "HintName"]
            .into_iter()
            .map(|key| objects.text(path, BLOCK, key))
            .find(|label| !label.is_empty())
            .unwrap_or_else(|| L10N.tr("untitled"));
        Some(Self {
            label,
            automount: flag("HintAuto") && !flag("HintIgnore") && !flag("HintSystem"),
            mounted: !objects.mount_points(path).is_empty(),
        })
    }
}

#[derive(Default)]
struct State {
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
    devices: Mutex<Option<Devices>>,
}

impl Automounter {
    pub fn follow_devices(&self, devices: Devices) {
        devices.session_changed(lock(&self.state).unlocked);
        *self.devices.lock().unwrap_or_else(PoisonError::into_inner) = Some(devices);
    }

    pub fn start(system: &Connection, session: &Connection) -> zbus::Result<Arc<Self>> {
        let automounter = Arc::new(Self {
            system: system.clone(),
            session: session.clone(),
            state: Shared::default(),
            devices: Mutex::new(None),
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
        let watcher = Arc::clone(&automounter);
        Udisks::follow(system, move |objects, change| {
            watcher.handle(objects, change)
        })?;
        Ok(automounter)
    }

    fn handle(&self, objects: &Objects, change: &Change) {
        match change {
            Change::Added(path) => self.appeared(objects, path),
            Change::Removed(path, gone) => self.disappeared(path, gone),
            Change::Changed(path, interface, keys)
                if interface == FILESYSTEM && keys.iter().any(|key| key == "MountPoints") =>
            {
                if let Some(volume) = lock(&self.state).volumes.get_mut(path) {
                    volume.mounted = !objects.mount_points(path).is_empty();
                }
            }
            Change::Changed(..) => {}
        }
    }

    fn appeared(&self, objects: &Objects, path: &OwnedObjectPath) {
        let mount = {
            let mut state = lock(&self.state);
            let Some(volume) = Volume::from_objects(objects, path) else {
                return;
            };
            if state.volumes.contains_key(path) {
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
            self.mount(path);
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
        if let Some(devices) = self
            .devices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            devices.session_changed(unlocked);
        }
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
                desktop_entry: None,
            },
        );
        if let Err(error) = posted {
            tracing::warn!("cannot post the notification: {error}");
        }
    }
}
