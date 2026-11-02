use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use zbus::blocking::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::SessionBus;

const LOGIN1: &str = "org.freedesktop.login1";
const SESSION: &str = "org.freedesktop.login1.Session";
const SESSION_PATH: &str = "/org/freedesktop/login1/session/_31";

struct Manager;

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl Manager {
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
}

impl FakeLogind {
    pub fn start(bus: &SessionBus) -> zbus::Result<Self> {
        let session = Session {
            locked: AtomicBool::new(false),
        };
        let daemon = zbus::blocking::connection::Builder::address(bus.address())?
            .name(LOGIN1)?
            .serve_at("/org/freedesktop/login1", Manager)?
            .serve_at(SESSION_PATH, session)?
            .build()?;
        Ok(Self {
            _daemon: daemon,
            caller: bus.connect()?,
        })
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

struct Block;

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
}

struct Filesystem {
    mounts: Mutex<Sender<String>>,
}

#[zbus::interface(name = "org.freedesktop.UDisks2.Filesystem")]
impl Filesystem {
    fn mount(
        &self,
        _options: HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> String {
        if let Ok(mounts) = self.mounts.lock() {
            let _ = mounts.send(emitter.path().to_string());
        }
        "/media/anna/USB".to_owned()
    }

    #[zbus(property)]
    fn mount_points(&self) -> Vec<Vec<u8>> {
        Vec::new()
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
        let (sender, mounts) = mpsc::channel();
        let server = self.daemon.object_server();
        server.at(block, Block)?;
        server.at(
            block,
            Filesystem {
                mounts: Mutex::new(sender),
            },
        )?;
        Ok(mounts)
    }
}
