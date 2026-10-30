#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use unbound_config::{
    AlertSound, Appearance, CapsLock, Config, ConfigError, ConfigStore, DEFAULT_DOCK_APPS,
    DisplaySettings, DockEdge, InputSource, Placement, ReloadError, Rotation, SecondaryClick,
    Suspend, WallpaperFit, WallpaperVariant, config_path, login_picture, system_input_sources,
};

fn problems(source: &str) -> Vec<ConfigError> {
    Config::load(source).expect("well-formed KDL").1
}

fn error_at(source: &str) -> (usize, usize) {
    let ConfigError { line, column, .. } = match Config::load(source) {
        Ok((_, problems)) => problems.into_iter().next().expect("a problem"),
        Err(error) => error,
    };
    (line, column)
}

const OWNERS_FILE: &str = "appearance \"dark\"
output \"Virtual-1\" {
    scale 1.5
}
dock {
    size 45
}
";

#[test]
fn a_file_from_before_displays_were_named_still_applies_every_setting() {
    let (config, problems) = Config::load(OWNERS_FILE).expect("well-formed KDL");

    assert_eq!(problems, []);
    assert_eq!(config.appearance, Appearance::Dark);
    assert_eq!(config.dock.icon_size, 45);
    assert_eq!(config.display("Virtual-1", "Virtual-1").scale, Some(1.5));
    assert_eq!(
        config
            .display("Dell DELL U2720Q 7ZD5K53", "Virtual-1")
            .scale,
        Some(1.5)
    );
    assert_eq!(
        config.display("Dell DELL U2720Q 7ZD5K53", "DP-1"),
        DisplaySettings::default()
    );
}

#[test]
fn a_named_display_wins_over_its_old_connector_block() {
    let config = Config::parse(
        "output \"DP-1\" {\n    scale 2\n}\ndisplay \"Dell U2720Q\" {\n    scale 1.5\n}\n",
    )
    .expect("well-formed KDL");

    assert_eq!(config.display("Dell U2720Q", "DP-1").scale, Some(1.5));
    assert_eq!(config.display("LG 27UK850", "DP-1").scale, Some(2.0));
}

#[test]
fn the_old_panel_section_is_read_as_the_dock() {
    let (config, problems) = Config::load("panel {\n    edge \"top\"\n}\n").expect("KDL");

    assert_eq!(problems, []);
    assert_eq!(config.dock.edge, DockEdge::Top);
    assert_eq!(error_at("dock {\n}\npanel {\n}\n"), (3, 1));
}

#[test]
fn unknown_and_invalid_settings_are_reported_while_every_valid_one_applies() {
    let (config, problems) = Config::load(
        "appearance \"dark\"\ntheme \"graphite\"\ndock {\n    edge \"middle\"\n    size 64\n    sparkle #true\n}\nidle {\n    dim-after \"10m\"\n    lock-after \"5m\"\n}\nkeyboard {\n    repeat-rate 40\n}\n",
    )
    .expect("well-formed KDL");

    assert_eq!(
        problems
            .iter()
            .map(|problem| (problem.line, problem.column))
            .collect::<Vec<_>>(),
        [(2, 1), (4, 10), (6, 5), (10, 5)]
    );
    assert_eq!(config.appearance, Appearance::Dark);
    assert_eq!(config.dock.edge, DockEdge::Left);
    assert_eq!(config.dock.icon_size, 64);
    assert_eq!(config.idle.dim_after, Duration::from_secs(10 * 60));
    assert_eq!(config.idle.lock_after, Duration::from_secs(10 * 60));
    assert_eq!(config.keyboard.repeat_rate, 40);
}

#[test]
fn an_empty_file_gives_the_plan_defaults() {
    let config = Config::parse("").expect("empty config");
    assert_eq!(config.dock.edge, DockEdge::Left);
    assert_eq!(config.dock.apps, DEFAULT_DOCK_APPS);
    assert_eq!(config.idle.dim_after, Duration::from_secs(5 * 60));
    assert_eq!(config.idle.lock_after, Duration::from_secs(10 * 60));
    assert!(config.displays.is_empty());
    assert!(config.arrangements.is_empty());
}

