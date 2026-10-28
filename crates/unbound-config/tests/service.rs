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
use zbus::zvariant::OwnedValue;

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
        move |error| {
            let _ = error_sender.send(error);
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
    let fixture = start("panel {\n    edge \"right\"\n}\n");
    let bus = DBusProxy::new(&fixture.client).expect("bus proxy");
    assert!(
        bus.name_has_owner(BUS_NAME.try_into().expect("bus name"))
            .expect("NameHasOwner")
    );
    assert_eq!(
        String::try_from(get(&fixture.client, "PanelEdge")).expect("string"),
        "right"
    );
    assert_eq!(
        u64::try_from(get(&fixture.client, "IdleLockAfter")).expect("u64"),
        600
    );
}

#[test]
fn replacing_the_file_emits_only_the_changed_properties() {
    let fixture = start("appearance \"dark\"\npanel {\n    edge \"left\"\n}\n");
    let changes = watch_changes(&fixture.client);

    replace_file(
        &fixture.path,
        "appearance \"dark\"\npanel {\n    edge \"top\"\n}\n",
    );

    let changed = changes
        .recv_timeout(SIGNAL_WAIT)
        .expect("PropertiesChanged");
    assert_eq!(
        changed.keys().map(String::as_str).collect::<Vec<_>>(),
        ["PanelEdge"]
    );
    assert_eq!(
        String::try_from(changed["PanelEdge"].try_clone().expect("clone")).expect("string"),
        "top"
    );
    assert_eq!(
        String::try_from(get(&fixture.client, "PanelEdge")).expect("string"),
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
    let fixture = start("panel {\n    edge \"left\"\n}\n");
    let changes = watch_changes(&fixture.client);

    replace_file(&fixture.path, "panel {\n    edge \"middle\"\n}\n");

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
        String::try_from(get(&fixture.client, "PanelEdge")).expect("string"),
        "left"
    );
}
