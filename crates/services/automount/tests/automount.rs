#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use bande_automount::Automounter;
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
