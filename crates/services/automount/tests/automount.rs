#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use bande_automount::{Automounter, LOGIN1, UDISKS, UDISKS_PATH};
use bande_test_bus::SessionBus;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SESSION_PATH: &str = "/org/freedesktop/login1/session/_31";
const USB: &str = "/org/freedesktop/UDisks2/block_devices/sdb1";

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

#[test]
fn a_drive_plugged_in_while_the_session_is_locked_is_mounted_only_after_unlocking() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let logind = bus.connect().expect("logind connection");
    logind
        .object_server()
        .at("/org/freedesktop/login1", Manager)
        .expect("manager");
    logind
        .object_server()
        .at(
            SESSION_PATH,
            Session {
                locked: AtomicBool::new(false),
            },
        )
        .expect("session");
    logind.request_name(LOGIN1).expect("logind name");
    let udisks = bus.connect().expect("udisks connection");
    udisks
        .object_server()
        .at(UDISKS_PATH, zbus::fdo::ObjectManager)
        .expect("object manager");
    udisks.request_name(UDISKS).expect("udisks name");
    let connection = bus.connect().expect("automount connection");
    let _automounter = Automounter::start(&connection, &connection).expect("automounter");
    let set_locked = |locked: bool| {
        udisks
            .call_method(
                Some(LOGIN1),
                SESSION_PATH,
                Some("org.freedesktop.login1.Session"),
                "SetLockedHint",
                &(locked,),
            )
            .expect("SetLockedHint");
    };

    set_locked(true);
    std::thread::sleep(Duration::from_millis(300));
    let (sender, mounts) = mpsc::channel();
    udisks.object_server().at(USB, Block).expect("block");
    udisks
        .object_server()
        .at(
            USB,
            Filesystem {
                mounts: Mutex::new(sender),
            },
        )
        .expect("filesystem");
    assert!(mounts.recv_timeout(Duration::from_millis(300)).is_err());

    set_locked(false);
    assert_eq!(
        mounts
            .recv_timeout(Duration::from_secs(5))
            .expect("mounted"),
        USB
    );
}
