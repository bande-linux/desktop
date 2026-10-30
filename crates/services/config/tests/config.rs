#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use unbound_config::{
    Appearance, Config, ConfigError, ConfigStore, DEFAULT_DOCK_APPS, DockEdge, Output, ReloadError,
    Suspend, WallpaperFit, WallpaperVariant, config_path,
};

fn error_at(source: &str) -> (usize, usize) {
    let ConfigError { line, column, .. } = Config::parse(source).expect_err("invalid config");
    (line, column)
}

#[test]
fn an_empty_file_gives_the_plan_defaults() {
    let config = Config::parse("").expect("empty config");
    assert_eq!(config.dock.edge, DockEdge::Left);
    assert_eq!(config.dock.apps, DEFAULT_DOCK_APPS);
    assert_eq!(config.idle.dim_after, Duration::from_secs(5 * 60));
    assert_eq!(config.idle.lock_after, Duration::from_secs(10 * 60));
    assert!(config.outputs.is_empty());
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
output "eDP-1" {
    scale 1.5
    refresh-rate 120
    variable-refresh-rate #true
}
output "HDMI-A-1" {
    scale 1
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
        config.outputs.get("eDP-1"),
        Some(&Output {
            scale: Some(1.5),
            refresh_rate: Some(120.0),
            variable_refresh_rate: Some(true),
        })
    );
    assert_eq!(
        config.outputs.get("HDMI-A-1"),
        Some(&Output {
            scale: Some(1.0),
            ..Output::default()
        })
    );
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
fn display_scale_must_stay_within_100_to_200_percent() {
    assert!(Config::parse("output \"eDP-1\" {\n    scale 2.0\n}\n").is_ok());
    assert_eq!(error_at("output \"eDP-1\" {\n    scale 2.5\n}\n"), (2, 11));
    assert_eq!(error_at("output \"eDP-1\" {\n    scale 0.75\n}\n"), (2, 11));
}

#[test]
fn refresh_rate_must_stay_within_60_to_240_hz() {
    assert!(Config::parse("output \"DP-1\" {\n    refresh-rate 240\n}\n").is_ok());
    assert_eq!(
        error_at("output \"DP-1\" {\n    refresh-rate 30\n}\n"),
        (2, 18)
    );
    assert_eq!(
        error_at("output \"DP-1\" {\n    refresh-rate 360\n}\n"),
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
    assert_eq!(
        error_at("output \"eDP-1\" {\n}\noutput \"eDP-1\" {\n}\n"),
        (3, 1)
    );
}

#[test]
fn a_valid_file_replaces_the_active_configuration() {
    let store = ConfigStore::default();
    let applied = store.apply("dock {\n    edge \"top\"\n}\n").expect("valid");
    assert!(Arc::ptr_eq(&applied, &store.current()));
    assert_eq!(store.current().dock.edge, DockEdge::Top);
}

#[test]
fn an_invalid_file_leaves_the_active_configuration_untouched() {
    let store = ConfigStore::default();
    store.apply("appearance \"light\"\n").expect("valid");
    let before = store.current();

    let error = store
        .apply("appearance \"dark\"\ndock {\n    edge \"middle\"\n}\n")
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
    assert!(Config::parse("dock {\n    size 96\n}\n").is_ok());
    assert_eq!(error_at("dock {\n    auto-hide \"yes\"\n}\n"), (2, 15));
    assert_eq!(error_at("wallpaper {\n    fit \"tile\"\n}\n"), (2, 9));
}
