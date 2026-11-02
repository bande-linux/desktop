#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::mpsc;
use std::time::Duration;

use bande_test_bus::SessionBus;
use bande_test_bus::drives::FakeUdisks;

const DRIVE: &str = "/org/freedesktop/UDisks2/drives/QEMU_USB";
const ISO: &str = "/org/freedesktop/UDisks2/block_devices/sdb1";
const ESP: &str = "/org/freedesktop/UDisks2/block_devices/sdb2";

#[test]
fn ejecting_a_removable_usb_iso_unmounts_all_its_volumes_and_ejects_without_powering_off_the_port()
{
    let bus = SessionBus::start().unwrap();
    unsafe { std::env::set_var("DBUS_SYSTEM_BUS_ADDRESS", bus.address()) };
    let udisks = FakeUdisks::start(&bus).unwrap();
    let volumes = [(ISO, "/media/anna/Debian"), (ESP, "/media/anna/ESP")];
    let calls = udisks.plug_in_drive(DRIVE, true, &volumes).unwrap();

    let (failed, failures) = mpsc::channel();
    bande_volumes::eject(vec!["/media/anna/Debian".into()], false, move |failure| {
        let _ = failed.send(failure);
    });
    let seen: Vec<String> = calls.iter().take(3).collect();
    assert_eq!(
        seen,
        [
            format!("Unmount {ISO}"),
            format!("Unmount {ESP}"),
            "Eject".to_owned()
        ]
    );
    assert!(calls.recv_timeout(Duration::from_millis(300)).is_err());
    assert!(failures.try_recv().is_err());
}
