#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use bande_config::{ConfigService, ConfigStore};
use bande_datetime::daylight::{Sky, place_of, sky};
use bande_datetime::timedate::TimedateProxyBlocking;
use bande_design::{Accent, Color, Contrast, Palette, Scheme};
use bande_test_bus::timedate::FakeTimedate;
use common::Portal;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use xdg_desktop_portal_bande::gtk;
use xdg_desktop_portal_bande::settings::{BANDE_APPEARANCE, CursorSettings, Settings};
use zbus::blocking::MessageIterator;
use zbus::zvariant::OwnedValue;
use zbus::{MatchRule, message};

const INTERFACE: &str = "org.freedesktop.impl.portal.Settings";
const APPEARANCE: &str = "org.freedesktop.appearance";
const GNOME_INTERFACE: &str = "org.gnome.desktop.interface";
const WAIT: Duration = Duration::from_secs(5);
const AROUND_THE_WORLD: [&str; 8] = [
    "Europe/London",
    "Asia/Kolkata",
    "Asia/Shanghai",
    "Australia/Sydney",
    "Pacific/Auckland",
    "America/Los_Angeles",
    "America/New_York",
    "America/Sao_Paulo",
];

type All = HashMap<String, HashMap<String, OwnedValue>>;

struct Fixture {
    portal: Portal,
    path: PathBuf,
    gtk_home: PathBuf,
    data_home: PathBuf,
    _config: ConfigService,
    _directory: tempfile::TempDir,
}