#[test]
fn every_setting_is_read() {
    let config = Config::parse(
        r#"
appearance "dark"
dock {
    edge "bottom"
    apps "org.unbound.Terminal" "org.mozilla.firefox"
}
idle {
    dim-after "90s"
    lock-after "1h"
}
display "BOE NE135A1M 0" {
    resolution 2256 1504
    scale 1.5
    refresh-rate 120
    rotation 90
    variable-refresh-rate #true
}
display "Dell Inc. DELL U2720Q 7ZD5K53" {
    scale 1
}
arrangement {
    display "BOE NE135A1M 0" x=0 y=0 main=#true
    display "Dell Inc. DELL U2720Q 7ZD5K53" x=1504 y=-300
}
arrangement {
    display "BOE NE135A1M 0"
}
"#,
    )
    .expect("valid config");

    assert_eq!(config.appearance, Appearance::Dark);
    assert_eq!(config.dock.edge, DockEdge::Bottom);
    assert_eq!(
        config.dock.apps,
        ["org.unbound.Terminal", "org.mozilla.firefox"]
    );
    assert_eq!(config.idle.dim_after, Duration::from_secs(90));
    assert_eq!(config.idle.lock_after, Duration::from_secs(3600));
    assert_eq!(
        config.displays.get("BOE NE135A1M 0"),
        Some(&DisplaySettings {
            resolution: Some((2256, 1504)),
            scale: Some(1.5),
            refresh_rate: Some(120.0),
            rotation: Some(Rotation::Quarter),
            variable_refresh_rate: Some(true),
        })
    );
    assert_eq!(
        config.displays.get("Dell Inc. DELL U2720Q 7ZD5K53"),
        Some(&DisplaySettings {
            scale: Some(1.0),
            ..DisplaySettings::default()
        })
    );
    let both = [
        "Dell Inc. DELL U2720Q 7ZD5K53".to_owned(),
        "BOE NE135A1M 0".to_owned(),
    ];
    assert_eq!(
        config.arrangement(&both),
        Some(
            &[
                Placement {
                    display: "BOE NE135A1M 0".to_owned(),
                    x: 0,
                    y: 0,
                    main: true,
                    mirror: None,
                },
                Placement {
                    display: "Dell Inc. DELL U2720Q 7ZD5K53".to_owned(),
                    x: 1504,
                    y: -300,
                    main: false,
                    mirror: None,
                },
            ][..]
        )
    );
    assert_eq!(
        config
            .arrangement(&["BOE NE135A1M 0".to_owned()])
            .map(<[Placement]>::len),
        Some(1)
    );
}

#[test]
fn a_mirror_must_name_another_display_of_its_arrangement() {
    assert_eq!(
        problems(
            "arrangement {\n    display \"A\" main=#true\n    display \"B\" mirror=\"A\"\n}\n"
        ),
        []
    );
    assert_eq!(
        error_at("arrangement {\n    display \"A\"\n    display \"B\" mirror=\"C\"\n}\n"),
        (1, 1)
    );
    assert_eq!(
        error_at("arrangement {\n    display \"A\" main=#true\n    display \"B\" main=#true\n}\n"),
        (1, 1)
    );
    assert_eq!(error_at("display \"A\" {\n    rotation 45\n}\n"), (2, 14));
}

#[test]
fn the_dock_accepts_each_of_the_four_edges() {
    for (name, edge) in [
        ("left", DockEdge::Left),
        ("bottom", DockEdge::Bottom),
        ("right", DockEdge::Right),
        ("top", DockEdge::Top),
    ] {
        let config = Config::parse(&format!("dock {{\n    edge \"{name}\"\n}}\n"));
        assert_eq!(config.map(|config| config.dock.edge), Ok(edge));
    }
}

#[test]
fn appearance_accepts_dark_light_and_automatic() {
    for (name, appearance) in [
        ("dark", Appearance::Dark),
        ("light", Appearance::Light),
        ("automatic", Appearance::Automatic),
    ] {
        let config = Config::parse(&format!("appearance \"{name}\"\n"));
        assert_eq!(config.map(|config| config.appearance), Ok(appearance));
    }
}

#[test]
fn a_syntax_error_reports_its_line_and_column() {
    assert_eq!(error_at("dock {\n    edge \"left\n}\n"), (2, 10));
    assert_eq!(
        error_at("appearance \"dark\"\ndock {\n    edge \"left\"\n"),
        (2, 6)
    );
}

