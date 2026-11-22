#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use bande_automount::Automounter;
use bande_automount::discovery::{Nearby, computers};
use bande_discovery::nearby::Service;
use bande_test_bus::SessionBus;
use bande_test_bus::drives::{FakeLogind, FakeUdisks};

const USB: &str = "/org/freedesktop/UDisks2/block_devices/sdb1";

#[test]
fn a_drive_plugged_in_while_the_session_is_locked_is_mounted_only_after_unlocking() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let logind = FakeLogind::start(&bus).expect("logind");
    let udisks = FakeUdisks::start(&bus).expect("udisks");
    let connection = bus.connect().expect("automount connection");
    let _automounter = Automounter::start(&connection, &connection).expect("automounter");

    logind.set_locked(true).expect("lock");
    std::thread::sleep(Duration::from_millis(300));
    let mounts = udisks.plug_in(USB).expect("plug in");
    assert!(mounts.recv_timeout(Duration::from_millis(300)).is_err());

    logind.set_locked(false).expect("unlock");
    let mounted = mounts.recv_timeout(Duration::from_secs(5));
    assert_eq!(mounted.as_deref(), Ok(USB));
}

fn service(name: &str, kind: &str, host: &str, port: u16) -> Service {
    Service {
        name: name.to_owned(),
        kind: kind.to_owned(),
        host: host.to_owned(),
        address: String::new(),
        port,
        text: Vec::new(),
    }
}

#[test]
fn a_computer_sharing_files_and_its_screen_is_listed_once_with_its_screen_beside_it() {
    let found = computers(vec![
        service("Studio Screen", "_rfb._tcp", "studio.local", 5900),
        service("Studio", "_sftp-ssh._tcp", "Studio.local", 22),
        service("Studio", "_smb._tcp", "studio.local", 445),
        service("Kiosk", "_rfb._tcp", "kiosk.local", 5901),
    ]);
    assert_eq!(
        found,
        vec![
            Nearby {
                name: "Kiosk".to_owned(),
                url: "vnc://kiosk.local:5901".to_owned(),
                screen: None,
            },
            Nearby {
                name: "Studio".to_owned(),
                url: "smb://studio.local".to_owned(),
                screen: Some("vnc://studio.local".to_owned()),
            },
        ]
    );
}
