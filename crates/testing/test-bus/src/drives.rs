use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use zbus::blocking::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::SessionBus;

const LOGIN1: &str = "org.freedesktop.login1";
const SESSION: &str = "org.freedesktop.login1.Session";
const SESSION_PATH: &str = "/org/freedesktop/login1/session/_31";

type InhibitorRow = (String, String, String, String, u32, u32);

#[derive(Default)]
struct Power {
    ended: Vec<&'static str>,
    inhibitors: Vec<(String, String)>,
}

#[derive(Clone, Default)]
struct Manager(Arc<Mutex<Power>>);

impl Manager {
    fn power(&self) -> MutexGuard<'_, Power> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl Manager {
    fn power_off(&self, _interactive: bool) {
        self.power().ended.push("poweroff");
    }

    fn reboot(&self, _interactive: bool) {
        self.power().ended.push("reboot");
    }

    fn list_inhibitors(&self) -> Vec<InhibitorRow> {
        self.power()
            .inhibitors
            .iter()
            .map(|(who, why)| {
                let what = "shutdown:sleep".to_owned();
                (what, who.clone(), why.clone(), "block".to_owned(), 0, 0)
            })
            .collect()
    }

    fn get_session(&self, _session_id: String) -> zbus::fdo::Result<OwnedObjectPath> {
        OwnedObjectPath::try_from(SESSION_PATH)
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
}

struct Session {
    locked: AtomicBool,
}

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl Session {
    async fn set_locked_hint(
        &self,
        locked: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.locked.store(locked, Ordering::SeqCst);
        self.locked_hint_changed(&emitter).await?;
        Ok(())
    }

    #[zbus(property)]
    fn active(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn locked_hint(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }
}

pub struct FakeLogind {
    _daemon: Connection,
    caller: Connection,
    manager: Manager,
}

impl FakeLogind {
    pub fn start(bus: &SessionBus) -> zbus::Result<Self> {
        let session = Session {
            locked: AtomicBool::new(false),
        };
        let manager = Manager::default();
        let daemon = zbus::blocking::connection::Builder::address(bus.address())?
            .name(LOGIN1)?
            .serve_at("/org/freedesktop/login1", manager.clone())?
            .serve_at(SESSION_PATH, session)?
            .build()?;
        Ok(Self {
            _daemon: daemon,
            caller: bus.connect()?,
            manager,
        })
    }

    pub fn ended(&self) -> Vec<&'static str> {
        self.manager.power().ended.clone()
    }

    pub fn inhibit(&self, who: &str, why: &str) {
        self.manager
            .power()
            .inhibitors
            .push((who.to_owned(), why.to_owned()));
    }

    pub fn set_locked(&self, locked: bool) -> zbus::Result<()> {
        self.caller.call_method(
            Some(LOGIN1),
            SESSION_PATH,
            Some(SESSION),
            "SetLockedHint",
            &(locked,),
        )?;
        Ok(())
    }
}

struct Block {
    drive: OwnedObjectPath,
}

#[zbus::interface(name = "org.freedesktop.UDisks2.Block")]
impl Block {
    #[zbus(property)]
    fn hint_auto(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn hint_ignore(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn hint_system(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn id_label(&self) -> String {
        "USB".to_owned()
    }

    #[zbus(property)]
    fn hint_name(&self) -> String {
        String::new()
    }

    #[zbus(property)]
    fn drive(&self) -> OwnedObjectPath {
        self.drive.clone()
    }
}

struct Filesystem {
    calls: Mutex<Sender<String>>,
    mount_points: Vec<Vec<u8>>,
}

impl Filesystem {
    fn record(&self, call: String) {
        if let Ok(calls) = self.calls.lock() {
            let _ = calls.send(call);
        }
    }
}

#[zbus::interface(name = "org.freedesktop.UDisks2.Filesystem")]
impl Filesystem {
    fn mount(
        &self,
        _options: HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> String {
        self.record(emitter.path().to_string());
        "/media/anna/USB".to_owned()
    }

    fn unmount(
        &self,
        _options: HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        self.record(format!("Unmount {}", emitter.path()));
    }

    #[zbus(property)]
    fn mount_points(&self) -> Vec<Vec<u8>> {
        self.mount_points.clone()
    }
}

struct Drive {
    calls: Mutex<Sender<String>>,
    ejectable: bool,
    can_power_off: bool,
}

#[zbus::interface(name = "org.freedesktop.UDisks2.Drive")]
impl Drive {
    fn eject(&self, _options: HashMap<String, OwnedValue>) {
        if let Ok(calls) = self.calls.lock() {
            let _ = calls.send("Eject".to_owned());
        }
    }

    fn power_off(&self, _options: HashMap<String, OwnedValue>) {
        if let Ok(calls) = self.calls.lock() {
            let _ = calls.send("PowerOff".to_owned());
        }
    }

    #[zbus(property)]
    fn ejectable(&self) -> bool {
        self.ejectable
    }

    #[zbus(property)]
    fn can_power_off(&self) -> bool {
        self.can_power_off
    }
}

pub struct FakeUdisks {
    daemon: Connection,
}

impl FakeUdisks {
    pub fn start(bus: &SessionBus) -> zbus::Result<Self> {
        let daemon = zbus::blocking::connection::Builder::address(bus.address())?
            .name("org.freedesktop.UDisks2")?
            .serve_at("/org/freedesktop/UDisks2", zbus::fdo::ObjectManager)?
            .build()?;
        Ok(Self { daemon })
    }

    pub fn plug_in(&self, block: &str) -> zbus::Result<Receiver<String>> {
        self.plug_in_drive("/", false, &[(block, "")])
    }

    pub fn plug_in_drive(
        &self,
        drive: &str,
        ejectable: bool,
        volumes: &[(&str, &str)],
    ) -> zbus::Result<Receiver<String>> {
        let (sender, calls) = mpsc::channel();
        let server = self.daemon.object_server();
        if drive != "/" {
            let drive_object = Drive {
                calls: Mutex::new(sender.clone()),
                ejectable,
                can_power_off: true,
            };
            server.at(drive, drive_object)?;
        }
        for (block, mount_point) in volumes {
            let drive = OwnedObjectPath::try_from(drive)?;
            server.at(*block, Block { drive })?;
            let mount_points = match mount_point.is_empty() {
                true => Vec::new(),
                false => vec![format!("{mount_point}\0").into_bytes()],
            };
            let filesystem = Filesystem {
                calls: Mutex::new(sender.clone()),
                mount_points,
            };
            server.at(*block, filesystem)?;
        }
        Ok(calls)
    }
}

#[derive(Clone, Default)]
struct SystemdManager(Arc<Mutex<Vec<String>>>);

#[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
impl SystemdManager {
    fn stop_unit(&self, name: String, _mode: String) -> zbus::fdo::Result<OwnedObjectPath> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name);
        OwnedObjectPath::try_from("/org/freedesktop/systemd1/job/1")
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
}

pub struct FakeSystemd {
    _daemon: Connection,
    manager: SystemdManager,
}

impl FakeSystemd {
    pub fn start(bus: &SessionBus) -> zbus::Result<Self> {
        let manager = SystemdManager::default();
        let daemon = zbus::blocking::connection::Builder::address(bus.address())?
            .name("org.freedesktop.systemd1")?
            .serve_at("/org/freedesktop/systemd1", manager.clone())?
            .build()?;
        Ok(Self {
            _daemon: daemon,
            manager,
        })
    }

    pub fn stopped(&self) -> Vec<String> {
        self.manager
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
