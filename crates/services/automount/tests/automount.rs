#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use unbound_automount::{Automounter, UDISKS, UDISKS_PATH};
use unbound_notify::{Notification, NotificationServer};
use unbound_test_bus::SessionBus;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(300);
const USB: &str = "/org/freedesktop/UDisks2/block_devices/sdb1";

#[derive(Clone)]
struct Hints {
    auto: bool,
    system: bool,
    label: &'static str,
}

struct FakeBlock {
    hints: Hints,
}

#[zbus::interface(name = "org.freedesktop.UDisks2.Block")]
impl FakeBlock {
    #[zbus(property)]
    fn hint_auto(&self) -> bool {
        self.hints.auto
    }

    #[zbus(property)]
    fn hint_ignore(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn hint_system(&self) -> bool {
        self.hints.system
    }

    #[zbus(property)]
    fn id_label(&self) -> String {
        self.hints.label.to_owned()
    }

    #[zbus(property)]
    fn hint_name(&self) -> String {
        String::new()
    }
}

struct FakeFilesystem {
    mount_points: Vec<Vec<u8>>,
    mounts: Arc<Mutex<Sender<String>>>,
}

#[zbus::interface(name = "org.freedesktop.UDisks2.Filesystem")]
impl FakeFilesystem {
    async fn mount(
        &mut self,
        _options: std::collections::HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> String {
        self.mount_points = vec![b"/media/anna/USB\0".to_vec()];
        if let Ok(mounts) = self.mounts.lock() {
            let _ = mounts.send(emitter.path().to_string());
        }
        let _ = self.mount_points_changed(&emitter).await;
        "/media/anna/USB".to_owned()
    }

    async fn unmount(
        &mut self,
        _options: std::collections::HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) {
        self.mount_points.clear();
        let _ = self.mount_points_changed(&emitter).await;
    }

    #[zbus(property)]
    fn mount_points(&self) -> Vec<Vec<u8>> {
        self.mount_points.clone()
    }
}

struct Fixture {
    udisks: Connection,
    mounts: Receiver<String>,
    mounts_sender: Arc<Mutex<Sender<String>>>,
    notifications: NotificationServer,
    _automounter: Arc<Automounter>,
    _bus: SessionBus,
}

fn plug(udisks: &Connection, path: &str, hints: Hints, mounts: &Arc<Mutex<Sender<String>>>) {
    udisks
        .object_server()
        .at(path, FakeBlock { hints })
        .expect("block");
    udisks
        .object_server()
        .at(
            path,
            FakeFilesystem {
                mount_points: Vec::new(),
                mounts: Arc::clone(mounts),
            },
        )
        .expect("filesystem");
}

fn unplug(udisks: &Connection, path: &str) {
    udisks
        .object_server()
        .remove::<FakeFilesystem, _>(path)
        .expect("remove filesystem");
    udisks
        .object_server()
        .remove::<FakeBlock, _>(path)
        .expect("remove block");
}

fn usb() -> Hints {
    Hints {
        auto: true,
        system: false,
        label: "USB",
    }
}

fn start(already_plugged: bool) -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let notifications = NotificationServer::start(&bus.connect().expect("notify connection"))
        .expect("notification server");
    let udisks = bus.connect().expect("udisks connection");
    udisks
        .object_server()
        .at(UDISKS_PATH, zbus::fdo::ObjectManager)
        .expect("object manager");
    let (sender, mounts) = mpsc::channel();
    let mounts_sender = Arc::new(Mutex::new(sender));
    if already_plugged {
        plug(&udisks, USB, usb(), &mounts_sender);
    }
    udisks.request_name(UDISKS).expect("udisks name");
    let connection = bus.connect().expect("automount connection");
    let automounter = Automounter::start(&connection, &connection).expect("automounter");
    Fixture {
        udisks,
        mounts,
        mounts_sender,
        notifications,
        _automounter: automounter,
        _bus: bus,
    }
}

impl Fixture {
    fn wait_for(&self, done: impl Fn(&[Notification]) -> bool) -> Vec<Notification> {
        let deadline = Instant::now() + WAIT;
        loop {
            let active = self.notifications.active();
            if done(&active) {
                return active;
            }
            assert!(Instant::now() < deadline, "still showing {active:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn unmount(&self, path: &str) {
        let client = self.udisks.clone();
        client
            .call_method(
                Some(UDISKS),
                path,
                Some("org.freedesktop.UDisks2.Filesystem"),
                "Unmount",
                &(std::collections::HashMap::<&str, OwnedValue>::new(),),
            )
            .expect("Unmount");
    }
}

#[test]
fn a_drive_plugged_in_is_mounted() {
    let fixture = start(false);
    plug(&fixture.udisks, USB, usb(), &fixture.mounts_sender);
    assert_eq!(fixture.mounts.recv_timeout(WAIT).expect("mounted"), USB);
}

#[test]
fn a_drive_present_at_login_is_mounted() {
    let fixture = start(true);
    assert_eq!(fixture.mounts.recv_timeout(WAIT).expect("mounted"), USB);
}

#[test]
fn system_and_non_auto_volumes_are_left_alone() {
    let fixture = start(false);
    plug(
        &fixture.udisks,
        "/org/freedesktop/UDisks2/block_devices/nvme0n1p2",
        Hints {
            auto: true,
            system: true,
            label: "root",
        },
        &fixture.mounts_sender,
    );
    plug(
        &fixture.udisks,
        "/org/freedesktop/UDisks2/block_devices/sdc1",
        Hints {
            auto: false,
            system: false,
            label: "SPARE",
        },
        &fixture.mounts_sender,
    );
    assert!(fixture.mounts.recv_timeout(QUIET).is_err());
}

#[test]
fn pulling_a_mounted_drive_warns_that_it_was_not_ejected() {
    let fixture = start(false);
    plug(&fixture.udisks, USB, usb(), &fixture.mounts_sender);
    fixture.mounts.recv_timeout(WAIT).expect("mounted");
    std::thread::sleep(QUIET);

    unplug(&fixture.udisks, USB);
    let shown = fixture.wait_for(|active| active.len() == 1);
    assert_eq!(shown[0].summary, "Disk Not Ejected Properly");
    assert!(
        shown[0].body.contains("\u{201c}USB\u{201d}"),
        "{}",
        shown[0].body
    );
}

#[test]
fn an_ejected_drive_leaves_quietly() {
    let fixture = start(false);
    plug(&fixture.udisks, USB, usb(), &fixture.mounts_sender);
    fixture.mounts.recv_timeout(WAIT).expect("mounted");
    std::thread::sleep(QUIET);
    fixture.unmount(USB);
    std::thread::sleep(QUIET);

    unplug(&fixture.udisks, USB);
    std::thread::sleep(QUIET);
    assert!(fixture.notifications.active().is_empty());
}

#[test]
fn a_drive_pulled_right_after_ejecting_leaves_quietly() {
    let fixture = start(false);
    for _ in 0..10 {
        plug(&fixture.udisks, USB, usb(), &fixture.mounts_sender);
        fixture.mounts.recv_timeout(WAIT).expect("mounted");
        fixture.unmount(USB);
        unplug(&fixture.udisks, USB);
    }
    std::thread::sleep(QUIET);
    assert!(fixture.notifications.active().is_empty());
}
