use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use unbound_config::{ConfigService, ConfigStore};
use unbound_power::backlight::Backlight;
use unbound_power::controller::{self, IdleController};
use unbound_power::logind::{self, ManagerProxyBlocking};
use unbound_test_bus::SessionBus;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(300);
const MINUTE: Duration = Duration::from_secs(60);
const SESSION_PATH: &str = "/org/freedesktop/login1/session/_31";

#[derive(Debug, PartialEq)]
enum Call {
    Lock,
    Suspend,
    Brightness(String, String, u32),
}

type Calls = Arc<Mutex<Sender<Call>>>;

fn record(calls: &Calls, call: Call) {
    if let Ok(calls) = calls.lock() {
        let _ = calls.send(call);
    }
}

struct FakeManager {
    calls: Calls,
    can_suspend: String,
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl FakeManager {
    fn get_session(&self, _session_id: String) -> zbus::fdo::Result<OwnedObjectPath> {
        OwnedObjectPath::try_from(SESSION_PATH)
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    fn can_suspend(&self) -> String {
        self.can_suspend.clone()
    }

    fn suspend(&self, _interactive: bool) {
        record(&self.calls, Call::Suspend);
    }
}

struct FakeSession {
    calls: Calls,
}

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl FakeSession {
    fn lock(&self) {
        record(&self.calls, Call::Lock);
    }

    fn set_brightness(&self, subsystem: String, name: String, brightness: u32) {
        record(&self.calls, Call::Brightness(subsystem, name, brightness));
    }

    #[zbus(property)]
    fn locked_hint(&self) -> bool {
        false
    }
}

struct Fixture {
    idle: Arc<IdleController>,
    calls: Receiver<Call>,
    path: std::path::PathBuf,
    _config: ConfigService,
    _logind: Connection,
    _directory: tempfile::TempDir,
    _bus: SessionBus,
}

fn start(config: &str, can_suspend: &str) -> Fixture {
    start_with_backlight(config, can_suspend, None)
}

fn start_with_backlight(config: &str, can_suspend: &str, backlight: Option<Backlight>) -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let (sender, calls) = mpsc::channel();
    let shared: Calls = Arc::new(Mutex::new(sender));
    let logind = bus.connect().expect("logind connection");
    logind
        .object_server()
        .at(
            "/org/freedesktop/login1",
            FakeManager {
                calls: Arc::clone(&shared),
                can_suspend: can_suspend.to_owned(),
            },
        )
        .expect("manager");
    logind
        .object_server()
        .at(SESSION_PATH, FakeSession { calls: shared })
        .expect("session");
    logind
        .request_name("org.freedesktop.login1")
        .expect("logind name");

    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("config.kdl");
    std::fs::write(&path, config).expect("config");
    let config = ConfigService::start(
        &bus.connect().expect("config connection"),
        &path,
        Arc::new(ConfigStore::default()),
        |_| {},
    )
    .expect("config service");

    let power = bus.connect().expect("power connection");
    let idle = Arc::new(IdleController::new(
        controller::configured_policy(&power).expect("policy"),
        controller::logind_executor(
            ManagerProxyBlocking::new(&power).expect("manager proxy"),
            logind::own_session(&power).expect("session proxy"),
            backlight,
        ),
    ));
    controller::follow_config(&power, &idle).expect("follow config");
    controller::follow_lock(&power, &idle).expect("follow lock");
    Fixture {
        idle,
        calls,
        path,
        _config: config,
        _logind: logind,
        _directory: directory,
        _bus: bus,
    }
}

impl Fixture {
    fn next(&self) -> Call {
        self.calls.recv_timeout(WAIT).expect("logind call")
    }

    fn quiet(&self) {
        assert_eq!(self.calls.recv_timeout(QUIET).ok(), None);
    }
}

#[test]
fn by_default_the_computer_locks_and_sleeps_when_the_display_turns_off() {
    let fixture = start("", "yes");
    assert_eq!(fixture.idle.thresholds(), [5 * MINUTE, 10 * MINUTE]);

    fixture.idle.idle_for(5 * MINUTE);
    fixture.quiet();
    fixture.idle.idle_for(10 * MINUTE);
    assert_eq!(fixture.next(), Call::Lock);
    assert_eq!(fixture.next(), Call::Suspend);
}

#[test]
fn a_later_suspend_after_waits_for_its_own_threshold() {
    let fixture = start("idle {\n    suspend-after \"1h\"\n}\n", "yes");
    fixture.idle.idle_for(10 * MINUTE);
    assert_eq!(fixture.next(), Call::Lock);
    fixture.quiet();
    fixture.idle.idle_for(60 * MINUTE);
    assert_eq!(fixture.next(), Call::Suspend);
}

#[test]
fn a_computer_that_cannot_suspend_only_locks() {
    let fixture = start("", "na");
    fixture.idle.idle_for(10 * MINUTE);
    assert_eq!(fixture.next(), Call::Lock);
    fixture.quiet();
}

#[test]
fn a_config_change_reaches_the_idle_policy() {
    let fixture = start("", "yes");
    let staging = fixture.path.with_extension("tmp");
    std::fs::write(&staging, "idle {\n    suspend-after \"1h\"\n}\n").expect("write");
    std::fs::rename(&staging, &fixture.path).expect("rename");

    let deadline = std::time::Instant::now() + WAIT;
    while fixture.idle.thresholds() != [5 * MINUTE, 10 * MINUTE, 60 * MINUTE] {
        assert!(std::time::Instant::now() < deadline, "policy not updated");
        std::thread::sleep(Duration::from_millis(20));
    }
    fixture.idle.idle_for(10 * MINUTE);
    assert_eq!(fixture.next(), Call::Lock);
    fixture.quiet();
}

fn fake_backlight(class: &std::path::Path, name: &str, kind: &str, brightness: u32) {
    let device = class.join(name);
    std::fs::create_dir_all(&device).expect("device directory");
    std::fs::write(device.join("type"), format!("{kind}\n")).expect("type");
    std::fs::write(device.join("brightness"), format!("{brightness}\n")).expect("brightness");
}

#[test]
fn the_firmware_backlight_is_preferred_over_raw_and_platform_ones() {
    let class = tempfile::tempdir().expect("backlight class");
    fake_backlight(class.path(), "acpi_video0", "raw", 10);
    fake_backlight(class.path(), "nvidia_0", "platform", 10);
    fake_backlight(class.path(), "apple-panel-bl", "firmware", 10);

    let backlight = Backlight::find(class.path()).expect("a backlight");
    assert_eq!(backlight.name, "apple-panel-bl");
}

#[test]
fn the_display_dims_before_it_locks_and_activity_restores_its_brightness() {
    let class = tempfile::tempdir().expect("backlight class");
    fake_backlight(class.path(), "apple-panel-bl", "firmware", 800);
    let fixture = start_with_backlight("", "yes", Backlight::find(class.path()));
    let brightness = |level| Call::Brightness("backlight".into(), "apple-panel-bl".into(), level);

    fixture.idle.idle_for(5 * MINUTE);
    assert_eq!(fixture.next(), brightness(200));
    fixture.idle.resumed();
    assert_eq!(fixture.next(), brightness(800));
    fixture.quiet();
}
