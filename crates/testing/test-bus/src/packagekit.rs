use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use zbus::blocking::Connection;
use zbus::object_server::{ObjectServer, SignalEmitter};
use zbus::zvariant::OwnedObjectPath;

use crate::SessionBus;

pub const ONLY_TRUSTED: u64 = 1 << 1;
pub const SIMULATE: u64 = 1 << 2;
pub const ONLY_DOWNLOAD: u64 = 1 << 3;
const INFO_AVAILABLE: u32 = 2;
const INFO_INSTALLING: u32 = 12;
const RESTART_NONE: u32 = 1;
const EXIT_SUCCESS: u32 = 1;
const TRANSACTION: &str = "org.freedesktop.PackageKit.Transaction";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub method: String,
    pub flags: u64,
    pub args: Vec<String>,
}

#[derive(Default)]
struct World {
    updates: Vec<String>,
    pulled_in: Vec<String>,
    calls: Vec<Call>,
    transactions: u32,
}

type Shared = Arc<Mutex<World>>;

fn lock(world: &Shared) -> MutexGuard<'_, World> {
    world.lock().unwrap_or_else(PoisonError::into_inner)
}

fn record(world: &Shared, method: &str, flags: u64, args: Vec<String>) {
    lock(world).calls.push(Call {
        method: method.to_owned(),
        flags,
        args,
    });
}

struct PackageKit(Shared);

#[zbus::interface(name = "org.freedesktop.PackageKit")]
impl PackageKit {
    async fn create_transaction(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let number = {
            let mut world = lock(&self.0);
            world.transactions += 1;
            world.transactions
        };
        let failed = |error: zbus::Error| zbus::fdo::Error::Failed(error.to_string());
        let path = OwnedObjectPath::try_from(format!("/org/freedesktop/PackageKit/{number}"))
            .map_err(|error| failed(error.into()))?;
        server
            .at(&path, Transaction(Arc::clone(&self.0)))
            .await
            .map_err(failed)?;
        Ok(path)
    }

    fn get_time_since_action(&self, _role: u32) -> u32 {
        0
    }
}

struct Offline(Shared);

#[zbus::interface(name = "org.freedesktop.PackageKit.Offline")]
impl Offline {
    fn trigger(&self, action: String) {
        record(&self.0, "Trigger", 0, vec![action]);
    }
}

struct Transaction(Shared);

#[zbus::interface(name = "org.freedesktop.PackageKit.Transaction")]
impl Transaction {
    fn set_hints(&self, _hints: Vec<String>) {}

    async fn get_updates(&self, _filter: u64, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let updates = lock(&self.0).updates.clone();
        for id in &updates {
            let _ = Self::package(&emitter, INFO_AVAILABLE, id, "").await;
        }
        let _ = Self::finished(&emitter, EXIT_SUCCESS, 1).await;
    }

    async fn get_details(
        &self,
        _ids: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let _ = Self::finished(&emitter, EXIT_SUCCESS, 1).await;
    }

    async fn get_update_detail(
        &self,
        ids: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let none = Vec::<String>::new();
        for id in &ids {
            let detail = (
                id,
                &none,
                &none,
                &none,
                &none,
                &none,
                RESTART_NONE,
                "",
                "",
                0_u32,
                "",
                "",
            );
            let _ = emitter.emit(TRANSACTION, "UpdateDetail", &detail).await;
        }
        let _ = Self::finished(&emitter, EXIT_SUCCESS, 1).await;
    }

    async fn update_packages(
        &self,
        flags: u64,
        ids: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        record(&self.0, "UpdatePackages", flags, ids);
        let pulled_in = lock(&self.0).pulled_in.clone();
        if flags & SIMULATE != 0 {
            for id in &pulled_in {
                let _ = Self::package(&emitter, INFO_INSTALLING, id, "").await;
            }
        }
        let _ = Self::finished(&emitter, EXIT_SUCCESS, 1).await;
    }

    #[zbus(signal)]
    async fn package(
        emitter: &SignalEmitter<'_>,
        info: u32,
        id: &str,
        summary: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn finished(emitter: &SignalEmitter<'_>, exit: u32, runtime: u32) -> zbus::Result<()>;
}

pub struct FakePackageKit {
    _daemon: Connection,
    world: Shared,
}

impl FakePackageKit {
    pub fn start(bus: &SessionBus, updates: &[&str]) -> zbus::Result<Self> {
        let world = Arc::new(Mutex::new(World {
            updates: updates.iter().map(|id| (*id).to_owned()).collect(),
            ..World::default()
        }));
        let daemon = zbus::blocking::connection::Builder::address(bus.address())?
            .name("org.freedesktop.PackageKit")?
            .serve_at(
                "/org/freedesktop/PackageKit",
                PackageKit(Arc::clone(&world)),
            )?
            .serve_at("/org/freedesktop/PackageKit", Offline(Arc::clone(&world)))?
            .build()?;
        Ok(Self {
            _daemon: daemon,
            world,
        })
    }

    pub fn pull_in(&self, id: &str) {
        lock(&self.world).pulled_in.push(id.to_owned());
    }

    pub fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut lock(&self.world).calls)
    }
}
