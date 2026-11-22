pub mod askpass;
pub mod devices;
pub mod discovery;
mod failing;
pub mod health;
pub mod mounts;
mod mounts_bus;
mod notice;
mod unlock;
pub use bande_volumes::server;
pub mod space;
mod thunderbolt;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_l10n::tr;
use bande_notify::Urgency;
use bande_notify::client::{self, Request};
use bande_volumes::udisks::{BLOCK, Change, ENCRYPTED, FILESYSTEM, Objects, Udisks};
use devices::Devices;
use failing::FailingDisks;
use thunderbolt::Thunderbolt;
use unlock::{Prompt, Unlocked};
use zbus::blocking::{Connection, MessageIterator};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{MatchRule, message};

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

fn label(objects: &Objects, path: &str) -> String {
    ["IdLabel", "HintName"]
        .into_iter()
        .map(|key| objects.text(path, BLOCK, key))
        .find(|label| !label.is_empty())
        .unwrap_or_else(|| tr!("Untitled"))
}

fn hinted_automount(objects: &Objects, path: &str) -> bool {
    let flag = |key: &str| objects.flag(path, BLOCK, key);
    flag("HintAuto") && !flag("HintIgnore") && !flag("HintSystem")
}

fn locked_label(objects: &Objects, path: &str) -> Option<String> {
    let locked = objects.has(path, BLOCK)
        && objects.has(path, ENCRYPTED)
        && objects
            .path(path, ENCRYPTED, "CleartextDevice")
            .is_none_or(|cleartext| cleartext == "/");
    (locked && hinted_automount(objects, path)).then(|| label(objects, path))
}

impl Volume {
    fn from_objects(objects: &Objects, path: &str) -> Option<Self> {
        if !objects.has(path, BLOCK) || !objects.has(path, FILESYSTEM) {
            return None;
        }
        Some(Self {
            label: label(objects, path),
            automount: hinted_automount(objects, path),
            mounted: !objects.mount_points(path).is_empty(),
        })
    }
}

#[derive(Default)]
struct State {
    volumes: HashMap<OwnedObjectPath, Volume>,
    locked: HashMap<OwnedObjectPath, String>,
    unlocked: bool,
    waiting: HashSet<OwnedObjectPath>,
    prompts: HashMap<OwnedObjectPath, Prompt>,
}

type Shared = Arc<Mutex<State>>;