#[test]
fn an_unknown_setting_is_reported_where_it_starts() {
    assert_eq!(
        error_at("appearance \"dark\"\n  theme \"graphite\"\n"),
        (2, 3)
    );
    assert_eq!(error_at("dock {\n    autohide #true\n}\n"), (2, 5));
    assert_eq!(error_at("dock {\n    apps \"a.b\" \"a.b\"\n}\n"), (2, 16));
    assert_eq!(error_at("dock {\n    apps 3\n}\n"), (2, 10));
}

#[test]
fn a_value_outside_its_choices_is_reported_at_the_value() {
    assert_eq!(error_at("dock {\n    edge \"middle\"\n}\n"), (2, 10));
    assert_eq!(error_at("appearance \"sepia\"\n"), (1, 12));
}

#[test]
fn display_scale_must_stay_within_100_to_300_percent() {
    assert_eq!(problems("display \"A\" {\n    scale 2.5\n}\n"), []);
    assert_eq!(error_at("display \"A\" {\n    scale 3.5\n}\n"), (2, 11));
    assert_eq!(error_at("display \"A\" {\n    scale 0.75\n}\n"), (2, 11));
}

#[test]
fn refresh_rate_must_stay_within_60_to_240_hz() {
    assert_eq!(problems("display \"A\" {\n    refresh-rate 240\n}\n"), []);
    assert_eq!(
        error_at("display \"A\" {\n    refresh-rate 30\n}\n"),
        (2, 18)
    );
    assert_eq!(
        error_at("display \"A\" {\n    refresh-rate 360\n}\n"),
        (2, 18)
    );
}

#[test]
fn key_repeat_defaults_to_25_per_second_after_600_ms() {
    let keyboard = Config::parse("").expect("empty").keyboard;
    assert_eq!(keyboard.repeat_rate, 25);
    assert_eq!(keyboard.repeat_delay, Duration::from_millis(600));
    let keyboard = Config::parse("keyboard {\n    repeat-rate 40\n    repeat-delay 250\n}\n")
        .expect("valid")
        .keyboard;
    assert_eq!(keyboard.repeat_rate, 40);
    assert_eq!(keyboard.repeat_delay, Duration::from_millis(250));
}

#[test]
fn key_repeat_must_stay_within_usable_limits() {
    for (setting, value) in [
        ("repeat-rate", "0"),
        ("repeat-rate", "61"),
        ("repeat-rate", "2.5"),
        ("repeat-delay", "100"),
        ("repeat-delay", "2001"),
        ("repeat-delay", "\"600ms\""),
    ] {
        let source = format!("keyboard {{\n    {setting} {value}\n}}\n");
        assert_eq!(
            error_at(&source),
            (2, 6 + setting.len()),
            "{setting} {value}"
        );
    }
}

#[test]
fn wallpapers_are_absolute_or_home_relative_paths() {
    let wallpaper = Config::parse(
        "wallpaper {\n    light \"/usr/share/backgrounds/day.jpg\"\n    dark \"~/Pictures/night.png\"\n}\n",
    )
    .expect("valid")
    .wallpaper;
    assert_eq!(
        wallpaper.path(WallpaperVariant::Light),
        Some(Path::new("/usr/share/backgrounds/day.jpg"))
    );
    assert_eq!(
        wallpaper.path(WallpaperVariant::Dark),
        Some(Path::new("~/Pictures/night.png"))
    );
    assert_eq!(Config::parse("").expect("empty").wallpaper.light, None);
    for path in ["night.png", "~", "", "~other/x.png"] {
        let source = format!("wallpaper {{\n    dark \"{path}\"\n}}\n");
        assert_eq!(error_at(&source), (2, 10), "path {path:?}");
    }
}

#[test]
fn the_screen_must_dim_before_it_locks() {
    assert_eq!(
        error_at("idle {\n    dim-after \"10m\"\n    lock-after \"5m\"\n}\n"),
        (3, 5)
    );
}

#[test]
fn durations_need_a_positive_amount_and_a_unit() {
    for duration in ["5", "0m", "m", "-5m", "5 m", "1d"] {
        let source = format!("idle {{\n    dim-after \"{duration}\"\n}}\n");
        assert_eq!(error_at(&source), (2, 15), "duration {duration:?}");
    }
}

