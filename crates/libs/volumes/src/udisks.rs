use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use zbus::blocking::fdo::ObjectManagerProxy;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};
use zbus::{MatchRule, Message, message};

pub const UDISKS: &str = "org.freedesktop.UDisks2";
pub const UDISKS_PATH: &str = "/org/freedesktop/UDisks2";
pub const MANAGER_PATH: &str = "/org/freedesktop/UDisks2/Manager";
pub const MANAGER: &str = "org.freedesktop.UDisks2.Manager";
pub const BLOCK: &str = "org.freedesktop.UDisks2.Block";
pub const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
pub const DRIVE: &str = "org.freedesktop.UDisks2.Drive";
pub const ATA: &str = "org.freedesktop.UDisks2.Drive.Ata";
pub const NVME: &str = "org.freedesktop.UDisks2.NVMe.Controller";
pub const PARTITION: &str = "org.freedesktop.UDisks2.Partition";
pub const PARTITION_TABLE: &str = "org.freedesktop.UDisks2.PartitionTable";
pub const LOOP: &str = "org.freedesktop.UDisks2.Loop";
pub const ENCRYPTED: &str = "org.freedesktop.UDisks2.Encrypted";
pub const SWAPSPACE: &str = "org.freedesktop.UDisks2.Swapspace";
const BUS: &str = "org.freedesktop.DBus";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const NONE: &str = "/";

pub type Properties = HashMap<String, OwnedValue>;
pub type Interfaces = HashMap<String, Properties>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added(OwnedObjectPath),
    Removed(OwnedObjectPath, Vec<String>),
    Changed(OwnedObjectPath, String, Vec<String>),
}

#[derive(Debug, Default)]
pub struct Objects(HashMap<OwnedObjectPath, Interfaces>);

impl From<HashMap<OwnedObjectPath, Interfaces>> for Objects {
    fn from(objects: HashMap<OwnedObjectPath, Interfaces>) -> Self {
        Self(objects)
    }
}

impl Objects {
    pub fn load(system: &Connection) -> zbus::Result<Self> {
        let manager = ObjectManagerProxy::builder(system)
            .destination(UDISKS)?
            .path(UDISKS_PATH)?
            .build()?;
        Ok(Self(
            manager
                .get_managed_objects()?
                .into_iter()
                .map(|(path, interfaces)| {
                    let interfaces = interfaces
                        .into_iter()
                        .map(|(name, properties)| (name.to_string(), properties))
                        .collect();
                    (path, interfaces)
                })
                .collect(),
        ))
    }

    pub fn paths(&self) -> impl Iterator<Item = &OwnedObjectPath> {
        self.0.keys()
    }

    pub fn interfaces(&self, path: &str) -> Option<&Interfaces> {
        let path: OwnedObjectPath = ObjectPath::try_from(path).ok()?.into();
        self.0.get(&path)
    }

    pub fn has(&self, path: &str, interface: &str) -> bool {
        self.interfaces(path)
            .is_some_and(|interfaces| interfaces.contains_key(interface))
    }

    pub fn get<T: TryFrom<OwnedValue>>(&self, path: &str, interface: &str, key: &str) -> Option<T> {
        property(self.interfaces(path)?.get(interface)?, key)
    }

    pub fn path(&self, path: &str, interface: &str, key: &str) -> Option<String> {
        self.get::<OwnedObjectPath>(path, interface, key)
            .map(|path| path.to_string())
            .filter(|path| path != NONE)
    }

    pub fn text(&self, path: &str, interface: &str, key: &str) -> String {
        self.get::<String>(path, interface, key).unwrap_or_default()
    }

    pub fn flag(&self, path: &str, interface: &str, key: &str) -> bool {
        self.get::<bool>(path, interface, key).unwrap_or(false)
    }

    pub fn mount_points(&self, path: &str) -> Vec<String> {
        self.get::<Vec<Vec<u8>>>(path, FILESYSTEM, "MountPoints")
            .unwrap_or_default()
            .into_iter()
            .map(|point| {
                let point = point.strip_suffix(&[0]).unwrap_or(&point);
                String::from_utf8_lossy(point).into_owned()
            })
            .collect()
    }

    pub fn device(&self, path: &str) -> String {
        self.get::<Vec<u8>>(path, BLOCK, "PreferredDevice")
            .map(|device| {
                let device = device.strip_suffix(&[0]).unwrap_or(&device);
                String::from_utf8_lossy(device).into_owned()
            })
            .unwrap_or_default()
    }

    fn replace(&mut self, fresh: Self) -> Vec<Change> {
        let mut changes = Vec::new();
        for (path, interfaces) in &self.0 {
            let kept = fresh.0.get(path);
            let gone: Vec<String> = interfaces
                .keys()
                .filter(|name| !kept.is_some_and(|now| now.contains_key(*name)))
                .cloned()
                .collect();
            if !gone.is_empty() {
                changes.push(Change::Removed(path.clone(), gone));
            }
        }
        for (path, interfaces) in &fresh.0 {
            let Some(before) = self
                .0
                .get(path)
                .filter(|before| interfaces.keys().all(|name| before.contains_key(name)))
            else {
                changes.push(Change::Added(path.clone()));
                continue;
            };
            for (interface, properties) in interfaces {
                let old = before.get(interface);
                let differs =
                    |key: &String| old.and_then(|old| old.get(key)) != properties.get(key);
                let keys: Vec<String> = properties
                    .keys()
                    .chain(old.into_iter().flat_map(HashMap::keys))
                    .filter(|key| differs(key))
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .cloned()
                    .collect();
                if !keys.is_empty() {
                    changes.push(Change::Changed(path.clone(), interface.clone(), keys));
                }
            }
        }
        *self = fresh;
        changes
    }