fn start(config: &str) -> Fixture {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("config.kdl");
    std::fs::write(&path, config).expect("config");
    let gtk_home = directory.path().join("config-home");
    let data_home = directory.path().join("data-home");
    let mut service = None;
    let portal = Portal::start(|connection| {
        service = Some(
            ConfigService::start(connection, &path, Arc::new(ConfigStore::default()), |_| {})
                .expect("config service"),
        );
        Settings::serve(
            connection,
            Some(connection),
            CursorSettings {
                theme: "Adwaita".to_owned(),
            },
            Some(gtk::GtkHomes {
                config: gtk_home.clone(),
                data: data_home.clone(),
            }),
        )
        .expect("settings");
    });
    Fixture {
        portal,
        path,
        gtk_home,
        data_home,
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
        std::fs::read_to_string(self.gtk_home.join("gtk-4.0/bande.css")).unwrap_or_default()
    }

    fn gtk3_stylesheet(&self) -> String {
        std::fs::read_to_string(self.gtk_home.join("gtk-3.0/bande.css")).unwrap_or_default()
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
    let color = Palette::for_scheme(scheme).accent_text;
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
            Some(xdg_desktop_portal_bande::BUS_NAME),
            xdg_desktop_portal_bande::OBJECT_PATH,
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
    let bande = &all[BANDE_APPEARANCE];
    assert_eq!(u32::try_from(&bande["reduce-transparency"]).expect("u"), 1);
    assert_eq!(u32::try_from(&bande["text-size"]).expect("u"), 150);
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
fn gtk_plays_its_event_sounds_from_the_bande_sound_theme() {
    let fixture = start("");
    let all = fixture.read_all(&["org.gnome.desktop.sound"]);
    let sound = &all["org.gnome.desktop.sound"];

    assert_eq!(
        String::try_from(sound["theme-name"].try_clone().expect("clone")).expect("s"),
        "Bande"
    );
    assert!(bool::try_from(&sound["event-sounds"]).expect("b"));
}

fn gnome_changes(
    changes: &Receiver<(String, String, OwnedValue)>,
    count: usize,
) -> Vec<(String, String)> {
    let mut seen = Vec::new();
    while seen.len() < count {
        let (namespace, key, value) = changes.recv_timeout(WAIT).expect("SettingChanged");
        if namespace == GNOME_INTERFACE {
            let value = String::try_from(value.try_clone().expect("clone"))
                .or_else(|_| bool::try_from(&value).map(|on| on.to_string()))
                .expect("s or b");
            seen.push((key, value));
        }
    }
    seen
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn switching_to_light_signals_the_gnome_color_scheme_and_gtk_theme() {
    let fixture = start("appearance \"dark\"\n");
    let changes = fixture.changes();
    fixture.write_config("appearance \"light\"\n");

    assert_eq!(
        gnome_changes(&changes, 2),
        pairs(&[("color-scheme", "prefer-light"), ("gtk-theme", "Bande")])
    );
}

#[test]
fn gtk_apps_get_the_bande_fonts_theme_and_motion() {
    let fixture = start("appearance \"dark\"\n");
    let interface = |key: &str| fixture.string(GNOME_INTERFACE, key);
    assert_eq!(interface("gtk-theme"), "Bande-dark");
    assert_eq!(interface("font-name"), "Inter 10");
    assert_eq!(interface("document-font-name"), "Inter 11");
    assert_eq!(interface("monospace-font-name"), "JetBrains Mono 10");
    let animations = || {
        bool::try_from(&fixture.read_all(&[GNOME_INTERFACE])[GNOME_INTERFACE]["enable-animations"])
            .expect("b")
    };
    assert!(animations());

    let changes = fixture.changes();
    fixture.write_config(
        "appearance \"dark\"\naccessibility {\n    reduce-motion #true\n    increase-contrast #true\n}\n",
    );
    assert_eq!(
        gnome_changes(&changes, 2),
        pairs(&[
            ("gtk-theme", "HighContrastInverse"),
            ("enable-animations", "false")
        ])
    );
    assert!(!animations());
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
    assert_eq!(fixture.string(GNOME_INTERFACE, "icon-theme"), "Bande-Teal");

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
        "Bande"
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
        let palette = Palette::generate(scheme, Accent::Plum, Contrast::Standard);
        for color in [
            palette.chassis,
            palette.sheet,
            palette.accent,
            palette.accent_text,
        ] {
            assert!(css.contains(&hex(color)), "{css}");
        }
    }
    let dark = Palette::generate(Scheme::Dark, Accent::Plum, Contrast::Standard);
    let gtk3 = fixture.gtk3_stylesheet();
    for (name, color) in [
        ("theme_bg_color", dark.chassis),
        ("theme_base_color", dark.sheet),
        ("theme_selected_bg_color", dark.accent),
    ] {
        assert!(
            gtk3.contains(&format!("@define-color {name} {};", hex(color))),
            "{gtk3}"
        );
    }
    for version in ["gtk-3.0", "gtk-4.0"] {
        assert_eq!(
            std::fs::read_to_string(fixture.gtk_home.join(version).join("gtk.css"))
                .expect("gtk.css"),
            "@import url(\"bande.css\");\n"
        );
    }

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
    assert_eq!(imported, format!("@import url(\"bande.css\");\n{user}"));
    assert_eq!(gtk::with_import(&imported), None);
}

#[test]
fn a_linked_gtk_stylesheet_gets_the_import_through_its_link() {
    let homes = tempfile::tempdir().expect("homes");
    let config = homes.path().join("config");
    let dotfiles = homes.path().join("dotfiles");
    std::fs::create_dir_all(config.join("gtk-4.0")).expect("gtk-4.0");
    std::fs::create_dir(&dotfiles).expect("dotfiles");
    let user = "window { font-size: 2em; }\n";
    std::fs::write(dotfiles.join("gtk.css"), user).expect("user stylesheet");
    let link = config.join("gtk-4.0").join("gtk.css");
    std::os::unix::fs::symlink(dotfiles.join("gtk.css"), &link).expect("link");

    gtk::install(
        &gtk::GtkHomes {
            config,
            data: homes.path().join("data"),
        },
        Accent::Plum,
        Scheme::Dark,
    )
    .expect("install");

    assert!(
        std::fs::symlink_metadata(&link)
            .expect("link")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(dotfiles.join("gtk.css")).expect("read"),
        format!("@import url(\"bande.css\");\n{user}")
    );
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
        let all = fixture.read_all(&[BANDE_APPEARANCE]);
        Vec::<f64>::try_from(
            all[BANDE_APPEARANCE]["frame-tint"]
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
        (BANDE_APPEARANCE, "frame-tint")
    );
    let served = tint(&fixture);
    assert_eq!(served.len(), 3);
    for (served, expected) in served.iter().zip([0x33, 0x66, 0x99]) {
        let expected = f64::from(expected) / 255.0;
        assert!((served - expected).abs() < 0.01, "{served} vs {expected}");
    }
}

fn color_scheme_in(zone: &str) -> u32 {
    let zone = TimeZone::get(zone).expect("zone");
    let now = Timestamp::now().to_zoned(zone.clone());
    match sky(&now, place_of(&zone)).expect("sky").0 {
        Sky::Day => 2,
        Sky::Night => 1,
    }
}

#[test]
fn a_new_time_zone_replans_the_automatic_appearance_at_once() {
    let fixture = start("appearance \"automatic\"\n");
    let _timedated = FakeTimedate::start(&fixture.portal.bus, "Europe/Stockholm").expect("fake");
    let shown = fixture.color_scheme();
    let (zone, wanted) = AROUND_THE_WORLD
        .into_iter()
        .map(|zone| (zone, color_scheme_in(zone)))
        .find(|(_, scheme)| *scheme != shown)
        .expect("a zone on the other side of the day");
    let changes = fixture.changes();

    TimedateProxyBlocking::new(&fixture.portal.client)
        .expect("timedate")
        .set_timezone(zone, true)
        .expect("SetTimezone");

    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let (namespace, key, value) = changes.recv_timeout(left).expect("SettingChanged");
        if (namespace.as_str(), key.as_str()) == (APPEARANCE, "color-scheme") {
            assert_eq!(u32::try_from(&value).expect("u"), wanted, "in {zone}");
            break;
        }
    }
    assert_eq!(fixture.color_scheme(), wanted);
}

#[test]
fn gtk_puts_the_window_controls_on_the_left_in_our_order() {
    let fixture = start("");

    assert_eq!(
        fixture.string("org.gnome.desktop.wm.preferences", "button-layout"),
        "close,minimize,maximize:"
    );
}

fn scheme_block(css: &str, scheme: &str) -> String {
    css.split(&format!("@media (prefers-color-scheme: {scheme})"))
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn gtk_window_controls_take_the_colours_of_our_title_bar_in_each_scheme() {
    let css = gtk::stylesheet(Accent::Plum, Scheme::Light);
    for (query, scheme) in [("light", Scheme::Light), ("dark", Scheme::Dark)] {
        let palette = Palette::generate(scheme, Accent::Plum, Contrast::Standard);
        let block = scheme_block(&css, query);
        for (name, color) in [
            ("bande-well", palette.well),
            ("bande-hairline", palette.hairline),
            ("bande-ink", palette.ink),
            ("bande-ink2", palette.ink2),
            ("bande-ink3", palette.ink3),
            ("bande-danger", palette.danger),
        ] {
            assert!(
                block.contains(&format!("--{name}: {};", hex(color))),
                "{query}: {block}"
            );
        }
    }
    assert!(css.contains("windowcontrols {"));
    assert!(css.contains("font-family: \"Manrope\";\n  font-size: 15px;\n  font-weight: 700;"));
}

#[test]
fn gtk3_apps_get_a_bande_theme_per_scheme_on_top_of_adwaita() {
    let fixture = start("appearance \"light\"\naccent \"teal\"\n");
    for (name, scheme, adwaita) in [
        ("Bande", Scheme::Light, "Adwaita/gtk-contained.css"),
        ("Bande-dark", Scheme::Dark, "Adwaita/gtk-contained-dark.css"),
    ] {
        let theme = fixture.data_home.join("themes").join(name);
        let css = std::fs::read_to_string(theme.join("gtk-3.0/gtk.css")).expect("gtk.css");
        let palette = Palette::generate(scheme, Accent::Teal, Contrast::Standard);
        assert!(css.starts_with(&format!(
            "@import url(\"resource:///org/gtk/libgtk/theme/{adwaita}\");"
        )));
        for color in [
            palette.chassis,
            palette.well,
            palette.accent,
            palette.danger,
        ] {
            assert!(css.contains(&hex(color)), "{name}: {css}");
        }
        assert!(
            std::fs::read_to_string(theme.join("index.theme"))
                .expect("index.theme")
                .contains(&format!("GtkTheme={name}"))
        );
        assert!(theme.join("gtk-4.0/gtk.css").exists());
    }
}
