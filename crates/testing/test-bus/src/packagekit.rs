use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use zbus::blocking::Connection;
use zbus::object_server::{ObjectServer, SignalEmitter};
use zbus::zvariant::{OwnedObjectPath, Value};

use crate::SessionBus;

pub const ONLY_TRUSTED: u64 = 1 << 1;
pub const SIMULATE: u64 = 1 << 2;
pub const ONLY_DOWNLOAD: u64 = 1 << 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakePackage {
    pub info: u32,
    pub id: String,
    pub summary: String,
    pub size: u64,
}

impl FakePackage {
    pub fn new(info: u32, id: &str, summary: &str, size: u64) -> Self {
        Self {
            info,
            id: id.to_owned(),
            summary: summary.to_owned(),
            size,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Script {
    pub updates: Vec<FakePackage>,
    pub installed: Vec<FakePackage>,
    pub changes: Vec<FakePackage>,
    pub local: Option<FakePackage>,
    pub files: Vec<String>,
    pub failures: HashMap<String, (u32, String)>,
    pub hang: bool,
    pub restart: bool,
    pub since_refresh: u32,
    pub prepared: Vec<String>,
    pub triggered: bool,
    pub results: Option<(bool, Vec<String>, u32, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub method: String,
    pub flags: u64,
    pub args: Vec<String>,
}

#[derive(Default)]
struct World {
    script: Script,
    calls: Vec<Call>,
    transactions: u32,
}

type Shared = Arc<Mutex<World>>;

fn lock(world: &Shared) -> MutexGuard<'_, World> {
    world.lock().unwrap_or_else(PoisonError::into_inner)
}

fn failed(error: impl std::fmt::Display) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(error.to_string())
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
        let path = OwnedObjectPath::try_from(format!("/org/freedesktop/PackageKit/{number}"))
            .map_err(failed)?;
        let transaction = Transaction {
            world: Arc::clone(&self.0),
            percentage: 101,
        };
        server.at(&path, transaction).await.map_err(failed)?;
        Ok(path)
    }

    fn get_time_since_action(&self, _role: u32) -> u32 {
        lock(&self.0).script.since_refresh
    }
}

struct Offline(Shared);

#[zbus::interface(name = "org.freedesktop.PackageKit.Offline")]
impl Offline {
    fn get_prepared(&self) -> Vec<String> {
        lock(&self.0).script.prepared.clone()
    }

    fn get_results(&self) -> zbus::fdo::Result<(bool, Vec<String>, u32, u64, u32, String)> {
        let results = lock(&self.0).script.results.clone();
        let (success, ids, code, details) =
            results.ok_or_else(|| failed("no update results available"))?;
        Ok((success, ids, 0, 0, code, details))
    }

    fn clear_results(&self) {
        let mut world = lock(&self.0);
        world.script.results = None;
        world.calls.push(Call {
            method: "ClearResults".to_owned(),
            flags: 0,
            args: Vec::new(),
        });
    }

    fn trigger(&self, action: String) {
        let mut world = lock(&self.0);
        world.script.triggered = true;
        world.calls.push(Call {
            method: "Trigger".to_owned(),
            flags: 0,
            args: vec![action],
        });
    }

    #[zbus(property)]
    fn update_triggered(&self) -> bool {
        lock(&self.0).script.triggered
    }
}

struct Transaction {
    world: Shared,
    percentage: u32,
}

impl Transaction {
    fn record(&self, method: &str, flags: u64, args: &[String]) -> Script {
        let mut world = lock(&self.world);
        world.calls.push(Call {
            method: method.to_owned(),
            flags,
            args: args.to_vec(),
        });
        world.script.clone()
    }

    async fn finish(
        emitter: &SignalEmitter<'_>,
        script: &Script,
        method: &str,
        packages: &[FakePackage],
    ) -> zbus::Result<()> {
        if let Some((code, details)) = script.failures.get(method) {
            Self::error_code(emitter, *code, details).await?;
            return Self::finished(emitter, 2, 10).await;
        }
        for package in packages {
            Self::package(emitter, package.info, &package.id, &package.summary).await?;
        }
        Self::finished(emitter, 1, 10).await
    }

    async fn work(
        &mut self,
        emitter: &SignalEmitter<'_>,
        script: &Script,
        method: &str,
        packages: &[FakePackage],
    ) -> zbus::Result<()> {
        self.percentage = 50;
        self.percentage_changed(emitter).await?;
        if script.hang {
            return Ok(());
        }
        if script.restart && !script.failures.contains_key(method) {
            Self::require_restart(emitter, 4, "").await?;
        }
        Self::finish(emitter, script, method, packages).await
    }
}

#[zbus::interface(name = "org.freedesktop.PackageKit.Transaction")]
impl Transaction {
    fn set_hints(&self, hints: Vec<String>) {
        self.record("SetHints", 0, &hints);
    }