#[test]
fn a_setting_given_twice_is_reported_at_the_second_one() {
    assert_eq!(
        error_at("appearance \"dark\"\nappearance \"light\"\n"),
        (2, 1)
    );
    assert_eq!(error_at("display \"A\" {\n}\ndisplay \"A\" {\n}\n"), (3, 1));
}

#[test]
fn a_valid_file_replaces_the_active_configuration() {
    let store = ConfigStore::default();
    let applied = store.apply("dock {\n    edge \"top\"\n}\n").expect("valid");
    assert!(Arc::ptr_eq(&applied.config, &store.current()));
    assert_eq!(store.current().dock.edge, DockEdge::Top);
}

#[test]
fn an_unreadable_file_leaves_the_active_configuration_untouched() {
    let store = ConfigStore::default();
    store.apply("appearance \"light\"\n").expect("valid");
    let before = store.current();

    let error = store
        .apply("appearance \"dark\"\ndock {\n    edge \"middle\n}\n")
        .expect_err("invalid");

    assert_eq!((error.line, error.column), (3, 10));
    assert!(Arc::ptr_eq(&before, &store.current()));
    assert_eq!(store.current().appearance, Appearance::Light);
}

#[test]
fn readers_never_observe_a_partially_applied_configuration() {
    let first = "appearance \"dark\"\ndock {\n    edge \"top\"\n}\n";
    let second = "appearance \"light\"\ndock {\n    edge \"bottom\"\n}\n";
    let store = ConfigStore::new(Config::parse(first).expect("valid"));
    let done = AtomicBool::new(false);

    thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    let config = store.current();
                    let pair = (config.appearance, config.dock.edge);
                    assert!(
                        pair == (Appearance::Dark, DockEdge::Top)
                            || pair == (Appearance::Light, DockEdge::Bottom),
                        "torn configuration {pair:?}"
                    );
                }
            });
        }
        for round in 0..2000 {
            let source = if round % 2 == 0 { second } else { first };
            store.apply(source).expect("valid");
        }
        done.store(true, Ordering::Relaxed);
    });
}

#[test]
fn reloading_reads_the_file_and_keeps_the_old_configuration_on_failure() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("config.kdl");
    let store = ConfigStore::default();

    assert!(matches!(store.reload(&path), Err(ReloadError::Read(_))));
    assert_eq!(*store.current(), Config::default());

    std::fs::write(&path, "dock {\n    edge \"right\"\n}\n").expect("write");
    store.reload(&path).expect("valid file");
    assert_eq!(store.current().dock.edge, DockEdge::Right);

    std::fs::write(&path, "dock {\n    edge \"right\"\n").expect("write");
    assert!(matches!(store.reload(&path), Err(ReloadError::Invalid(_))));
    assert_eq!(store.current().dock.edge, DockEdge::Right);
}

#[test]
fn the_file_lives_under_the_xdg_config_home() {
    let home = Path::new("/home/ada");
    assert_eq!(
        config_path(None, Some(home)),
        Some(Path::new("/home/ada/.config/unbound/config.kdl").to_path_buf())
    );
    assert_eq!(
        config_path(Some(Path::new("/srv/config")), Some(home)),
        Some(Path::new("/srv/config/unbound/config.kdl").to_path_buf())
    );
    assert_eq!(
        config_path(Some(Path::new("relative/config")), Some(home)),
        Some(Path::new("/home/ada/.config/unbound/config.kdl").to_path_buf())
    );
    assert_eq!(config_path(None, None), None);
}

#[test]
fn the_computer_sleeps_when_the_display_turns_off_unless_told_otherwise() {
    let default = Config::parse("").expect("valid");
    assert_eq!(default.idle.suspend, Suspend::WithDisplayOff);
    assert_eq!(
        default.idle.suspend_after(),
        Some(Duration::from_secs(10 * 60))
    );

    let later_lock = Config::parse("idle {\n    lock-after \"20m\"\n}\n").expect("valid");
    assert_eq!(
        later_lock.idle.suspend_after(),
        Some(Duration::from_secs(20 * 60))
    );

    let explicit = Config::parse("idle {\n    suspend-after \"1h\"\n}\n").expect("valid");
    assert_eq!(
        explicit.idle.suspend_after(),
        Some(Duration::from_secs(3600))
    );

    let never = Config::parse("idle {\n    suspend-after \"never\"\n}\n").expect("valid");
    assert_eq!(never.idle.suspend_after(), None);
}

