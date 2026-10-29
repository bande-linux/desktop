#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use unbound_config::{BUS_NAME, ConfigService, ConfigStore, OBJECT_PATH, ReloadError};
use unbound_test_bus::SessionBus;
use zbus::blocking::Connection;
use zbus::blocking::fdo::{DBusProxy, PropertiesProxy};
use zbus::names::InterfaceName;
use zbus::zvariant::{OwnedValue, Value};

const INTERFACE: &str = "org.unbound.Config";
const SIGNAL_WAIT: Duration = Duration::from_secs(5);
const QUIET_WAIT: Duration = Duration::from_millis(1500);

type Changes = HashMap<String, OwnedValue>;

fn replace_file(path: &Path, contents: &str) {
    let staging = path.with_extension("kdl.tmp");
    std::fs::write(&staging, contents).expect("write staging file");
    std::fs::rename(&staging, path).expect("rename into place");
}

fn properties(connection: &Connection) -> PropertiesProxy<'static> {
    PropertiesProxy::builder(connection)
        .destination(BUS_NAME)
        .expect("destination")
        .path(OBJECT_PATH)
        .expect("path")
        .build()
        .expect("properties proxy")
}

fn watch_changes(connection: &Connection) -> Receiver<Changes> {
    let proxy = properties(connection);
    let signals = proxy
        .receive_properties_changed()
        .expect("subscribe to PropertiesChanged");
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for signal in signals {
            let Ok(arguments) = signal.args() else {
                continue;
            };
            if arguments.interface_name().as_str() != INTERFACE {
                continue;
            }
            let changes = arguments
                .changed_properties()
                .iter()
                .filter_map(|(name, value)| Some((name.to_string(), value.try_to_owned().ok()?)))
                .collect();
            if sender.send(changes).is_err() {
                break;
            }
        }
    });
    receiver
}

fn get(connection: &Connection, property: &str) -> OwnedValue {
    properties(connection)
        .get(
            InterfaceName::from_static_str_unchecked(INTERFACE),
            property,
        )
        .expect("property")
}

struct Fixture {
    _bus: SessionBus,
    _directory: tempfile::TempDir,
    path: std::path::PathBuf,
    _service: ConfigService,
    client: Connection,
    errors: Receiver<ReloadError>,
}

fn start(initial: &str) -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("unbound").join("config.kdl");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("config dir");
    std::fs::write(&path, initial).expect("initial config");

    let (error_sender, errors) = mpsc::channel();
    let service_connection = bus.connect().expect("service connection");
    let service = ConfigService::start(
        &service_connection,
        &path,
        Arc::new(ConfigStore::default()),
        move |result: Result<(), ReloadError>| {
            if let Err(error) = result {
                let _ = error_sender.send(error);
            }
        },
    )
    .expect("config service");
    let client = bus.connect().expect("client connection");
    Fixture {
        _bus: bus,
        _directory: directory,
        path,
        _service: service,
        client,
        errors,
    }
}

#[test]
fn the_service_owns_its_bus_name_and_exposes_the_loaded_file() {
    let fixture = start("dock {\n    edge \"right\"\n}\n");
    let bus = DBusProxy::new(&fixture.client).expect("bus proxy");
    assert!(
        bus.name_has_owner(BUS_NAME.try_into().expect("bus name"))
            .expect("NameHasOwner")
    );
    assert_eq!(
        String::try_from(get(&fixture.client, "DockEdge")).expect("string"),
        "right"
    );
    assert_eq!(
        u64::try_from(get(&fixture.client, "IdleLockAfter")).expect("u64"),
        600
    );
}

#[test]
fn replacing_the_file_emits_only_the_changed_properties() {
    let fixture = start("appearance \"dark\"\ndock {\n    edge \"left\"\n}\n");
    let changes = watch_changes(&fixture.client);

    replace_file(
        &fixture.path,
        "appearance \"dark\"\ndock {\n    edge \"top\"\n}\n",
    );

    let changed = changes
        .recv_timeout(SIGNAL_WAIT)
        .expect("PropertiesChanged");
    assert_eq!(
        changed.keys().map(String::as_str).collect::<Vec<_>>(),
        ["DockEdge"]
    );
    assert_eq!(
        String::try_from(changed["DockEdge"].try_clone().expect("clone")).expect("string"),
        "top"
    );
    assert_eq!(
        String::try_from(get(&fixture.client, "DockEdge")).expect("string"),
        "top"
    );
}

