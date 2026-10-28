use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use unbound_notify::Urgency;
use unbound_notify::client::{self, Request};
use zbus::blocking::fdo::ObjectManagerProxy;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{MatchRule, message};

pub const UDISKS: &str = "org.freedesktop.UDisks2";
pub const UDISKS_PATH: &str = "/org/freedesktop/UDisks2";
pub const BLOCK: &str = "org.freedesktop.UDisks2.Block";
pub const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";

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
        .unwrap_or_else(|| "Untitled".to_owned());
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
        let added = manager.receive_interfaces_added()?;
        let removed = manager.receive_interfaces_removed()?;
        let rule = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .sender(UDISKS)?
            .interface("org.freedesktop.DBus.Properties")?
            .member("PropertiesChanged")?
            .path_namespace(UDISKS_PATH)?
            .arg(0, FILESYSTEM)?
            .build();
        let changes = MessageIterator::for_match_rule(rule, system, None)?;
        let automounter = Arc::new(Self {
            system: system.clone(),
            session: session.clone(),
            state: Shared::default(),
        });
        for (path, interfaces) in manager.get_managed_objects()? {
            let interfaces: Interfaces = interfaces
                .into_iter()
                .map(|(name, properties)| (name.to_string(), properties))
                .collect();
            automounter.appeared(path, &interfaces);
        }
        let watcher = Arc::clone(&automounter);
        thread::spawn(move || {
            for signal in added {
                if let Ok(arguments) = signal.args() {
                    let interfaces: Interfaces = arguments
                        .interfaces_and_properties()
                        .iter()
                        .map(|(name, properties)| {
                            (
                                name.to_string(),
                                properties
                                    .iter()
                                    .filter_map(|(key, value)| {
                                        Some((key.to_string(), value.try_to_owned().ok()?))
                                    })
                                    .collect(),
                            )
                        })
                        .collect();
                    watcher.appeared(arguments.object_path().to_owned().into(), &interfaces);
                }
            }
        });
        let watcher = Arc::clone(&automounter);
        thread::spawn(move || {
            for signal in removed {
                if let Ok(arguments) = signal.args() {
                    let gone: Vec<String> = arguments
                        .interfaces()
                        .iter()
                        .map(|interface| interface.to_string())
                        .collect();
                    watcher.disappeared(&arguments.object_path().to_owned().into(), &gone);
                }
            }
        });
        let watcher = Arc::clone(&automounter);
        thread::spawn(move || {
            for signal in changes.flatten() {
                let Some(path) = signal
                    .header()
                    .path()
                    .map(|path| OwnedObjectPath::from(path.to_owned()))
                else {
                    continue;
                };
                if let Ok((_, changed, _)) = signal
                    .body()
                    .deserialize::<(String, Properties, Vec<String>)>()
                    && changed.contains_key("MountPoints")
                    && let Some(volume) = lock(&watcher.state).volumes.get_mut(&path)
                {
                    volume.mounted = mounted(&changed);
                }
            }
        });
        Ok(automounter)
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
            let mount = volume.automount && !volume.mounted;
            state.volumes.insert(path.clone(), volume);
            mount
        };
        if mount {
            let options: HashMap<&str, OwnedValue> = HashMap::new();
            if let Err(error) =
                self.system
                    .call_method(Some(UDISKS), &path, Some(FILESYSTEM), "Mount", &(options,))
            {
                eprintln!("unbound-automount: cannot mount {path}: {error}");
            }
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
            state.volumes.remove(path)
        };
        let Some(volume) = volume else {
            return;
        };
        if !volume.mounted {
            return;
        }
        let body = format!(
            "Eject \u{201c}{}\u{201d} before disconnecting it, or files on it may be damaged.",
            volume.label
        );
        let posted = client::notify(
            &self.session,
            &Request {
                app_name: "Disks",
                replaces_id: 0,
                app_icon: "drive-removable-media",
                summary: "Disk Not Ejected Properly",
                body: &body,
                actions: &[],
                urgency: Urgency::Normal,
            },
        );
        if let Err(error) = posted {
            eprintln!("unbound-automount: cannot post the notification: {error}");
        }
    }
}
