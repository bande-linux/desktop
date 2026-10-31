use std::sync::{Arc, Mutex, MutexGuard};

use zbus::blocking::Connection;
use zbus::object_server::ObjectServer;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::SessionBus;

const MANAGER_PATH: &str = "/org/freedesktop/systemd1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedUnit {
    pub name: String,
    pub pids: Vec<u32>,
    pub slice: String,
    pub collect_mode: Option<String>,
}

#[derive(Default)]
struct Record {
    started: Vec<StartedUnit>,
    reset: Vec<String>,
}

type Shared = Arc<Mutex<Record>>;

fn lock(shared: &Shared) -> MutexGuard<'_, Record> {
    shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn unit_path(index: usize) -> zbus::fdo::Result<OwnedObjectPath> {
    OwnedObjectPath::try_from(format!("{MANAGER_PATH}/unit/{index}"))
        .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
}

fn index_of(record: &Record, name: &str) -> Option<usize> {
    record.started.iter().position(|unit| unit.name == name)
}

struct Manager(Shared);

#[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
impl Manager {
    async fn start_transient_unit(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        name: String,
        _mode: String,
        properties: Vec<(String, OwnedValue)>,
        _aux: Vec<(String, Vec<(String, OwnedValue)>)>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let text = |key: &str| {
            properties
                .iter()
                .find(|(name, _)| name == key)
                .and_then(|(_, value)| String::try_from(value.try_clone().ok()?).ok())
        };
        let pids = properties
            .iter()
            .find(|(name, _)| name == "PIDs")
            .and_then(|(_, value)| Vec::<u32>::try_from(value.try_clone().ok()?).ok())
            .unwrap_or_default();
        let unit = StartedUnit {
            pids,
            slice: text("Slice").unwrap_or_default(),
            collect_mode: text("CollectMode"),
            name,
        };
        let index = {
            let mut record = lock(&self.0);
            record.started.push(unit);
            record.started.len() - 1
        };
        let path = unit_path(index)?;
        server
            .at(
                &path,
                Unit {
                    active_state: "active".to_owned(),
                },
            )
            .await?;
        server
            .at(
                &path,
                Scope {
                    result: "success".to_owned(),
                },
            )
            .await?;
        OwnedObjectPath::try_from("/org/freedesktop/systemd1/job/1")
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    async fn get_unit(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        name: String,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let index = index_of(&lock(&self.0), &name);
        let path = index.map(unit_path).transpose()?;
        match path {
            Some(path) if server.interface::<_, Unit>(&path).await.is_ok() => Ok(path),
            _ => Err(zbus::fdo::Error::UnknownObject(name)),
        }
    }

    async fn reset_failed_unit(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        name: String,
    ) -> zbus::fdo::Result<()> {
        let index = {
            let mut record = lock(&self.0);
            record.reset.push(name.clone());
            index_of(&record, &name)
        };
        if let Some(path) = index.map(unit_path).transpose()? {
            server.remove::<Unit, _>(&path).await?;
            server.remove::<Scope, _>(&path).await?;
        }
        Ok(())
    }
}

struct Unit {
    active_state: String,
}

#[zbus::interface(name = "org.freedesktop.systemd1.Unit")]
impl Unit {
    #[zbus(property)]
    fn active_state(&self) -> String {
        self.active_state.clone()
    }
}

struct Scope {
    result: String,
}

#[zbus::interface(name = "org.freedesktop.systemd1.Scope")]
impl Scope {
    #[zbus(property)]
    fn result(&self) -> String {
        self.result.clone()
    }
}

pub struct FakeSystemd {
    systemd: Connection,
    record: Shared,
}

impl FakeSystemd {
    pub fn start(bus: &SessionBus) -> zbus::Result<Self> {
        let systemd = bus.connect()?;
        let record = Shared::default();
        systemd
            .object_server()
            .at(MANAGER_PATH, Manager(Arc::clone(&record)))?;
        systemd.request_name("org.freedesktop.systemd1")?;
        Ok(Self { systemd, record })
    }

    pub fn started(&self) -> Vec<StartedUnit> {
        lock(&self.record).started.clone()
    }

    pub fn reset(&self) -> Vec<String> {
        lock(&self.record).reset.clone()
    }

    fn path(&self, name: &str) -> zbus::Result<OwnedObjectPath> {
        let index = index_of(&lock(&self.record), name)
            .ok_or_else(|| zbus::Error::Failure(format!("{name} was never started")))?;
        Ok(unit_path(index)?)
    }

    pub fn fail(&self, name: &str, result: &str) -> zbus::Result<()> {
        let path = self.path(name)?;
        let server = self.systemd.object_server();
        server.interface::<_, Scope>(&path)?.get_mut().result = result.to_owned();
        server.interface::<_, Unit>(&path)?.get_mut().active_state = "failed".to_owned();
        Ok(())
    }

    pub fn end(&self, name: &str) -> zbus::Result<()> {
        let path = self.path(name)?;
        let server = self.systemd.object_server();
        server.remove::<Unit, _>(&path)?;
        server.remove::<Scope, _>(&path)?;
        Ok(())
    }
}