    fn apply(&mut self, signal: &Message) -> Option<Change> {
        let header = signal.header();
        let body = signal.body();
        match (header.interface()?.as_str(), header.member()?.as_str()) {
            (OBJECT_MANAGER, "InterfacesAdded") => {
                let (path, added) = body.deserialize::<(OwnedObjectPath, Interfaces)>().ok()?;
                self.0.entry(path.clone()).or_default().extend(added);
                Some(Change::Added(path))
            }
            (OBJECT_MANAGER, "InterfacesRemoved") => {
                let (path, gone) = body.deserialize::<(OwnedObjectPath, Vec<String>)>().ok()?;
                if let Some(object) = self.0.get_mut(&path) {
                    object.retain(|name, _| !gone.contains(name));
                    if object.is_empty() {
                        self.0.remove(&path);
                    }
                }
                Some(Change::Removed(path, gone))
            }
            (PROPERTIES, "PropertiesChanged") => {
                let path = OwnedObjectPath::from(header.path()?.to_owned());
                let (interface, changed, invalidated) = body
                    .deserialize::<(String, Properties, Vec<String>)>()
                    .ok()?;
                let properties = self.0.get_mut(&path)?.get_mut(&interface)?;
                let mut keys: Vec<String> = changed.keys().cloned().collect();
                for key in &invalidated {
                    properties.remove(key);
                }
                properties.extend(changed);
                keys.extend(invalidated);
                Some(Change::Changed(path, interface, keys))
            }
            _ => None,
        }
    }
}

pub fn property<T: TryFrom<OwnedValue>>(properties: &Properties, key: &str) -> Option<T> {
    properties
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| T::try_from(value).ok())
}

pub struct Udisks {
    system: Connection,
    objects: Arc<Mutex<Objects>>,
}

impl Udisks {
    pub fn follow(
        system: &Connection,
        mut changed: impl FnMut(&Objects, &Change) + Send + 'static,
    ) -> zbus::Result<Self> {
        let rule = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .sender(UDISKS)?
            .path_namespace(UDISKS_PATH)?
            .build();
        let owner = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .sender(BUS)?
            .interface(BUS)?
            .member("NameOwnerChanged")?
            .add_arg(UDISKS)?
            .build();
        let signals = MessageIterator::for_match_rule(rule, system, None)?;
        let owners = MessageIterator::for_match_rule(owner, system, None)?;
        let (sender, received) = mpsc::channel();
        forward(signals, sender.clone())?;
        forward(owners, sender)?;
        let loaded = load(system);
        let mut ready = loaded.is_some();
        let objects = Arc::new(Mutex::new(loaded.unwrap_or_default()));
        let shared = Arc::clone(&objects);
        let connection = system.clone();
        thread::Builder::new()
            .name("udisks".to_owned())
            .spawn(move || {
                {
                    let objects = lock(&shared);
                    let paths: Vec<OwnedObjectPath> = objects.paths().cloned().collect();
                    for path in paths {
                        changed(&objects, &Change::Added(path));
                    }
                }
                for signal in received {
                    let changes = match owner_change(&signal) {
                        Some(false) => {
                            ready = false;
                            continue;
                        }
                        Some(true) => resync(&connection, &shared, &mut ready),
                        None if !ready => resync(&connection, &shared, &mut ready),
                        None => lock(&shared).apply(&signal).into_iter().collect(),
                    };
                    let objects = lock(&shared);
                    for change in &changes {
                        changed(&objects, change);
                    }
                }
            })
            .map_err(|error| zbus::Error::Failure(error.to_string()))?;
        Ok(Self {
            system: system.clone(),
            objects,
        })
    }

    pub fn objects(&self) -> MutexGuard<'_, Objects> {
        lock(&self.objects)
    }

    pub fn system(&self) -> &Connection {
        &self.system
    }
}

fn forward(messages: MessageIterator, sender: Sender<Message>) -> zbus::Result<()> {
    thread::Builder::new()
        .name("udisks signals".to_owned())
        .spawn(move || {
            for message in messages.flatten() {
                if sender.send(message).is_err() {
                    return;
                }
            }
        })
        .map(drop)
        .map_err(|error| zbus::Error::Failure(error.to_string()))
}

fn owner_change(signal: &Message) -> Option<bool> {
    let header = signal.header();
    if header.member()?.as_str() != "NameOwnerChanged" || header.interface()?.as_str() != BUS {
        return None;
    }
    let (_, _, owner) = signal
        .body()
        .deserialize::<(String, String, String)>()
        .ok()?;
    Some(!owner.is_empty())
}

fn load(system: &Connection) -> Option<Objects> {
    Objects::load(system)
        .inspect_err(|error| tracing::info!("UDisks2 is not ready, waiting for it: {error}"))
        .ok()
}

fn resync(system: &Connection, objects: &Mutex<Objects>, ready: &mut bool) -> Vec<Change> {
    let Some(fresh) = load(system) else {
        *ready = false;
        return Vec::new();
    };
    *ready = true;
    lock(objects).replace(fresh)
}

fn lock(objects: &Mutex<Objects>) -> MutexGuard<'_, Objects> {
    objects.lock().unwrap_or_else(PoisonError::into_inner)
}