fn lock(state: &Shared) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn ask_to_unlock(
    system: Connection,
    session: Connection,
    state: Shared,
    path: OwnedObjectPath,
    label: String,
) {
    let spawned = thread::Builder::new()
        .name("unlock".to_owned())
        .spawn(move || {
            let unlocked = unlock::ask_and_unlock(&system, &path, &label, |prompt| {
                let mut state = lock(&state);
                if state.locked.contains_key(&path) {
                    state.prompts.insert(path.clone(), prompt);
                } else {
                    prompt.close();
                }
            });
            let dismissed = {
                let mut state = lock(&state);
                state.prompts.remove(&path).is_some() && state.locked.contains_key(&path)
            };
            match unlocked {
                Unlocked::Opened(cleartext) => {
                    let mount = {
                        let mut state = lock(&state);
                        match state.volumes.get(&cleartext) {
                            Some(volume) => !volume.automount && !volume.mounted,
                            None => {
                                state.volumes.insert(
                                    cleartext.clone(),
                                    Volume {
                                        label: label.clone(),
                                        automount: false,
                                        mounted: false,
                                    },
                                );
                                true
                            }
                        }
                    };
                    if mount {
                        notice::mount(&system, &session, &cleartext, &label);
                    }
                }
                Unlocked::Cancelled if dismissed => {
                    let again = (session.clone(), label.clone());
                    notice::still_locked(&session, &label, move || {
                        let still_locked = {
                            let state = lock(&state);
                            state.locked.contains_key(&path) && !state.prompts.contains_key(&path)
                        };
                        if still_locked {
                            ask_to_unlock(system, again.0, state, path, again.1);
                        }
                    });
                }
                Unlocked::Cancelled => {}
                Unlocked::NoPrompt => notice::not_unlocked(&session, &label),
                Unlocked::Failed(error) => {
                    tracing::warn!("cannot unlock {path}: {error}");
                    notice::not_unlocked(&session, &label);
                }
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot ask for the passphrase: {error}");
    }
}

pub struct Automounter {
    system: Connection,
    session: Connection,
    state: Shared,
    devices: Mutex<Option<Devices>>,
    failing: FailingDisks,
    thunderbolt: Thunderbolt,
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
            failing: FailingDisks::new(session),
            thunderbolt: Thunderbolt::new(system, session),
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
        automounter
            .thunderbolt
            .session_changed(lock(&automounter.state).unlocked);
        if let Err(error) = automounter.thunderbolt.follow() {
            tracing::warn!("Thunderbolt devices are not followed: {error}");
        }
        let watcher = Arc::clone(&automounter);
        Udisks::follow(system, move |objects, change| {
            watcher.handle(objects, change)
        })?;
        Ok(automounter)
    }

    fn handle(&self, objects: &Objects, change: &Change) {
        self.failing.changed(objects, change);
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
            Change::Changed(path, interface, keys)
                if interface == ENCRYPTED && keys.iter().any(|key| key == "CleartextDevice") =>
            {
                if objects.path(path, ENCRYPTED, "CleartextDevice").is_some() {
                    let mut state = lock(&self.state);
                    state.locked.remove(path);
                    state.waiting.remove(path);
                    if let Some(prompt) = state.prompts.remove(path) {
                        prompt.close();
                    }
                }
            }
            Change::Changed(..) => {}
        }
    }

    fn appeared(&self, objects: &Objects, path: &OwnedObjectPath) {
        if let Some(label) = locked_label(objects, path) {
            let ask = {
                let mut state = lock(&self.state);
                if state.locked.insert(path.clone(), label.clone()).is_some() {
                    return;
                }
                if !state.unlocked {
                    state.waiting.insert(path.clone());
                }
                state.unlocked
            };
            if ask {
                self.unlock(path, label);
            }
            return;
        }
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

    fn unlock(&self, path: &OwnedObjectPath, label: String) {
        ask_to_unlock(
            self.system.clone(),
            self.session.clone(),
            Arc::clone(&self.state),
            path.clone(),
            label,
        );
    }

    fn mount(&self, path: &OwnedObjectPath) {
        let label = lock(&self.state)
            .volumes
            .get(path)
            .map(|volume| volume.label.clone())
            .unwrap_or_default();
        notice::mount(&self.system, &self.session, path, &label);
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
        self.thunderbolt.session_changed(unlocked);
        let mut unlocks = Vec::new();
        let ready: Vec<OwnedObjectPath> = {
            let mut state = lock(&self.state);
            state.unlocked = unlocked;
            if !unlocked {
                return;
            }
            let waiting = std::mem::take(&mut state.waiting);
            unlocks.extend(
                waiting
                    .iter()
                    .filter_map(|path| Some((path.clone(), state.locked.get(path)?.clone()))),
            );
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
        for (path, label) in unlocks {
            self.unlock(&path, label);
        }
        for path in &ready {
            self.mount(path);
        }
    }

    fn disappeared(&self, path: &OwnedObjectPath, interfaces: &[String]) {
        let volume = {
            let mut state = lock(&self.state);
            if interfaces
                .iter()
                .any(|name| name == ENCRYPTED || name == BLOCK)
            {
                state.locked.remove(path);
                if let Some(prompt) = state.prompts.remove(path) {
                    prompt.close();
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
        let body = tr!(
            "Eject “{label}” before disconnecting it, or files on it may be damaged.",
            label = volume.label.as_str()
        );
        let app_name = tr!("Disks");
        let summary = tr!("Disk Not Ejected Properly");
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
