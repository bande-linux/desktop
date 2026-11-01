#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_packages::Client;
use bande_settings::{Reboot, Refresh, SoftwareState, check, update};
use bande_test_bus::SessionBus;
use bande_test_bus::packagekit::{Call, FakePackageKit, ONLY_DOWNLOAD, ONLY_TRUSTED, SIMULATE};

const LIBC: &str = "libc6;2.42-1;arm64;debian";
const ASPELL: &str = "aspell;0.60.8.2-5;arm64;debian";
const SYSTEMD: &str = "systemd;259-1;arm64;debian";

fn updated(flags: u64, ids: &[&str]) -> Call {
    Call {
        method: "UpdatePackages".to_owned(),
        flags,
        args: ids.iter().map(|id| (*id).to_owned()).collect(),
    }
}

fn triggered() -> Call {
    Call {
        method: "Trigger".to_owned(),
        flags: 0,
        args: vec!["reboot".to_owned()],
    }
}

#[test]
fn updates_install_now_unless_they_need_the_system_restarted() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let packagekit = FakePackageKit::start(&bus, &[LIBC, ASPELL]).expect("fake PackageKit");
    let system = bus.connect().expect("client connection");
    let client = Client::new(&system);
    let SoftwareState::Available(packages) = check(&client, Refresh::WhenStale) else {
        panic!("no updates found");
    };

    let state = update(&system, &client, &packages, true, &|_| {});
    assert_eq!(
        state,
        SoftwareState::Installed {
            count: 1,
            reboot: Some(Reboot::Install(1)),
            logout: false,
            apps: Vec::new(),
        }
    );
    assert_eq!(
        packagekit.take_calls(),
        [
            updated(ONLY_TRUSTED | SIMULATE, &[ASPELL]),
            updated(ONLY_TRUSTED, &[ASPELL]),
            updated(ONLY_TRUSTED | ONLY_DOWNLOAD, &[LIBC]),
            triggered(),
        ]
    );

    packagekit.pull_in(SYSTEMD);
    let state = update(&system, &client, &packages, true, &|_| {});
    assert_eq!(state, SoftwareState::Prepared(2));
    assert_eq!(
        packagekit.take_calls(),
        [
            updated(ONLY_TRUSTED | SIMULATE, &[ASPELL]),
            updated(ONLY_TRUSTED | ONLY_DOWNLOAD, &[ASPELL, LIBC]),
            triggered(),
        ]
    );
}
