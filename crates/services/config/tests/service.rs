#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use bande_config::{BUS_NAME, ConfigService, ConfigStore, Loaded, OBJECT_PATH, ReloadError};
use bande_test_bus::SessionBus;
use support::replace_file;
use zbus::blocking::Connection;
use zbus::blocking::fdo::{DBusProxy, PropertiesProxy};
use zbus::names::InterfaceName;
use zbus::zvariant::{OwnedValue, Value};

const INTERFACE: &str = "org.bande.Config";
const SIGNAL_WAIT: Duration = Duration::from_secs(5);
const QUIET_WAIT: Duration = Duration::from_millis(1500);

type Changes = HashMap<String, OwnedValue>;

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
    let path = directory.path().join("bande").join("config.kdl");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("config dir");
    std::fs::write(&path, initial).expect("initial config");

    let (error_sender, errors) = mpsc::channel();
    let service_connection = bus.connect().expect("service connection");
    let service = ConfigService::start(
        &service_connection,
        &path,
        Arc::new(ConfigStore::default()),
        move |result: Result<Loaded, ReloadError>| {
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
    let fixture = start("bar {\n    edge \"right\"\n    apps \"org.bande.Files\"\n}\n");
    let bus = DBusProxy::new(&fixture.client).expect("bus proxy");
    assert!(
        bus.name_has_owner(BUS_NAME.try_into().expect("bus name"))
            .expect("NameHasOwner")
    );
    assert_eq!(
        String::try_from(get(&fixture.client, "BarEdge")).expect("string"),
        "right"
    );
    assert_eq!(
        Vec::<String>::try_from(get(&fixture.client, "BarApps")).expect("app IDs"),
        ["org.bande.Files"]
    );
    assert_eq!(
        u64::try_from(get(&fixture.client, "IdleLockAfter")).expect("u64"),
        600
    );
}

#[test]
fn replacing_the_file_emits_only_the_changed_properties() {
    let fixture = start("appearance \"dark\"\nbar {\n    edge \"left\"\n}\n");
    let changes = watch_changes(&fixture.client);

    replace_file(
        &fixture.path,
        "appearance \"dark\"\nbar {\n    edge \"top\"\n}\n",
    );

    let changed = changes
        .recv_timeout(SIGNAL_WAIT)
        .expect("PropertiesChanged");
    assert_eq!(
        changed.keys().map(String::as_str).collect::<Vec<_>>(),
        ["BarEdge"]
    );
    assert_eq!(
        String::try_from(changed["BarEdge"].try_clone().expect("clone")).expect("string"),
        "top"
    );
    assert_eq!(
        String::try_from(get(&fixture.client, "BarEdge")).expect("string"),
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
fn an_unreadable_file_changes_nothing_and_reports_where_it_is_wrong() {
    let fixture = start("bar {\n    edge \"left\"\n}\n");
    let changes = watch_changes(&fixture.client);

    replace_file(&fixture.path, "bar {\n    edge \"middle\n}\n");

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
        String::try_from(get(&fixture.client, "BarEdge")).expect("string"),
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
    let fixture = start("// mine\nbar {\n    edge \"left\" // keep\n}\n");
    let changes = watch_changes(&fixture.client);

    set(&fixture.client, "BarEdge", Value::from("bottom")).expect("Set");

    assert_eq!(
        std::fs::read_to_string(&fixture.path).expect("read"),
        "// mine\nbar {\n    edge \"bottom\" // keep\n}\n"
    );
    assert_eq!(
        String::try_from(get(&fixture.client, "BarEdge")).expect("string"),
        "bottom"
    );
    let changed = changes
        .recv_timeout(SIGNAL_WAIT)
        .expect("PropertiesChanged");
    assert_eq!(
        changed.keys().map(String::as_str).collect::<Vec<_>>(),
        ["BarEdge"]
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
fn an_unreadable_file_is_not_overwritten_by_a_setting() {
    let invalid = "bar {\n    edge \"middle\n}\n";
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
fn set_display_writes_the_display_and_publishes_displays() {
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
            "SetDisplay",
            &(
                "BOE NE135A1M 0",
                HashMap::from([("scale", &scale), ("variable-refresh-rate", &vrr)]),
            ),
        )
        .expect("SetDisplay");

    let changed = changes
        .recv_timeout(SIGNAL_WAIT)
        .expect("PropertiesChanged");
    assert_eq!(
        changed.keys().map(String::as_str).collect::<Vec<_>>(),
        ["Displays"]
    );
    let displays: HashMap<String, HashMap<String, OwnedValue>> = get(&fixture.client, "Displays")
        .try_into()
        .expect("a{sa{sv}}");
    let display = &displays["BOE NE135A1M 0"];
    assert_eq!(f64::try_from(&display["scale"]).expect("d"), 1.25);
    assert!(bool::try_from(&display["variable-refresh-rate"]).expect("b"));
    let written =
        bande_config::Config::parse(&std::fs::read_to_string(&fixture.path).expect("read"))
            .expect("valid file");
    assert_eq!(written.displays["BOE NE135A1M 0"].scale, Some(1.25));
}

#[test]
fn set_arrangement_writes_the_arrangement_and_publishes_arrangements() {
    let fixture = start("");
    let changes = watch_changes(&fixture.client);
    let arranged = vec![
        ("BOE".to_owned(), 0, 0, true, String::new()),
        ("Dell".to_owned(), 1504, -300, false, String::new()),
    ];

    fixture
        .client
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE),
            "SetArrangement",
            &(arranged.clone(),),
        )
        .expect("SetArrangement");

    let changed = changes
        .recv_timeout(SIGNAL_WAIT)
        .expect("PropertiesChanged");
    assert_eq!(
        changed.keys().map(String::as_str).collect::<Vec<_>>(),
        ["Arrangements"]
    );
    let arrangements: Vec<Vec<bande_config::ArrangedDisplay>> =
        get(&fixture.client, "Arrangements")
            .try_into()
            .expect("aa(siibs)");
    assert_eq!(arrangements, [arranged]);
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
fn settings_are_writable_properties_within_their_limits() {
    let fixture = start("");
    assert_eq!(
        u32::try_from(get(&fixture.client, "KeyRepeatDelay")).expect("u32"),
        600
    );
    assert!(
        !Vec::<String>::try_from(get(&fixture.client, "InputSources"))
            .expect("strings")
            .is_empty()
    );
    for (property, value) in [
        ("KeyRepeatRate", Value::from(40u32)),
        (
            "WallpaperLight",
            Value::from("/usr/share/backgrounds/day.jpg"),
        ),
        ("WallpaperLight", Value::from("")),
        ("InputSources", Value::from(vec!["se", "us(dvorak)"])),
        ("TrackpadSpeed", Value::from(0.5)),
        ("TrackpadSecondaryClick", Value::from("corner")),
        ("IdleSuspendAfter", Value::from(0u64)),
    ] {
        set(&fixture.client, property, value.try_clone().expect("clone")).expect(property);
        assert_eq!(
            get(&fixture.client, property),
            OwnedValue::try_from(value).expect("owned"),
            "{property}"
        );
    }
    assert!(
        std::fs::read_to_string(&fixture.path)
            .expect("read")
            .contains("suspend-after \"never\"")
    );
    for (property, value) in [
        ("KeyRepeatRate", Value::from(100u32)),
        ("InputSources", Value::from(Vec::<&str>::new())),
        ("InputSources", Value::from(vec!["us,se"])),
        ("MouseSpeed", Value::from(3.0)),
        ("MousePrimaryButton", Value::from("middle")),
        ("TrackpadDragging", Value::from("always")),
        ("TrackpadScrollMethod", Value::from("corner")),
    ] {
        let rejected = set(&fixture.client, property, value);
        assert!(
            matches!(rejected, Err(zbus::fdo::Error::InvalidArgs(_))),
            "{property}: {rejected:?}"
        );
    }
}

#[test]
fn every_pointer_option_round_trips_and_is_signalled() {
    let fixture = start("");
    let changes = watch_changes(&fixture.client);
    let options = [
        ("MousePrimaryButton", Value::from("right")),
        ("MouseAcceleration", Value::from(false)),
        ("TrackpadAcceleration", Value::from(false)),
        ("TrackpadDragging", Value::from("tap-with-lock")),
        ("TrackpadScrollMethod", Value::from("edge")),
        ("TrackpadWorkspaceSwipe", Value::from(false)),
        ("TrackpadOverviewSwipe", Value::from(false)),
        ("TrackpadDesktopSpread", Value::from(false)),
        ("TrackpadLauncherPinch", Value::from(false)),
    ];
    for (property, value) in &options {
        set(&fixture.client, property, value.try_clone().expect("clone")).expect(property);
        assert_eq!(
            get(&fixture.client, property),
            OwnedValue::try_from(value.try_clone().expect("clone")).expect("owned"),
            "{property}"
        );
    }
    let mut changed = Changes::new();
    while options
        .iter()
        .any(|(property, _)| !changed.contains_key(*property))
    {
        changed.extend(
            changes
                .recv_timeout(SIGNAL_WAIT)
                .expect("PropertiesChanged"),
        );
    }
}

#[test]
fn night_light_is_written_whole_and_rejects_a_bad_time_schedule_or_warmth() {
    let fixture = start("");
    let changes = watch_changes(&fixture.client);
    let night = |on: bool, schedule: &'static str, from: &'static str, warmth: u32| {
        Value::from((on, schedule, from, "07:00", warmth))
    };

    set(
        &fixture.client,
        "NightLight",
        night(true, "custom", "21:30", 70),
    )
    .expect("Set");

    assert_eq!(
        <(bool, String, String, String, u32)>::try_from(get(&fixture.client, "NightLight"))
            .expect("night light"),
        (
            true,
            "custom".to_owned(),
            "21:30".to_owned(),
            "07:00".to_owned(),
            70
        )
    );
    let written = std::fs::read_to_string(&fixture.path).expect("read");
    assert!(written.contains("from \"21:30\""), "{written}");
    let changed = changes
        .recv_timeout(SIGNAL_WAIT)
        .expect("PropertiesChanged");
    assert!(changed.contains_key("NightLight"));
    for rejected in [
        night(true, "custom", "25:00", 70),
        night(true, "always", "21:30", 70),
        night(true, "custom", "21:30", 101),
    ] {
        let result = set(&fixture.client, "NightLight", rejected);
        assert!(
            matches!(result, Err(zbus::fdo::Error::InvalidArgs(_))),
            "{result:?}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(&fixture.path).expect("read"),
        written
    );
}

#[test]
fn the_dock_apps_size_auto_hide_and_wallpaper_fit_are_writable_and_signalled() {
    let fixture = start("bar {\n    apps \"org.bande.Files\" // mine\n}\n");
    let changes = watch_changes(&fixture.client);

    set(
        &fixture.client,
        "BarApps",
        Value::from(vec!["org.bande.Terminal", "org.bande.Files"]),
    )
    .expect("Set");
    set(&fixture.client, "BarAutoHide", Value::from(true)).expect("Set");
    set(&fixture.client, "BarIconSize", Value::from(52u32)).expect("Set");
    set(&fixture.client, "WallpaperFit", Value::from("fit")).expect("Set");

    let mut changed = Changes::new();
    while changed.len() < 4 {
        changed.extend(
            changes
                .recv_timeout(SIGNAL_WAIT)
                .expect("PropertiesChanged"),
        );
    }
    assert_eq!(
        Vec::<String>::try_from(changed["BarApps"].try_clone().expect("clone")).expect("as"),
        ["org.bande.Terminal", "org.bande.Files"]
    );
    assert!(bool::try_from(get(&fixture.client, "BarAutoHide")).expect("b"));
    assert_eq!(
        u32::try_from(get(&fixture.client, "BarIconSize")).expect("u32"),
        52
    );
    assert_eq!(
        String::try_from(get(&fixture.client, "WallpaperFit")).expect("s"),
        "fit"
    );
    assert_eq!(
        std::fs::read_to_string(&fixture.path).expect("read"),
        "bar {\n    apps \"org.bande.Terminal\" \"org.bande.Files\" // mine\n    auto-hide #true\n    size 52\n}\nwallpaper {\n    fit \"fit\"\n}\n"
    );
    let too_big = set(&fixture.client, "BarIconSize", Value::from(200u32));
    assert!(
        matches!(too_big, Err(zbus::fdo::Error::InvalidArgs(_))),
        "{too_big:?}"
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

#[test]
fn a_file_with_a_named_display_is_fully_applied() {
    let fixture = start(
        "appearance \"dark\"\ndisplay \"Virtual-1\" {\n    scale 1.5\n}\nbar {\n    size 44\n}\n",
    );
    assert_eq!(
        String::try_from(get(&fixture.client, "Appearance")).expect("string"),
        "dark"
    );
    assert_eq!(
        u32::try_from(get(&fixture.client, "BarIconSize")).expect("u32"),
        44
    );
    let displays: HashMap<String, HashMap<String, OwnedValue>> = get(&fixture.client, "Displays")
        .try_into()
        .expect("a{sa{sv}}");
    assert_eq!(
        f64::try_from(&displays["Virtual-1"]["scale"]).expect("d"),
        1.5
    );
}

#[test]
fn a_file_with_an_unknown_setting_is_served_and_still_writable() {
    let fixture = start("appearance \"dark\"\nsparkle #true\n");

    assert_eq!(
        String::try_from(get(&fixture.client, "Appearance")).expect("string"),
        "dark"
    );
    set(&fixture.client, "Appearance", Value::from("light")).expect("Set");
    assert_eq!(
        std::fs::read_to_string(&fixture.path).expect("read"),
        "appearance \"light\"\nsparkle #true\n"
    );
}
