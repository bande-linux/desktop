#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bande_packages::offline::{self, Results};
use bande_packages::{Client, Error, ErrorCode, Failure, Info, Mode, available};
use bande_test_bus::SessionBus;
use bande_test_bus::packagekit::{
    FakePackage, FakePackageKit, ONLY_DOWNLOAD, ONLY_TRUSTED, SIMULATE, Script,
};

const WAIT: Duration = Duration::from_secs(10);
const SECURITY_WITH_SEVERITY: u32 = 8 | (8 << 16);

fn updates() -> Vec<FakePackage> {
    vec![
        FakePackage::new(
            SECURITY_WITH_SEVERITY,
            "libnss3;2:3.110-1+deb13u5;arm64;debian-stable-security-main",
            "Network Security Service libraries",
            1_400_000,
        ),
        FakePackage::new(9, "held;2.0;arm64;debian", "Held back", 10),
        FakePackage::new(
            5,
            "firefox-esr;128.4.0;arm64;debian",
            "Web browser",
            70_000_000,
        ),
    ]
}

fn start(script: Script) -> (SessionBus, FakePackageKit, zbus::blocking::Connection) {
    let bus = SessionBus::start().expect("dbus-daemon");
    let fake = FakePackageKit::start(&bus, script).expect("fake PackageKit");
    let system = bus.connect().expect("connection");
    (bus, fake, system)
}

#[test]
fn updates_drop_blocked_packages_and_read_the_info_under_the_severity_bits() {
    let (_bus, _fake, system) = start(Script {
        updates: updates(),
        ..Script::default()
    });
    let client = Client::new(&system);
    let found = client.updates().expect("updates");
    let names: Vec<(&str, Info)> = found
        .iter()
        .map(|package| (package.name.as_str(), package.info))
        .collect();
    assert_eq!(
        names,
        [("libnss3", Info::Security), ("firefox-esr", Info::Other(5))]
    );
    assert_eq!(found[0].version, "2:3.110-1+deb13u5");
    assert_eq!(found[0].origin, "debian-stable-security-main");
    let ids: Vec<String> = found.iter().map(|package| package.id.clone()).collect();
    let sizes: Vec<u64> = client
        .details(&ids)
        .expect("details")
        .iter()
        .map(|details| details.size)
        .collect();
    assert_eq!(sizes, [1_400_000, 70_000_000]);
}

#[test]
fn an_error_code_keeps_its_kind_and_text() {
    let (_bus, _fake, system) = start(Script {
        failures: HashMap::from([
            ("RefreshCache".to_owned(), (2, "offline".to_owned())),
            ("InstallFiles".to_owned(), (13, "Depends: nope".to_owned())),
        ]),
        ..Script::default()
    });
    let client = Client::new(&system);
    assert!(matches!(
        client.refresh(true),
        Err(Error::Failed(Failure { code: ErrorCode::NoNetwork, details })) if details == "offline"
    ));
    assert!(matches!(
        client.install_files(Path::new("/tmp/x.deb"), Mode::Simulate, &|_| {}),
        Err(Error::Failed(Failure {
            code: ErrorCode::Dependencies,
            ..
        }))
    ));
}

#[test]
fn packagekit_leaving_mid_transaction_ends_as_vanished() {
    let (_bus, fake, system) = start(Script {
        hang: true,
        ..Script::default()
    });
    let started = Instant::now();
    let client = Client::new(&system);
    let download =
        std::thread::spawn(move || client.download(&["a;1;arm64;x".to_owned()], &|_| {}));
    let deadline = Instant::now() + WAIT;
    while fake.called("UpdatePackages").is_empty() {
        assert!(Instant::now() < deadline, "the download never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(fake);
    assert!(matches!(
        download.join().expect("join"),
        Err(Error::Vanished)
    ));
    assert!(started.elapsed() < WAIT);
}

#[test]
fn without_packagekit_the_client_says_it_is_missing() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let system = bus.connect().expect("connection");
    assert!(!available(&system).expect("bus"));
    assert!(matches!(
        Client::new(&system).updates(),
        Err(Error::Missing)
    ));
    assert!(matches!(offline::prepared(&system), Err(Error::Missing)));
}

#[test]
fn download_only_fetches_trusted_packages_and_reports_progress() {
    let (_bus, fake, system) = start(Script::default());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    let ids = ["libnss3;2:3.110-1+deb13u5;arm64;debian".to_owned()];
    Client::new(&system)
        .download(&ids, &move |progress| {
            record.lock().expect("seen").push(progress.percent);
        })
        .expect("download");
    let calls = fake.called("UpdatePackages");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].flags, ONLY_TRUSTED | ONLY_DOWNLOAD);
    assert_eq!(*seen.lock().expect("seen"), [Some(50)]);
    assert_eq!(offline::prepared(&system).expect("prepared"), ids);
}

