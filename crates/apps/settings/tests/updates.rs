#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_packages::{Class, Client};
use bande_settings::{Reboot, Refresh, SoftwareState, check, update};
use bande_test_bus::SessionBus;
use bande_test_bus::fwupd::{self, Device, FakeFwupd, METADATA, SIGNATURE, SYSTEM_POWER_TOO_LOW};
use bande_test_bus::packagekit::{Call, FakePackageKit, ONLY_DOWNLOAD, ONLY_TRUSTED, SIMULATE};

const LIBC: &str = "libc6;2.42-1;arm64;debian";
const ASPELL: &str = "aspell;0.60.8.2-5;arm64;debian";
const SYSTEMD: &str = "systemd;259-1;arm64;debian";
const DOCK: Device = Device {
    id: "dock",
    name: "Desk Dock",
    from: "1.0",
    to: "1.1",
    at_restart: false,
    problems: 0,
};
const UEFI: Device = Device {
    id: "uefi",
    name: "System Firmware",
    from: "0.9",
    to: "1.2",
    at_restart: true,
    problems: 0,
};

#[test]
fn updates_install_now_unless_they_need_the_system_restarted() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let packagekit = FakePackageKit::start(&bus, &[LIBC, ASPELL]).expect("fake PackageKit");
    let system = bus.connect().expect("client connection");
    let client = Client::new(&system);
    let SoftwareState::Available(packages) = check(&system, &client, Refresh::WhenStale) else {
        panic!("no updates found");
    };

    let state = update(&system, &client, &packages, &|_| {});
    assert_eq!(
        state,
        SoftwareState::Installed {
            count: 1,
            reboot: Some(Reboot::Install {
                count: 1,
                firmware: false,
            }),
            logout: false,
            apps: Vec::new(),
        }
    );
    assert_eq!(
        packagekit.take_calls(),
        [
            Call::update(ONLY_TRUSTED | SIMULATE, &[ASPELL]),
            Call::update(ONLY_TRUSTED, &[ASPELL]),
            Call::update(ONLY_TRUSTED | ONLY_DOWNLOAD, &[LIBC]),
            Call::reboot(),
        ]
    );

    packagekit.pull_in(SYSTEMD);
    let state = update(&system, &client, &packages, &|_| {});
    assert_eq!(
        state,
        SoftwareState::Prepared {
            count: 2,
            firmware: false
        }
    );
    assert_eq!(
        packagekit.take_calls(),
        [
            Call::update(ONLY_TRUSTED | SIMULATE, &[ASPELL]),
            Call::update(ONLY_TRUSTED | ONLY_DOWNLOAD, &[ASPELL, LIBC]),
            Call::reboot(),
        ]
    );
}

#[test]
fn firmware_lists_beside_packages_and_capsules_wait_for_the_restart() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let _packagekit = FakePackageKit::start(&bus, &[LIBC]).expect("fake PackageKit");
    let fwupd = FakeFwupd::start(&bus, &[UEFI, DOCK]).expect("fake fwupd");
    let system = bus.connect().expect("client connection");
    let client = Client::new(&system);

    let SoftwareState::Available(listed) = check(&system, &client, Refresh::WhenStale) else {
        panic!("no updates found");
    };
    assert!(fwupd.take_calls().is_empty(), "fetched without a check");
    let names: Vec<&str> = listed.iter().map(|update| update.name.as_str()).collect();
    assert_eq!(names, ["Desk Dock", "System Firmware", "libc6"]);
    assert_eq!(
        (&listed[1].from, &listed[1].version, &listed[1].summary),
        (
            &"0.9".to_owned(),
            &"1.2".to_owned(),
            &"Fixes waking from sleep.\n• Quieter fan".to_owned()
        )
    );
    assert_eq!(listed[1].class, Class::Firmware { at_restart: true });

    check(&system, &client, Refresh::Always);
    assert_eq!(
        fwupd.take_calls(),
        [fwupd::Call::UpdateMetadata(
            "lvfs".to_owned(),
            METADATA.to_vec(),
            SIGNATURE.to_vec()
        )]
    );

    let firmware: Vec<_> = listed
        .into_iter()
        .filter(|update| update.name != "libc6")
        .collect();
    let state = update(&system, &client, &firmware, &|_| {});
    assert_eq!(
        state,
        SoftwareState::Installed {
            count: 1,
            reboot: Some(Reboot::Install {
                count: 1,
                firmware: true
            }),
            logout: false,
            apps: Vec::new(),
        }
    );
    assert_eq!(
        fwupd.take_calls(),
        [
            fwupd::Call::Install("dock".to_owned(), DOCK.cab()),
            fwupd::Call::Install("uefi".to_owned(), UEFI.cab()),
        ]
    );
}

#[test]
fn firmware_is_refused_while_fwupd_reports_the_battery_too_low() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let _packagekit = FakePackageKit::start(&bus, &[]).expect("fake PackageKit");
    let low = Device {
        problems: SYSTEM_POWER_TOO_LOW,
        ..UEFI
    };
    let fwupd = FakeFwupd::start(&bus, &[low]).expect("fake fwupd");
    let system = bus.connect().expect("client connection");
    let client = Client::new(&system);
    let SoftwareState::Available(listed) = check(&system, &client, Refresh::Never) else {
        panic!("no firmware update found");
    };

    let state = update(&system, &client, &listed, &|_| {});
    let SoftwareState::Failed { reason, .. } = state else {
        panic!("installed on a low battery: {state:?}");
    };
    assert!(reason.contains("battery"), "{reason}");
    assert!(fwupd.take_calls().is_empty());
}