#[test]
fn writing_the_file_in_place_is_picked_up() {
    let fixture = start("");
    let changes = watch_changes(&fixture.client);

    std::fs::write(&fixture.path, "idle {\n    dim-after \"1m\"\n}\n").expect("write");

    let changed = changes
        .recv_timeout(SIGNAL_WAIT)
        .expect("PropertiesChanged");
    assert_eq!(
        u64::try_from(changed["IdleDimAfter"].try_clone().expect("clone")).expect("u64"),
        60
    );
}

#[test]
fn an_invalid_file_changes_nothing_and_reports_where_it_is_wrong() {
    let fixture = start("dock {\n    edge \"left\"\n}\n");
    let changes = watch_changes(&fixture.client);

    replace_file(&fixture.path, "dock {\n    edge \"middle\"\n}\n");

    match fixture
        .errors
        .recv_timeout(SIGNAL_WAIT)
        .expect("reload error")
    {
        ReloadError::Invalid(error) => assert_eq!((error.line, error.column), (2, 10)),
        ReloadError::Read(error) => panic!("unexpected read error {error}"),
    }
    assert!(changes.recv_timeout(QUIET_WAIT).is_err());
    assert_eq!(
        String::try_from(get(&fixture.client, "DockEdge")).expect("string"),
        "left"
    );
}

fn set(connection: &Connection, property: &str, value: Value<'_>) -> zbus::fdo::Result<()> {
    properties(connection).set(
        InterfaceName::from_static_str_unchecked(INTERFACE),
        property,
        value,
    )
}

#[test]
fn setting_a_property_rewrites_the_file_and_signals_the_change_once() {
    let fixture = start("// mine\ndock {\n    edge \"left\" // keep\n}\n");
    let changes = watch_changes(&fixture.client);

    set(&fixture.client, "DockEdge", Value::from("bottom")).expect("Set");

    assert_eq!(
        std::fs::read_to_string(&fixture.path).expect("read"),
        "// mine\ndock {\n    edge \"bottom\" // keep\n}\n"
    );
    assert_eq!(
        String::try_from(get(&fixture.client, "DockEdge")).expect("string"),
        "bottom"
    );
    let changed = changes
        .recv_timeout(SIGNAL_WAIT)
        .expect("PropertiesChanged");
    assert_eq!(
        changed.keys().map(String::as_str).collect::<Vec<_>>(),
        ["DockEdge"]
    );
    assert!(changes.recv_timeout(QUIET_WAIT).is_err());
    assert!(fixture.errors.try_recv().is_err());
}

#[test]
fn a_value_the_schema_rejects_leaves_the_file_alone() {
    let initial = "appearance \"dark\"\n";
    let fixture = start(initial);

    let unknown = set(&fixture.client, "Appearance", Value::from("sepia"));
    assert!(
        matches!(unknown, Err(zbus::fdo::Error::InvalidArgs(_))),
        "{unknown:?}"
    );
    let too_late = set(&fixture.client, "IdleDimAfter", Value::from(3600u64));
    assert!(
        matches!(too_late, Err(zbus::fdo::Error::InvalidArgs(_))),
        "{too_late:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&fixture.path).expect("read"),
        initial
    );
}

#[test]
fn an_invalid_file_is_not_overwritten_by_a_setting() {
    let invalid = "dock {\n    edge \"middle\"\n}\n";
    let fixture = start(invalid);

    let result = set(&fixture.client, "Appearance", Value::from("dark"));
    assert!(
        matches!(result, Err(zbus::fdo::Error::Failed(_))),
        "{result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&fixture.path).expect("read"),
        invalid
    );
}