#[test]
fn a_simulated_local_install_lists_each_change_and_never_asks_for_trusted_only() {
    let (_bus, fake, system) = start(Script {
        changes: vec![
            FakePackage::new(12, "tool;1.0;arm64;local", "", 0),
            FakePackage::new(11, "libfoo;2.0;arm64;debian", "", 0),
            FakePackage::new(20, "libbar;0.9;arm64;debian", "", 0),
            FakePackage::new(13, "old;1.0;arm64;installed", "", 0),
        ],
        ..Script::default()
    });
    let client = Client::new(&system);
    let path = Path::new("/home/user/tool.deb");
    let changes = client
        .install_files(path, Mode::Simulate, &|_| {})
        .expect("simulate");
    let infos: Vec<Info> = changes
        .packages
        .iter()
        .map(|package| package.info)
        .collect();
    assert_eq!(
        infos,
        [
            Info::Installing,
            Info::Updating,
            Info::Downgrading,
            Info::Removing
        ]
    );
    client
        .install_files(path, Mode::Apply, &|_| {})
        .expect("apply");
    let flags: Vec<u64> = fake
        .called("InstallFiles")
        .iter()
        .map(|call| call.flags)
        .collect();
    assert_eq!(flags, [SIMULATE, 0]);
    assert_eq!(
        fake.called("InstallFiles")[0].args,
        ["/home/user/tool.deb".to_owned()]
    );
}

#[test]
fn cancel_stops_the_running_transaction() {
    let (_bus, fake, system) = start(Script {
        hang: true,
        ..Script::default()
    });
    let client = Client::new(&system);
    let canceller = client.canceller();
    let install = std::thread::spawn(move || {
        client.install_files(Path::new("/tmp/x.deb"), Mode::Apply, &|_| {})
    });
    let deadline = Instant::now() + WAIT;
    while fake.called("InstallFiles").is_empty() {
        assert!(Instant::now() < deadline, "the install never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    canceller.cancel().expect("cancel");
    assert!(matches!(
        install.join().expect("join"),
        Err(Error::Failed(Failure {
            code: ErrorCode::Cancelled,
            ..
        }))
    ));
    assert_eq!(fake.called("Cancel").len(), 1);
}

#[test]
fn offline_results_read_success_failure_and_nothing() {
    let (_bus, fake, system) = start(Script {
        results: Some((true, vec!["a;1;arm64;x".to_owned()], 0, String::new())),
        ..Script::default()
    });
    assert_eq!(
        offline::results(&system).expect("results"),
        Some(Results::Installed(vec!["a;1;arm64;x".to_owned()]))
    );
    offline::clear_results(&system).expect("clear");
    assert_eq!(offline::results(&system).expect("results"), None);
    fake.script(|script| script.results = Some((false, Vec::new(), 46, "disk full".to_owned())));
    assert_eq!(
        offline::results(&system).expect("results"),
        Some(Results::Failed(Failure {
            code: ErrorCode::NoSpace,
            details: "disk full".to_owned()
        }))
    );
    offline::trigger(&system).expect("trigger");
    assert!(offline::triggered(&system).expect("triggered"));
    assert_eq!(fake.called("Trigger")[0].args, ["reboot".to_owned()]);
}
