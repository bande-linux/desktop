#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use common::Portal;
use unbound_config::{ConfigService, ConfigStore};
use unbound_graphics::{Palette, Scheme};
use xdg_desktop_portal_unbound::settings::Settings;
use zbus::blocking::MessageIterator;
use zbus::zvariant::OwnedValue;
use zbus::{MatchRule, message};

const INTERFACE: &str = "org.freedesktop.impl.portal.Settings";
const APPEARANCE: &str = "org.freedesktop.appearance";
const GNOME_INTERFACE: &str = "org.gnome.desktop.interface";
const WAIT: Duration = Duration::from_secs(5);

type All = HashMap<String, HashMap<String, OwnedValue>>;

struct Fixture {
    portal: Portal,
    path: PathBuf,
    _config: ConfigService,
    _directory: tempfile::TempDir,
}

fn start(config: &str) -> Fixture {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("config.kdl");
    std::fs::write(&path, config).expect("config");
    let mut service = None;
    let portal = Portal::start(|connection| {
        service = Some(
            ConfigService::start(connection, &path, Arc::new(ConfigStore::default()), |_| {})
                .expect("config service"),
        );
        Settings::serve(connection, "Adwaita".to_owned()).expect("settings");
    });
    Fixture {
        portal,
        path,
        _config: service.expect("config"),
        _directory: directory,
    }
}

impl Fixture {
    fn read_all(&self, namespaces: &[&str]) -> All {
        self.portal
            .call(INTERFACE, "ReadAll", &(namespaces,))
            .expect("ReadAll")
            .body()
            .deserialize()
            .expect("a{sa{sv}}")
    }

    fn color_scheme(&self) -> u32 {
        let all = self.read_all(&[]);
        u32::try_from(&all[APPEARANCE]["color-scheme"]).expect("u")
    }

    fn changes(&self) -> Receiver<(String, String, OwnedValue)> {
        let rule = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .interface(INTERFACE)
            .expect("interface")
            .member("SettingChanged")
            .expect("member")
            .build();
        let signals =
            MessageIterator::for_match_rule(rule, &self.portal.client, None).expect("subscribe");
        let (sender, changes) = mpsc::channel();
        std::thread::spawn(move || {
            for signal in signals.flatten() {
                if let Ok(change) = signal.body().deserialize::<(String, String, OwnedValue)>() {
                    let _ = sender.send(change);
                }
            }
        });
        changes
    }
}

fn accent(scheme: Scheme) -> (f64, f64, f64) {
    let color = Palette::for_scheme(scheme).accent;
    (
        f64::from(color.r) / 255.0,
        f64::from(color.g) / 255.0,
        f64::from(color.b) / 255.0,
    )
}

#[test]
fn the_appearance_maps_to_the_portal_color_scheme() {
    assert_eq!(start("appearance \"dark\"\n").color_scheme(), 1);
    assert_eq!(start("appearance \"light\"\n").color_scheme(), 2);
    assert_eq!(start("").color_scheme(), 0);
}

#[test]
fn the_accent_color_is_the_toolkit_accent_as_rgb_fractions() {
    let fixture = start("appearance \"dark\"\n");
    let all = fixture.read_all(&[APPEARANCE]);
    let color =
        <(f64, f64, f64)>::try_from(all[APPEARANCE]["accent-color"].try_clone().expect("clone"))
            .expect("(ddd)");
    assert_eq!(color, accent(Scheme::Dark));
    for channel in [color.0, color.1, color.2] {
        assert!((0.0..=1.0).contains(&channel));
    }
}

#[test]
fn namespaces_filter_with_a_trailing_glob() {
    let fixture = start("");
    assert!(
        fixture
            .read_all(&["org.freedesktop.*"])
            .contains_key(APPEARANCE)
    );
    assert!(fixture.read_all(&["org.kde.*"]).is_empty());
    assert!(
        fixture
            .read_all(&["org.freedesktop.appearance"])
            .contains_key(APPEARANCE)
    );
}

#[test]
fn reading_an_unknown_key_is_not_found() {
    let fixture = start("");
    let error = fixture
        .portal
        .call(INTERFACE, "Read", &(APPEARANCE, "wallpaper"))
        .expect_err("unknown key");
    assert!(
        matches!(&error, zbus::Error::MethodError(name, _, _) if name.as_str() == "org.freedesktop.portal.Error.NotFound"),
        "{error:?}"
    );
    let value: OwnedValue = fixture
        .portal
        .call(INTERFACE, "Read", &(APPEARANCE, "color-scheme"))
        .expect("Read")
        .body()
        .deserialize()
        .expect("v");
    assert_eq!(u32::try_from(&value).expect("u"), 0);
}