#[test]
fn the_computer_must_not_sleep_before_the_display_turns_off() {
    assert_eq!(error_at("idle {\n    suspend-after \"5m\"\n}\n"), (2, 5));
    assert_eq!(error_at("idle {\n    suspend-after \"soon\"\n}\n"), (2, 19));
}

#[test]
fn an_empty_dock_apps_list_empties_the_dock() {
    let config = Config::parse("dock {\n    apps\n}\n").expect("valid config");
    assert!(config.dock.apps.is_empty());
}

#[test]
fn the_dock_size_auto_hide_wallpaper_fit_and_reduce_motion_are_read() {
    let config = Config::parse(
        "accessibility {\n    reduce-motion #true\n}\ndock {\n    auto-hide #true\n    size 64\n}\nwallpaper {\n    fit \"fit\"\n}\n",
    )
    .expect("valid config");
    assert!(config.reduce_motion);
    assert!(config.dock.auto_hide);
    assert_eq!(config.dock.icon_size, 64);
    assert_eq!(config.wallpaper.fit, WallpaperFit::Fit);

    let defaults = Config::parse("").expect("empty config");
    assert!(!defaults.reduce_motion && !defaults.dock.auto_hide);
    assert_eq!(defaults.dock.icon_size, 48);
    assert_eq!(defaults.wallpaper.fit, WallpaperFit::Fill);
}

#[test]
fn the_dock_size_stays_within_24_to_96_points_and_flags_need_a_boolean() {
    assert_eq!(error_at("dock {\n    size 23\n}\n"), (2, 10));
    assert_eq!(error_at("dock {\n    size 97\n}\n"), (2, 10));
    assert_eq!(problems("dock {\n    size 96\n}\n"), []);
    assert_eq!(error_at("dock {\n    auto-hide \"yes\"\n}\n"), (2, 15));
    assert_eq!(error_at("wallpaper {\n    fit \"tile\"\n}\n"), (2, 9));
}

#[test]
fn the_login_window_gets_the_dark_wallpaper_with_the_home_folder_spelled_out() {
    let home = Path::new("/home/me");
    let wallpaper = |source: &str| Config::parse(source).expect("valid").wallpaper;
    assert_eq!(
        login_picture(
            &wallpaper("wallpaper {\n    light \"/a.png\"\n    dark \"~/b.png\"\n}\n"),
            Some(home)
        ),
        Some(PathBuf::from("/home/me/b.png"))
    );
    assert_eq!(
        login_picture(
            &wallpaper("wallpaper {\n    light \"/a.png\"\n}\n"),
            Some(home)
        ),
        Some(PathBuf::from("/a.png"))
    );
    assert_eq!(login_picture(&wallpaper(""), Some(home)), None);
}

fn source(layout: &str, variant: Option<&str>) -> InputSource {
    InputSource {
        layout: layout.to_owned(),
        variant: variant.map(str::to_owned),
    }
}

#[test]
fn input_sources_are_xkb_layouts_with_an_optional_variant() {
    let keyboard = Config::parse(
        "keyboard {\n    input-sources \"us\" \"de(nodeadkeys)\"\n    caps-lock \"escape\"\n}\n",
    )
    .expect("valid")
    .keyboard;
    assert_eq!(
        keyboard.input_sources,
        [source("us", None), source("de", Some("nodeadkeys"))]
    );
    assert_eq!(keyboard.caps_lock, CapsLock::Escape);
    assert_eq!(keyboard.caps_lock.xkb_option(), Some("caps:escape"));
    assert_eq!(source("de", Some("nodeadkeys")).code(), "de(nodeadkeys)");
}

#[test]
fn input_sources_must_be_distinct_layouts_and_at_most_four() {
    for (line, column, sources) in [
        (2, 24, "\"us\" \"us\""),
        (2, 19, "\"de(\""),
        (2, 19, "\"us,se\""),
        (2, 39, "\"us\" \"se\" \"de\" \"fr\" \"fi\""),
    ] {
        let config = format!("keyboard {{\n    input-sources {sources}\n}}\n");
        assert_eq!(error_at(&config), (line, column), "{sources}");
    }
}

