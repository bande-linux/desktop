#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use common::Portal;
use unbound_config::{ConfigService, ConfigStore};
use unbound_design::{Accent, Color, Palette, Scheme};
use xdg_desktop_portal_unbound::gtk;
use xdg_desktop_portal_unbound::settings::{CursorSettings, Settings, UNBOUND_APPEARANCE};
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
    gtk_home: PathBuf,
    _config: ConfigService,
    _directory: tempfile::TempDir,
}

fn start(config: &str) -> Fixture {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("config.kdl");
    std::fs::write(&path, config).expect("config");
    let gtk_home = directory.path().join("config-home");
    let mut service = None;
    let portal = Portal::start(|connection| {
        service = Some(
            ConfigService::start(connection, &path, Arc::new(ConfigStore::default()), |_| {})
                .expect("config service"),
        );
        Settings::serve(
            connection,
            CursorSettings {
                theme: "Adwaita".to_owned(),
                size: 32,
            },
            Some(gtk_home.clone()),
        )
        .expect("settings");
    });
    Fixture {
        portal,
        path,
        gtk_home,
        _config: service.expect("config"),
        _directory: directory,
    }
}

impl Fixture {
    fn write_config(&self, config: &str) {
        let staging = self.path.with_extension("tmp");
        std::fs::write(&staging, config).expect("write");
        std::fs::rename(&staging, &self.path).expect("rename");
    }

    fn string(&self, namespace: &str, key: &str) -> String {
        let all = self.read_all(&[namespace]);
        String::try_from(all[namespace][key].try_clone().expect("clone")).expect("s")
    }

    fn stylesheet(&self) -> String {
        std::fs::read_to_string(self.gtk_home.join("gtk-4.0/unbound.css")).unwrap_or_default()
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
    assert!([1, 2].contains(&start("").color_scheme()));
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
    let fixture = start("appearance \"light\"\n");
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
    assert_eq!(u32::try_from(&value).expect("u"), 2);
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
fn gtk_reads_the_color_scheme_and_cursor_from_the_gnome_interface_namespace() {
    let fixture = start("appearance \"dark\"\n");
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
    assert_eq!(i32::try_from(&interface["cursor-size"]).expect("i"), 32);
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

fn fractions(color: Color) -> (f64, f64, f64) {
    (
        f64::from(color.r) / 255.0,
        f64::from(color.g) / 255.0,
        f64::from(color.b) / 255.0,
    )
}

fn hex(color: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

fn served_accent(fixture: &Fixture) -> (f64, f64, f64) {
    let all = fixture.read_all(&[APPEARANCE]);
    <(f64, f64, f64)>::try_from(all[APPEARANCE]["accent-color"].try_clone().expect("clone"))
        .expect("(ddd)")
}

#[test]
fn the_chosen_accent_is_served_exactly_with_its_folder_icon_theme() {
    let fixture = start("appearance \"light\"\naccent \"teal\"\n");
    assert_eq!(
        served_accent(&fixture),
        fractions(Accent::Teal.color(Scheme::Light))
    );
    assert_eq!(
        fixture.string(GNOME_INTERFACE, "icon-theme"),
        "Unbound-Teal"
    );

    let changes = fixture.changes();
    fixture.write_config("appearance \"light\"\n");
    let mut seen = HashMap::new();
    while seen.len() < 2 {
        let (_, key, value) = changes.recv_timeout(WAIT).expect("SettingChanged");
        seen.insert(key, value);
    }
    assert_eq!(
        <(f64, f64, f64)>::try_from(seen["accent-color"].try_clone().expect("clone"))
            .expect("(ddd)"),
        fractions(Accent::Ultramarine.color(Scheme::Light))
    );
    assert_eq!(
        String::try_from(seen["icon-theme"].try_clone().expect("clone")).expect("s"),
        "Unbound"
    );
}

#[test]
fn gtk_apps_get_the_accent_and_frame_tones_through_an_import_in_their_own_stylesheet() {
    let fixture = start("appearance \"dark\"\naccent \"plum\"\n");
    let css = fixture.stylesheet();
    let unconditional = css.split("@media").next().unwrap_or_default();
    assert!(unconditional.contains(&hex(Palette::for_scheme(Scheme::Dark).chassis)));
    assert!(!unconditional.contains(&hex(Palette::for_scheme(Scheme::Light).chassis)));
    for scheme in [Scheme::Light, Scheme::Dark] {
        assert!(css.contains(&hex(Accent::Plum.color(scheme))), "{css}");
        assert!(
            css.contains(&hex(Palette::for_scheme(scheme).chassis)),
            "{css}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(fixture.gtk_home.join("gtk-4.0/gtk.css")).expect("gtk.css"),
        "@import url(\"unbound.css\");\n"
    );

    fixture.write_config("appearance \"dark\"\naccent \"moss\"\n");
    let deadline = Instant::now() + WAIT;
    while !fixture
        .stylesheet()
        .contains(&hex(Accent::Moss.color(Scheme::Dark)))
    {
        assert!(
            Instant::now() < deadline,
            "the stylesheet follows the accent"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_users_own_gtk_stylesheet_keeps_its_rules_below_the_import() {
    let user = "window { font-size: 2em; }\n";
    let imported = gtk::with_import(user).expect("adds the import");
    assert_eq!(imported, format!("@import url(\"unbound.css\");\n{user}"));
    assert_eq!(gtk::with_import(&imported), None);
}

#[test]
fn the_sound_effects_switch_drives_gtk_event_sounds() {
    let fixture = start("sound {\n    effects #false\n}\n");
    let all = fixture.read_all(&["org.gnome.desktop.sound"]);
    assert!(!bool::try_from(&all["org.gnome.desktop.sound"]["event-sounds"]).expect("b"));
}

#[test]
fn a_tinted_frame_serves_the_wallpapers_average_colour() {
    let directory = tempfile::tempdir().expect("temp dir");
    let wallpaper = directory.path().join("plain.svg");
    std::fs::write(
        &wallpaper,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\"><rect width=\"64\" height=\"64\" fill=\"#336699\"/></svg>",
    )
    .expect("svg");
    let config = format!(
        "appearance \"light\"\nwallpaper {{\n    light \"{}\"\n}}\n",
        wallpaper.display()
    );
    let fixture = start(&config);
    let tint = |fixture: &Fixture| {
        let all = fixture.read_all(&[UNBOUND_APPEARANCE]);
        Vec::<f64>::try_from(
            all[UNBOUND_APPEARANCE]["frame-tint"]
                .try_clone()
                .expect("clone"),
        )
        .expect("ad")
    };
    assert!(tint(&fixture).is_empty());

    let changes = fixture.changes();
    fixture.write_config(&format!("{config}tinted-frame #true\n"));
    let (namespace, key, _) = changes.recv_timeout(WAIT).expect("SettingChanged");
    assert_eq!(
        (namespace.as_str(), key.as_str()),
        (UNBOUND_APPEARANCE, "frame-tint")
    );
    let served = tint(&fixture);
    assert_eq!(served.len(), 3);
    for (served, expected) in served.iter().zip([0x33, 0x66, 0x99]) {
        let expected = f64::from(expected) / 255.0;
        assert!((served - expected).abs() < 0.01, "{served} vs {expected}");
    }
}