#[test]
fn switching_to_dark_signals_the_color_scheme_and_accent() {
    let fixture = start("appearance \"light\"\n");
    let changes = fixture.changes();
    let staging = fixture.path.with_extension("tmp");
    std::fs::write(&staging, "appearance \"dark\"\n").expect("write");
    std::fs::rename(&staging, &fixture.path).expect("rename");

    let mut seen = HashMap::new();
    while seen.len() < 2 {
        let (namespace, key, value) = changes.recv_timeout(WAIT).expect("SettingChanged");
        if namespace == APPEARANCE {
            seen.insert(key, value);
        }
    }
    assert_eq!(u32::try_from(&seen["color-scheme"]).expect("u"), 1);
    assert_eq!(
        <(f64, f64, f64)>::try_from(seen["accent-color"].try_clone().expect("clone"))
            .expect("(ddd)"),
        accent(Scheme::Dark)
    );
}

#[test]
fn reduce_motion_in_the_configuration_is_served_and_signalled_as_reduced_motion() {
    let fixture = start("");
    let reduced = |fixture: &Fixture| {
        u32::try_from(&fixture.read_all(&[APPEARANCE])[APPEARANCE]["reduced-motion"]).expect("u")
    };
    assert_eq!(reduced(&fixture), 0);
    let changes = fixture.changes();
    let staging = fixture.path.with_extension("tmp");
    std::fs::write(&staging, "accessibility {\n    reduce-motion #true\n}\n").expect("write");
    std::fs::rename(&staging, &fixture.path).expect("rename");

    let (namespace, key, value) = changes.recv_timeout(WAIT).expect("SettingChanged");
    assert_eq!(
        (namespace.as_str(), key.as_str()),
        (APPEARANCE, "reduced-motion")
    );
    assert_eq!(u32::try_from(&value).expect("u"), 1);
    assert_eq!(reduced(&fixture), 1);
}

#[test]
fn the_interface_is_version_2() {
    let fixture = start("");
    let version: OwnedValue = fixture
        .portal
        .client
        .call_method(
            Some(xdg_desktop_portal_unbound::BUS_NAME),
            xdg_desktop_portal_unbound::OBJECT_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(INTERFACE, "version"),
        )
        .expect("Get")
        .body()
        .deserialize()
        .expect("v");
    assert_eq!(u32::try_from(&version).expect("u"), 2);
}

#[test]
fn the_display_accommodations_are_served_to_gtk_qt_and_the_toolkit() {
    let fixture = start(
        "accessibility {\n    increase-contrast #true\n    reduce-transparency #true\n    text-size 150\n}\n",
    );
    let all = fixture.read_all(&[]);
    assert_eq!(u32::try_from(&all[APPEARANCE]["contrast"]).expect("u"), 1);
    let unbound = &all["org.unbound.appearance"];
    assert_eq!(
        u32::try_from(&unbound["reduce-transparency"]).expect("u"),
        1
    );
    assert_eq!(u32::try_from(&unbound["text-size"]).expect("u"), 150);
    assert_eq!(
        f64::try_from(&all[GNOME_INTERFACE]["text-scaling-factor"]).expect("d"),
        1.5
    );
    let standard = start("").read_all(&[APPEARANCE]);
    assert_eq!(
        u32::try_from(&standard[APPEARANCE]["contrast"]).expect("u"),
        0
    );
}

#[test]
fn gtk_reads_the_color_scheme_and_cursor_from_the_gnome_interface_namespace() {
    let fixture = start("appearance \"dark\"\naccessibility {\n    cursor-size 48\n}\n");
    let all = fixture.read_all(&["org.gnome.desktop.interface"]);
    let interface = &all[GNOME_INTERFACE];

    assert_eq!(
        String::try_from(interface["color-scheme"].try_clone().expect("clone")).expect("s"),
        "prefer-dark"
    );
    assert_eq!(
        String::try_from(interface["cursor-theme"].try_clone().expect("clone")).expect("s"),
        "Adwaita"
    );
    assert_eq!(i32::try_from(&interface["cursor-size"]).expect("i"), 48);
}

#[test]
fn gtk_plays_its_event_sounds_from_the_unbound_sound_theme() {
    let fixture = start("");
    let all = fixture.read_all(&["org.gnome.desktop.sound"]);
    let sound = &all["org.gnome.desktop.sound"];

    assert_eq!(
        String::try_from(sound["theme-name"].try_clone().expect("clone")).expect("s"),
        "Unbound"
    );
    assert!(bool::try_from(&sound["event-sounds"]).expect("b"));
}

#[test]
fn switching_to_light_signals_the_gnome_color_scheme() {
    let fixture = start("appearance \"dark\"\n");
    let changes = fixture.changes();
    let staging = fixture.path.with_extension("tmp");
    std::fs::write(&staging, "appearance \"light\"\n").expect("write");
    std::fs::rename(&staging, &fixture.path).expect("rename");

    let value = loop {
        let (namespace, key, value) = changes.recv_timeout(WAIT).expect("SettingChanged");
        if namespace == GNOME_INTERFACE && key == "color-scheme" {
            break value;
        }
        assert_ne!(namespace, GNOME_INTERFACE, "only color-scheme changes");
    };
    assert_eq!(String::try_from(value).expect("s"), "prefer-light");
}