    async fn cancel(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        self.record("Cancel", 0, &[]);
        let _ = Self::finished(&emitter, 3, 10).await;
    }

    async fn refresh_cache(&self, force: bool, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let script = self.record("RefreshCache", 0, &[force.to_string()]);
        lock(&self.world).script.since_refresh = 0;
        let _ = Self::finish(&emitter, &script, "RefreshCache", &[]).await;
    }

    async fn get_updates(&self, filter: u64, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let script = self.record("GetUpdates", filter, &[]);
        let _ = Self::finish(&emitter, &script, "GetUpdates", &script.updates).await;
    }

    async fn resolve(
        &self,
        filter: u64,
        names: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let script = self.record("Resolve", filter, &names);
        let found: Vec<FakePackage> = script
            .installed
            .iter()
            .filter(|package| {
                names
                    .iter()
                    .any(|name| package.id.split(';').next() == Some(name.as_str()))
            })
            .cloned()
            .collect();
        let _ = Self::finish(&emitter, &script, "Resolve", &found).await;
    }

    async fn get_details(
        &self,
        ids: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let script = self.record("GetDetails", 0, &ids);
        for package in script
            .updates
            .iter()
            .filter(|package| ids.contains(&package.id))
        {
            let _ = Self::details(&emitter, details(package)).await;
        }
        let _ = Self::finish(&emitter, &script, "GetDetails", &[]).await;
    }

    async fn get_details_local(
        &self,
        files: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let script = self.record("GetDetailsLocal", 0, &files);
        if let Some(package) = &script.local {
            let _ = Self::details(&emitter, details(package)).await;
        }
        let _ = Self::finish(&emitter, &script, "GetDetailsLocal", &[]).await;
    }

    async fn get_files_local(
        &self,
        files: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let script = self.record("GetFilesLocal", 0, &files);
        let id = script
            .local
            .as_ref()
            .map(|package| package.id.clone())
            .unwrap_or_default();
        let _ = Self::files(&emitter, &id, script.files.clone()).await;
        let _ = Self::finish(&emitter, &script, "GetFilesLocal", &[]).await;
    }

    async fn update_packages(
        &mut self,
        flags: u64,
        ids: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let script = self.record("UpdatePackages", flags, &ids);
        if flags & ONLY_DOWNLOAD != 0 && !script.failures.contains_key("UpdatePackages") {
            lock(&self.world).script.prepared.clone_from(&ids);
        }
        let _ = self.work(&emitter, &script, "UpdatePackages", &[]).await;
    }

    async fn install_files(
        &mut self,
        flags: u64,
        files: Vec<String>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let script = self.record("InstallFiles", flags, &files);
        let _ = self
            .work(&emitter, &script, "InstallFiles", &script.changes)
            .await;
    }

    async fn remove_packages(
        &mut self,
        flags: u64,
        ids: Vec<String>,
        _allow_deps: bool,
        _autoremove: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let script = self.record("RemovePackages", flags, &ids);
        let _ = self
            .work(&emitter, &script, "RemovePackages", &script.changes)
            .await;
    }

    #[zbus(property)]
    fn percentage(&self) -> u32 {
        self.percentage
    }

    #[zbus(signal)]
    async fn package(
        emitter: &SignalEmitter<'_>,
        info: u32,
        id: &str,
        summary: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn details(
        emitter: &SignalEmitter<'_>,
        data: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn files(emitter: &SignalEmitter<'_>, id: &str, files: Vec<String>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn require_restart(emitter: &SignalEmitter<'_>, kind: u32, id: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn error_code(emitter: &SignalEmitter<'_>, code: u32, details: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn finished(emitter: &SignalEmitter<'_>, exit: u32, runtime: u32) -> zbus::Result<()>;
}

fn details(package: &FakePackage) -> HashMap<&str, Value<'_>> {
    HashMap::from([
        ("package-id", Value::from(package.id.as_str())),
        ("summary", Value::from(package.summary.as_str())),
        ("download-size", Value::from(package.size)),
    ])
}

pub struct FakePackageKit {
    _daemon: Connection,
    world: Shared,
}

impl FakePackageKit {
    pub fn start(bus: &SessionBus, script: Script) -> zbus::Result<Self> {
        let world = Arc::new(Mutex::new(World {
            script,
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

    pub fn calls(&self) -> Vec<Call> {
        lock(&self.world).calls.clone()
    }

    pub fn called(&self, method: &str) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|call| call.method == method)
            .collect()
    }

    pub fn script(&self, change: impl FnOnce(&mut Script)) {
        change(&mut lock(&self.world).script);
    }
}
