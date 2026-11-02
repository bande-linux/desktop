use std::collections::HashMap;
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
        let signals = MessageIterator::for_match_rule(rule, system, None)?;
        let objects = Arc::new(Mutex::new(Objects::load(system)?));
        let shared = Arc::clone(&objects);
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
                for signal in signals.flatten() {
                    let mut objects = lock(&shared);
                    if let Some(change) = objects.apply(&signal) {
                        changed(&objects, &change);
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

fn lock(objects: &Mutex<Objects>) -> MutexGuard<'_, Objects> {
    objects.lock().unwrap_or_else(PoisonError::into_inner)
}