#[test]
fn set_output_writes_the_connector_and_publishes_outputs() {
    let fixture = start("");
    let changes = watch_changes(&fixture.client);
    let scale = Value::from(1.25f64);
    let vrr = Value::from(true);

    fixture
        .client
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE),
            "SetOutput",
            &(
                "eDP-1",
                HashMap::from([("scale", &scale), ("variable-refresh-rate", &vrr)]),
            ),
        )
        .expect("SetOutput");

    let changed = changes
        .recv_timeout(SIGNAL_WAIT)
        .expect("PropertiesChanged");
    assert_eq!(
        changed.keys().map(String::as_str).collect::<Vec<_>>(),
        ["Outputs"]
    );
    let outputs: HashMap<String, HashMap<String, OwnedValue>> = get(&fixture.client, "Outputs")
        .try_into()
        .expect("a{sa{sv}}");
    assert_eq!(f64::try_from(&outputs["eDP-1"]["scale"]).expect("d"), 1.25);
    assert!(bool::try_from(&outputs["eDP-1"]["variable-refresh-rate"]).expect("b"));
    let written =
        unbound_config::Config::parse(&std::fs::read_to_string(&fixture.path).expect("read"))
            .expect("valid file");
    assert_eq!(written.outputs["eDP-1"].scale, Some(1.25));
}

#[test]
fn moving_the_lock_moves_a_suspend_that_follows_the_display() {
    let fixture = start("");
    let changes = watch_changes(&fixture.client);
    assert_eq!(
        u64::try_from(get(&fixture.client, "IdleSuspendAfter")).expect("u64"),
        600
    );

    set(&fixture.client, "IdleLockAfter", Value::from(1200u64)).expect("Set");

    let mut changed = Changes::new();
    while changed.len() < 2 {
        changed.extend(
            changes
                .recv_timeout(SIGNAL_WAIT)
                .expect("PropertiesChanged"),
        );
    }
    assert_eq!(
        u64::try_from(changed["IdleSuspendAfter"].try_clone().expect("clone")).expect("u64"),
        1200
    );
    assert!(changed.contains_key("IdleLockAfter"));
}

#[test]
fn key_repeat_and_wallpaper_are_writable_properties() {
    let fixture = start("");
    assert_eq!(
        u32::try_from(get(&fixture.client, "KeyRepeatDelay")).expect("u32"),
        600
    );
    set(&fixture.client, "KeyRepeatRate", Value::from(40u32)).expect("Set");
    set(
        &fixture.client,
        "WallpaperLight",
        Value::from("/usr/share/backgrounds/day.jpg"),
    )
    .expect("Set");
    assert_eq!(
        u32::try_from(get(&fixture.client, "KeyRepeatRate")).expect("u32"),
        40
    );
    assert_eq!(
        String::try_from(get(&fixture.client, "WallpaperLight")).expect("string"),
        "/usr/share/backgrounds/day.jpg"
    );
    let too_fast = set(&fixture.client, "KeyRepeatRate", Value::from(100u32));
    assert!(
        matches!(too_fast, Err(zbus::fdo::Error::InvalidArgs(_))),
        "{too_fast:?}"
    );
    set(&fixture.client, "WallpaperLight", Value::from("")).expect("Set");
    assert_eq!(
        String::try_from(get(&fixture.client, "WallpaperLight")).expect("string"),
        ""
    );
}

#[test]
fn suspend_after_zero_means_never() {
    let fixture = start("");
    set(&fixture.client, "IdleSuspendAfter", Value::from(0u64)).expect("Set");
    assert_eq!(
        u64::try_from(get(&fixture.client, "IdleSuspendAfter")).expect("u64"),
        0
    );
    assert!(
        std::fs::read_to_string(&fixture.path)
            .expect("read")
            .contains("suspend-after \"never\"")
    );
}

#[test]
fn sleep_can_be_returned_to_following_the_display() {
    let fixture = start("idle {\n    suspend-after \"3h\"\n}\n");
    assert!(!bool::try_from(get(&fixture.client, "IdleSuspendWithDisplay")).expect("b"));
    let changes = watch_changes(&fixture.client);

    set(&fixture.client, "IdleSuspendWithDisplay", Value::from(true)).expect("Set");

    let mut changed = Changes::new();
    while changed.len() < 2 {
        changed.extend(
            changes
                .recv_timeout(SIGNAL_WAIT)
                .expect("PropertiesChanged"),
        );
    }
    assert_eq!(
        u64::try_from(changed["IdleSuspendAfter"].try_clone().expect("clone")).expect("u64"),
        600
    );
    assert!(bool::try_from(get(&fixture.client, "IdleSuspendWithDisplay")).expect("b"));
    assert!(
        !std::fs::read_to_string(&fixture.path)
            .expect("read")
            .contains("suspend-after")
    );
}
