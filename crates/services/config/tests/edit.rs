#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::Duration;

use unbound_config::{Appearance, Config, DockEdge, Output, Setting, Suspend, WallpaperVariant};

fn apply(setting: Setting, source: &str) -> String {
    setting.apply(source).expect("edit")
}

#[test]
fn changing_a_value_keeps_every_comment_and_line_around_it() {
    let source = "// Unbound settings\nappearance \"dark\" // at night\n\ndock {\n    edge \"left\" // near the dock\n}\n";
    assert_eq!(
        apply(Setting::DockEdge(DockEdge::Top), source),
        "// Unbound settings\nappearance \"dark\" // at night\n\ndock {\n    edge \"top\" // near the dock\n}\n"
    );
}

#[test]
fn a_setting_missing_from_the_file_is_appended() {
    assert_eq!(
        apply(Setting::Appearance(Appearance::Light), ""),
        "appearance \"light\"\n"
    );
}

#[test]
fn a_missing_section_is_created_after_a_last_line_without_newline() {
    let edited = apply(
        Setting::IdleDimAfter(Duration::from_secs(90)),
        "appearance \"dark\"",
    );
    assert_eq!(
        edited,
        "appearance \"dark\"\nidle {\n    dim-after \"90s\"\n}\n"
    );
    assert_eq!(
        Config::parse(&edited).expect("valid").idle.dim_after,
        Duration::from_secs(90)
    );
}

#[test]
fn a_new_entry_in_a_section_follows_the_indentation_of_its_siblings() {
    assert_eq!(
        apply(
            Setting::IdleLockAfter(Duration::from_secs(7200)),
            "idle {\n  dim-after \"1m\"\n}\n"
        ),
        "idle {\n  dim-after \"1m\"\n  lock-after \"2h\"\n}\n"
    );
}

#[test]
fn durations_are_written_in_the_largest_whole_unit() {
    for (seconds, written) in [(45, "45s"), (300, "5m"), (3600, "1h"), (5400, "90m")] {
        let edited = apply(
            Setting::IdleLockAfter(Duration::from_secs(seconds)),
            "idle {\n    dim-after \"30s\"\n}\n",
        );
        assert!(
            edited.contains(&format!("lock-after \"{written}\"")),
            "{seconds} s written as {edited:?}"
        );
    }
}

#[test]
fn key_repeat_and_wallpapers_are_written_and_a_reset_wallpaper_leaves_no_trace() {
    let edited = apply(Setting::KeyRepeatRate(40), "");
    let edited = apply(Setting::KeyRepeatDelay(Duration::from_millis(300)), &edited);
    assert_eq!(
        edited,
        "keyboard {\n    repeat-rate 40\n    repeat-delay 300\n}\n"
    );
    let path = PathBuf::from("/home/me/Pictures/sea shore.jpg");
    let edited = apply(
        Setting::Wallpaper(WallpaperVariant::Dark, Some(path.clone())),
        "appearance \"dark\"\n",
    );
    assert_eq!(
        Config::parse(&edited).expect("valid").wallpaper.dark,
        Some(path)
    );
    assert_eq!(
        apply(Setting::Wallpaper(WallpaperVariant::Dark, None), &edited),
        "appearance \"dark\"\n"
    );
}

#[test]
fn a_value_that_breaks_a_rule_is_rejected() {
    let error = Setting::IdleDimAfter(Duration::from_secs(20 * 60))
        .apply("")
        .expect_err("dimming after the default lock time");
    assert!(error.message.contains("lock-after"), "{}", error.message);

    assert!(
        Setting::IdleDimAfter(Duration::ZERO).apply("").is_err(),
        "durations must be positive"
    );
}

#[test]
fn an_invalid_file_is_not_edited() {
    let error = Setting::Appearance(Appearance::Dark)
        .apply("appearance \"dark\"\ndock {\n    edge \"middle\"\n}\n")
        .expect_err("invalid file");
    assert_eq!((error.line, error.column), (3, 10));
}

#[test]
fn an_output_is_added_changed_and_removed_by_connector() {
    let added = apply(
        Setting::Output(
            "HDMI-A-1".to_owned(),
            Output {
                scale: Some(1.5),
                refresh_rate: Some(120.0),
                variable_refresh_rate: None,
            },
        ),
        "output \"eDP-1\" {\n    scale 2.0\n}\n",
    );
    assert_eq!(
        added,
        "output \"eDP-1\" {\n    scale 2.0\n}\noutput \"HDMI-A-1\" {\n    scale 1.5\n    refresh-rate 120\n}\n"
    );

    let changed = apply(
        Setting::Output(
            "HDMI-A-1".to_owned(),
            Output {
                scale: None,
                refresh_rate: Some(60.0),
                variable_refresh_rate: Some(true),
            },
        ),
        &added,
    );
    assert_eq!(
        changed,
        "output \"eDP-1\" {\n    scale 2.0\n}\noutput \"HDMI-A-1\" {\n    refresh-rate 60\n    variable-refresh-rate #true\n}\n"
    );

    let removed = apply(
        Setting::Output("HDMI-A-1".to_owned(), Output::default()),
        &changed,
    );
    assert_eq!(removed, "output \"eDP-1\" {\n    scale 2.0\n}\n");
}

#[test]
fn suspend_after_is_written_as_a_duration_or_never_or_left_to_follow_the_display() {
    assert_eq!(
        apply(Setting::IdleSuspend(Suspend::Never), ""),
        "idle {\n    suspend-after \"never\"\n}\n"
    );
    assert_eq!(
        apply(
            Setting::IdleSuspend(Suspend::After(Duration::from_secs(3600))),
            "idle {\n    suspend-after \"never\"\n}\n"
        ),
        "idle {\n    suspend-after \"1h\"\n}\n"
    );
    assert_eq!(
        apply(
            Setting::IdleSuspend(Suspend::WithDisplayOff),
            "idle {\n    dim-after \"1m\"\n    suspend-after \"1h\"\n}\n"
        ),
        "idle {\n    dim-after \"1m\"\n}\n"
    );
}
