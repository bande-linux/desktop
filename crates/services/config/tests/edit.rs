#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::Duration;

use unbound_config::{
    Accessibility, AlertSound, Appearance, BarEdge, CapsLock, ClockFormat, Config, DisplaySettings,
    InputSource, LidOnPower, Placement, Rotation, Setting, Suspend, WallpaperFit, WallpaperVariant,
    migrate,
};

fn apply(setting: Setting, source: &str) -> String {
    setting.apply(source).expect("edit")
}

#[test]
fn changing_a_value_keeps_every_comment_and_line_around_it() {
    let source = "// Unbound settings\nappearance \"dark\" // at night\n\nbar {\n    edge \"left\" // near the dock\n}\n";
    assert_eq!(
        apply(Setting::BarEdge(BarEdge::Top), source),
        "// Unbound settings\nappearance \"dark\" // at night\n\nbar {\n    edge \"top\" // near the dock\n}\n"
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
fn the_clock_format_is_written_and_automatic_leaves_no_trace() {
    let edited = apply(
        Setting::Clock(ClockFormat::TwentyFourHour),
        "appearance \"dark\"\n",
    );
    assert_eq!(edited, "appearance \"dark\"\nclock \"24-hour\"\n");
    assert_eq!(
        Config::parse(&edited).expect("valid").clock,
        ClockFormat::TwentyFourHour
    );
    assert_eq!(
        apply(Setting::Clock(ClockFormat::Automatic), &edited),
        "appearance \"dark\"\n"
    );
    assert_eq!(Config::load("clock \"25-hour\"").expect("KDL").1.len(), 1);
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
fn the_dock_apps_are_rewritten_in_place_keeping_comments() {
    let source =
        "bar {\n    edge \"left\" // near\n    apps \"org.unbound.Files\" // kept apps\n}\n";
    assert_eq!(
        apply(
            Setting::BarApps(vec![
                "org.unbound.Terminal".to_owned(),
                "org.mozilla.firefox".to_owned()
            ]),
            source
        ),
        "bar {\n    edge \"left\" // near\n    apps \"org.unbound.Terminal\" \"org.mozilla.firefox\" // kept apps\n}\n"
    );
    assert_eq!(
        apply(Setting::BarApps(Vec::new()), ""),
        "bar {\n    apps\n}\n"
    );
    assert!(
        Setting::BarApps(vec!["a.b".to_owned(), "a.b".to_owned()])
            .apply("")
            .is_err()
    );
}

#[test]
fn dock_and_wallpaper_defaults_leave_no_trace() {
    let edited = apply(Setting::BarAutoHide(true), "");
    let edited = apply(Setting::BarIconSize(52), &edited);
    assert_eq!(edited, "bar {\n    auto-hide #true\n    size 52\n}\n");
    let edited = apply(Setting::BarIconSize(40), &edited);
    assert_eq!(apply(Setting::BarAutoHide(false), &edited), "");

    let fitted = apply(Setting::WallpaperFit(WallpaperFit::Fit), "");
    assert_eq!(fitted, "wallpaper {\n    fit \"fit\"\n}\n");
    assert_eq!(
        apply(Setting::WallpaperFit(WallpaperFit::Fill), &fitted),
        ""
    );
}

#[test]
fn accessibility_writes_only_what_differs_from_the_defaults_and_reads_back() {
    let access = Accessibility {
        reduce_motion: true,
        zoom: 2.5,
        cursor_size: 48,
        sticky_keys: true,
        ..Accessibility::default()
    };
    let written = apply(Setting::Accessibility(access), "");
    assert_eq!(
        written,
        "accessibility {\n    reduce-motion #true\n    sticky-keys #true\n    zoom 2.5\n    cursor-size 48\n}\n"
    );
    assert_eq!(
        Config::parse(&written).expect("valid").accessibility,
        access
    );
    assert_eq!(
        apply(Setting::Accessibility(Accessibility::default()), &written),
        ""
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
fn an_unreadable_file_is_not_edited() {
    let error = Setting::Appearance(Appearance::Dark)
        .apply("appearance \"dark\"\nbar {\n    edge \"middle\n}\n")
        .expect_err("invalid file");
    assert_eq!((error.line, error.column), (3, 10));
}

#[test]
fn a_file_with_a_wrong_value_is_still_edited_and_keeps_it_for_the_user() {
    let source = "bar {\n    edge \"middle\"\n}\n";

    assert_eq!(
        apply(Setting::Appearance(Appearance::Dark), source),
        "bar {\n    edge \"middle\"\n}\nappearance \"dark\"\n"
    );
    assert_eq!(
        apply(Setting::BarEdge(BarEdge::Top), source),
        "bar {\n    edge \"top\"\n}\n"
    );
}

#[test]
fn a_write_renames_the_old_panel_section_instead_of_adding_a_bar() {
    assert_eq!(
        apply(
            Setting::BarEdge(BarEdge::Top),
            "panel {\n    edge \"left\" // mine\n}\n"
        ),
        "bar {\n    edge \"top\" // mine\n}\n"
    );
}

#[test]
fn the_old_dock_section_moves_into_an_existing_bar_without_overriding_it() {
    assert_eq!(
        apply(
            Setting::BarNames(true),
            "dock {\n    size 48\n    edge \"right\"\n}\nbar {\n    edge \"top\"\n}\n"
        ),
        "bar {\n    edge \"top\"\n    size 48\n    names #true\n}\n"
    );
}

const OWNERS_FILE: &str = "appearance \"dark\"
output \"Virtual-1\" {
    scale 1.5
}
bar {
    size 44
}
";

#[test]
fn the_old_connector_block_becomes_the_named_display_keeping_its_layout() {
    let connected = [("Virtual-1".to_owned(), "Virtual-1".to_owned())];
    assert_eq!(
        migrate(OWNERS_FILE, &connected).expect("KDL").as_deref(),
        Some(
            "appearance \"dark\"\ndisplay \"Virtual-1\" {\n    scale 1.5\n}\nbar {\n    size 44\n}\n"
        )
    );

    let commented = "// laptop\noutput \"eDP-1\" { // built in\n    scale 2 // retina\n}\n";
    let connected = [(
        "eDP-1".to_owned(),
        "Apple Built-in Retina Display".to_owned(),
    )];
    assert_eq!(
        migrate(commented, &connected).expect("KDL").as_deref(),
        Some(
            "// laptop\ndisplay \"Apple Built-in Retina Display\" { // built in\n    scale 2 // retina\n}\n"
        )
    );
}

#[test]
fn an_old_connector_block_waits_for_its_display_and_yields_to_a_named_one() {
    assert_eq!(migrate(OWNERS_FILE, &[]).expect("KDL"), None);
    assert_eq!(migrate("appearance \"dark\"\n", &[]).expect("KDL"), None);

    let both = "output \"DP-1\" {\n    scale 2\n}\ndisplay \"Dell U2720Q\" {\n    scale 1.5\n}\n";
    let connected = [("DP-1".to_owned(), "Dell U2720Q".to_owned())];
    assert_eq!(
        migrate(both, &connected).expect("KDL").as_deref(),
        Some("display \"Dell U2720Q\" {\n    scale 1.5\n}\n")
    );
}

#[test]
fn a_display_is_added_changed_and_removed_by_its_name() {
    let added = apply(
        Setting::Display(
            "Dell U2720Q".to_owned(),
            DisplaySettings {
                resolution: Some((3840, 2160)),
                scale: Some(1.5),
                refresh_rate: Some(120.0),
                ..DisplaySettings::default()
            },
        ),
        "display \"BOE\" {\n    scale 2.0\n}\n",
    );
    assert_eq!(
        added,
        "display \"BOE\" {\n    scale 2.0\n}\ndisplay \"Dell U2720Q\" {\n    resolution 3840 2160\n    refresh-rate 120\n    scale 1.5\n}\n"
    );

    let changed = apply(
        Setting::Display(
            "Dell U2720Q".to_owned(),
            DisplaySettings {
                refresh_rate: Some(60.0),
                rotation: Some(Rotation::Quarter),
                variable_refresh_rate: Some(true),
                ..DisplaySettings::default()
            },
        ),
        &added,
    );
    assert_eq!(
        changed,
        "display \"BOE\" {\n    scale 2.0\n}\ndisplay \"Dell U2720Q\" {\n    refresh-rate 60\n    rotation 90\n    variable-refresh-rate #true\n}\n"
    );

    let removed = apply(
        Setting::Display("Dell U2720Q".to_owned(), DisplaySettings::default()),
        &changed,
    );
    assert_eq!(removed, "display \"BOE\" {\n    scale 2.0\n}\n");
}

#[test]
fn an_arrangement_replaces_the_one_for_the_same_displays() {
    let placement = |display: &str, x: i32, main: bool| Placement {
        display: display.to_owned(),
        x,
        y: 0,
        main,
        mirror: None,
    };
    let laptop = apply(
        Setting::Arrangement(vec![placement("BOE", 0, true)]),
        "appearance \"dark\"\n",
    );
    let both = apply(
        Setting::Arrangement(vec![
            placement("BOE", 0, true),
            placement("Dell", 1504, false),
        ]),
        &laptop,
    );
    let moved = apply(
        Setting::Arrangement(vec![
            placement("Dell", -2560, true),
            Placement {
                mirror: Some("Dell".to_owned()),
                ..placement("BOE", 0, false)
            },
        ]),
        &both,
    );

    assert_eq!(
        moved,
        "appearance \"dark\"\narrangement {\n    display \"BOE\" x=0 y=0 main=#true\n}\narrangement {\n    display \"Dell\" x=-2560 y=0 main=#true\n    display \"BOE\" x=0 y=0 mirror=\"Dell\"\n}\n"
    );
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

#[test]
fn input_sources_are_written_as_codes_and_defaults_leave_no_trace() {
    let sources = vec![
        InputSource::parse("se").expect("layout"),
        InputSource::parse("de(nodeadkeys)").expect("layout"),
    ];
    let written = apply(Setting::InputSources(sources.clone()), "");
    assert_eq!(
        written,
        "keyboard {\n    input-sources \"se\" \"de(nodeadkeys)\"\n}\n"
    );
    assert_eq!(
        Config::parse(&written)
            .expect("valid")
            .keyboard
            .input_sources,
        sources
    );
    let escape = apply(Setting::CapsLock(CapsLock::Escape), &written);
    assert!(escape.contains("caps-lock \"escape\""), "{escape}");
    let restored = apply(Setting::CapsLock(CapsLock::CapsLock), &escape);
    assert_eq!(apply(Setting::InputSources(Vec::new()), &restored), "");
}

#[test]
fn pointer_settings_equal_to_the_defaults_leave_no_trace() {
    let slower = apply(Setting::TrackpadSpeed(-0.25), "");
    assert_eq!(slower, "trackpad {\n    speed -0.25\n}\n");
    let tapless = apply(Setting::TrackpadTapToClick(false), &slower);
    assert!(tapless.contains("tap-to-click #false"), "{tapless}");
    let natural = apply(Setting::MouseNaturalScrolling(true), "");
    assert_eq!(natural, "mouse {\n    natural-scrolling #true\n}\n");
    assert_eq!(apply(Setting::MouseNaturalScrolling(false), &natural), "");
    let reset = apply(
        Setting::TrackpadTapToClick(true),
        &apply(Setting::TrackpadSpeed(0.0), &tapless),
    );
    assert_eq!(reset, "");
}

#[test]
fn power_settings_live_in_one_section_and_defaults_leave_no_trace() {
    let edited = apply(Setting::LowPowerOnBattery(true), "");
    let edited = apply(Setting::LidOnPower(LidOnPower::KeepRunning), &edited);
    let edited = apply(Setting::ShowBatteryPercentage(true), &edited);
    let power = Config::parse(&edited).expect("valid").power;
    assert!(power.low_power_on_battery && power.show_battery_percentage);
    assert_eq!(power.lid_on_power, LidOnPower::KeepRunning);
    assert!(edited.contains("lid-on-power \"keep-running\""));
    let edited = apply(Setting::LowPowerOnBattery(false), &edited);
    let edited = apply(Setting::LidOnPower(LidOnPower::Sleep), &edited);
    let edited = apply(Setting::ShowBatteryPercentage(false), &edited);
    assert_eq!(
        Config::parse(&edited).expect("valid").power,
        Default::default()
    );
    assert!(!edited.contains("low-power") && !edited.contains("lid"));
    assert_eq!(
        Config::load("power { lid-on-power \"explode\" }")
            .expect("KDL")
            .1
            .len(),
        1
    );
}

#[test]
fn sound_settings_are_written_only_when_they_differ_from_the_defaults() {
    let edited = apply(Setting::AlertSound(AlertSound::Low), "");
    let edited = apply(Setting::AlertVolume(0.5), &edited);
    let edited = apply(Setting::SoundEffects(false), &edited);
    assert_eq!(
        edited,
        "sound {\n    alert \"low\"\n    alert-volume 0.5\n    effects #false\n}\n"
    );
    let edited = apply(Setting::SoundEffects(true), &edited);
    let edited = apply(Setting::AlertSound(AlertSound::Mallet), &edited);
    assert_eq!(edited, "sound {\n    alert-volume 0.5\n}\n");
}