#[test]
fn without_input_sources_the_system_keyboard_layouts_are_used() {
    let keyboard = Config::parse("").expect("empty").keyboard;
    let system = system_input_sources(
        "# KEYBOARD CONFIGURATION FILE\nXKBMODEL=\"pc105\"\nXKBLAYOUT=\"se,us,ru\"\nXKBVARIANT=\",dvorak,\"\nXKBOPTIONS=\"\"\n",
    );
    assert_eq!(
        keyboard.input_sources_or(|| system.clone()),
        [
            source("se", None),
            source("us", Some("dvorak")),
            source("ru", None)
        ]
    );
    assert_eq!(system_input_sources(""), [source("us", None)]);
}

#[test]
fn trackpad_and_mouse_settings_are_read_with_mac_like_defaults() {
    let defaults = Config::parse("").expect("empty");
    assert!(defaults.trackpad.tap_to_click);
    assert!(defaults.trackpad.natural_scrolling);
    assert!(!defaults.mouse.natural_scrolling);
    let config = Config::parse(
        "trackpad {\n    tap-to-click #false\n    speed 0.5\n    secondary-click \"corner\"\n    disable-while-typing #false\n}\nmouse {\n    natural-scrolling #true\n    speed -1\n}\n",
    )
    .expect("valid");
    assert!(!config.trackpad.tap_to_click);
    assert_eq!(config.trackpad.speed, 0.5);
    assert_eq!(config.trackpad.secondary_click, SecondaryClick::Corner);
    assert!(!config.trackpad.disable_while_typing);
    assert!(config.mouse.natural_scrolling);
    assert_eq!(config.mouse.speed, -1.0);
    assert_eq!(error_at("mouse {\n    speed 2\n}\n"), (2, 11));
}

#[test]
fn sound_settings_default_to_effects_and_feedback_on_and_reject_a_loud_alert() {
    let defaults = Config::parse("").expect("empty").sound;
    assert!(defaults.effects && defaults.volume_feedback);
    assert_eq!(defaults.alert, AlertSound::Mallet);
    let sound = Config::parse(
        "sound {\n    effects #false\n    volume-feedback #false\n    alert \"knock\"\n    alert-volume 0.4\n}\n",
    )
    .expect("valid")
    .sound;
    assert!(!sound.effects && !sound.volume_feedback);
    assert_eq!(sound.alert, AlertSound::Knock);
    assert_eq!(sound.alert_volume, 0.4);
    assert_eq!(error_at("sound {\n    alert-volume 2\n}\n"), (2, 18));
    assert_eq!(error_at("sound {\n    alert \"gong\"\n}\n"), (2, 11));
}

fn connector(drm: &Path, name: &str, status: &str, edid: &[u8]) {
    let folder = drm.join(name);
    std::fs::create_dir_all(&folder).expect("connector folder");
    std::fs::write(folder.join("status"), status).expect("status");
    std::fs::write(folder.join("edid"), edid).expect("edid");
}

#[test]
fn connected_displays_are_named_as_the_compositor_names_them() {
    let drm = tempfile::tempdir().expect("temp dir");
    let mut dell = vec![0u8; 128];
    dell[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
    dell[8..10].copy_from_slice(&[0x10, 0xac]);
    dell[72..77].copy_from_slice(&[0, 0, 0, 0xfc, 0]);
    dell[77..90].copy_from_slice(b"DELL P2419H\n ");
    connector(drm.path(), "card0-Virtual-1", "connected\n", &[]);
    connector(drm.path(), "card0-HDMI-A-1", "disconnected\n", &[]);
    connector(drm.path(), "card1-DP-1", "connected\n", &dell);
    connector(drm.path(), "card1-DP-2", "connected\n", &dell);
    std::fs::create_dir_all(drm.path().join("renderD128")).expect("render node");

    assert_eq!(
        unbound_config::connected_displays(drm.path()),
        [
            ("DP-1".to_owned(), "Dell DELL P2419H (DP-1)".to_owned()),
            ("DP-2".to_owned(), "Dell DELL P2419H (DP-2)".to_owned()),
            ("Virtual-1".to_owned(), "Virtual-1".to_owned()),
        ]
    );
}
