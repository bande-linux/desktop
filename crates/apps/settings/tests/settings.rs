#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use accesskit::{Node, Role};
use unbound_config::{
    Appearance, CapsLock, Config, ConfigService, ConfigStore, DockEdge, SecondaryClick, Suspend,
    WallpaperFit,
};
use unbound_settings::{Command, Idle, PaneId, SettingId, SettingsClient, SettingsView, Value};
use unbound_settings_panes::panes;
use unbound_test_bus::SessionBus;
use unbound_toolkit::ui::{Event, Key, Modifiers, Pointer};
use unbound_toolkit::{Palette, Point, Scheme, TextSystem};

const SIZE: (u32, u32) = (900, 600);

fn view() -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.scene(SIZE.0, SIZE.1);
    view
}

fn press(view: &mut SettingsView, key: Key) -> bool {
    let handled = view.handle(&Event::Key {
        key,
        modifiers: Modifiers::default(),
    });
    view.scene(SIZE.0, SIZE.1);
    handled
}

fn type_text(view: &mut SettingsView, text: &str) {
    for character in text.chars() {
        press(view, Key::Character(character));
    }
}

fn keyword(text: &str) -> Value {
    Value::Keyword(text.to_owned())
}

fn node(view: &mut SettingsView, role: Role, label: &str) -> Node {
    view.scene(SIZE.0, SIZE.1);
    view.accessibility()
        .nodes
        .into_iter()
        .map(|(_, node)| node)
        .find(|node| node.role() == role && node.label().is_some_and(|text| text == label))
        .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
}

fn changes(view: &mut SettingsView) -> Vec<(SettingId, Value)> {
    view.take_commands()
        .into_iter()
        .filter_map(|command| match command {
            Command::Set(id, value) => Some((id, value)),
            Command::Storage(_) | Command::Login(_) => None,
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

fn click(view: &mut SettingsView, at: Point) {
    view.handle(&Event::PointerDown(Pointer::at(at)));
    view.handle(&Event::PointerUp(Pointer::at(at)));
    view.scene(SIZE.0, SIZE.1);
}

#[test]
fn changing_one_idle_time_keeps_dim_before_lock_before_sleep() {
    let idle = Idle {
        dim: 300,
        lock: 600,
        sleep: Some(1800),
    };
    assert_eq!(
        idle.writes(SettingId::LockAfter, &Value::Seconds(60)),
        [
            (SettingId::DimAfter, Value::Seconds(60)),
            (SettingId::LockAfter, Value::Seconds(60)),
        ]
    );
    assert_eq!(
        idle.writes(SettingId::LockAfter, &Value::Seconds(3600)),
        [
            (SettingId::SleepAfter, keyword("with-display")),
            (SettingId::LockAfter, Value::Seconds(3600)),
        ]
    );
    assert_eq!(
        Idle {
            sleep: None,
            ..idle
        }
        .writes(SettingId::DimAfter, &Value::Seconds(600)),
        [(SettingId::DimAfter, Value::Seconds(600))]
    );
    assert_eq!(
        Idle { lock: 300, ..idle }.writes(SettingId::DimAfter, &Value::Seconds(600)),
        [
            (SettingId::LockAfter, Value::Seconds(600)),
            (SettingId::DimAfter, Value::Seconds(600)),
        ]
    );
    assert_eq!(
        Idle { lock: 3600, ..idle }.writes(SettingId::SleepAfter, &Value::Seconds(1800)),
        [
            (SettingId::LockAfter, Value::Seconds(1200)),
            (SettingId::SleepAfter, Value::Seconds(1800)),
        ]
    );
    assert_eq!(
        Idle { lock: 3600, ..idle }.writes(SettingId::SleepAfter, &Value::Seconds(0)),
        [(SettingId::SleepAfter, Value::Seconds(0))]
    );
}

#[test]
fn typing_anywhere_searches_and_narrows_the_sidebar_to_ranked_panes() {
    let mut view = view();
    assert_eq!(view.visible_panes().len(), 6);

    type_text(&mut view, "suspend");

    assert!(view.search_focused());
    assert_eq!(view.query(), "suspend");
    assert_eq!(view.visible_panes(), [PaneId::LockScreen]);
    assert_eq!(view.selected_pane(), PaneId::LockScreen);
    assert_eq!(view.highlighted_settings(), [SettingId::SleepAfter]);

    press(&mut view, Key::Escape);
    assert_eq!(view.query(), "");
    assert_eq!(view.visible_panes().len(), 6);
    assert!(view.highlighted_settings().is_empty());
    assert_eq!(view.selected_pane(), PaneId::LockScreen);
}

#[test]
fn return_in_the_search_field_focuses_the_best_match() {
    let mut view = view();
    view.set_value(SettingId::KeyRepeatRate, Value::Number(25));
    type_text(&mut view, "repeat rate");

    press(&mut view, Key::Enter);
    assert_eq!(view.focused_setting(), Some(SettingId::KeyRepeatRate));

    press(&mut view, Key::Right);
    assert_eq!(
        changes(&mut view),
        [(SettingId::KeyRepeatRate, Value::Number(30))]
    );
    assert_eq!(view.focused_setting(), Some(SettingId::KeyRepeatRate));
}

#[test]
fn arrows_in_the_search_field_walk_the_sidebar() {
    let mut view = view();
    type_text(&mut view, "e");
    let visible = view.visible_panes();
    assert!(visible.len() > 1);
    assert_eq!(view.selected_pane(), visible[0]);

    press(&mut view, Key::Down);
    assert_eq!(view.selected_pane(), visible[1]);
    press(&mut view, Key::Up);
    assert_eq!(view.selected_pane(), visible[0]);
    assert!(view.search_focused());
}

#[test]
fn tab_reaches_a_control_and_arrows_change_it() {
    let mut view = view();
    view.set_value(SettingId::Appearance, keyword("light"));

    while view.focused_setting().is_none() {
        press(&mut view, Key::Tab);
    }
    assert_eq!(view.focused_setting(), Some(SettingId::Appearance));
    press(&mut view, Key::Right);
    assert_eq!(
        changes(&mut view),
        [(SettingId::Appearance, keyword("dark"))]
    );
    assert_eq!(view.value(SettingId::Appearance), Some(&keyword("dark")));
    assert_eq!(view.focused_setting(), Some(SettingId::Appearance));
}

#[test]
fn clicking_a_pane_and_then_a_choice_selects_both() {
    let mut view = view();
    view.set_value(SettingId::DockEdge, keyword("left"));
    let row = node(&mut view, Role::ListItem, "Dock")
        .bounds()
        .expect("bounds");
    click(
        &mut view,
        Point::new(row.x0 as f32 + 10.0, row.y0 as f32 + 10.0),
    );
    assert_eq!(view.selected_pane(), PaneId::Dock);

    let control = node(&mut view, Role::RadioGroup, "Position on screen")
        .bounds()
        .expect("bounds");
    let mut chosen = Vec::new();
    let mut x = control.x0 as f32 + 2.0;
    while x < control.x1 as f32 {
        click(
            &mut view,
            Point::new(x, (control.y0 + control.y1) as f32 / 2.0),
        );
        chosen.extend(changes(&mut view).into_iter().map(|(_, value)| value));
        x += 4.0;
    }
    assert_eq!(chosen, ["bottom", "right", "top"].map(keyword));
}

#[test]
fn shortening_the_lock_time_also_shortens_dimming() {
    let mut view = view();
    view.set_value(SettingId::DimAfter, Value::Seconds(300));
    view.set_value(SettingId::LockAfter, Value::Seconds(600));
    view.set_value(SettingId::SleepAfter, keyword("with-display"));
    type_text(&mut view, "lock after");
    press(&mut view, Key::Enter);
    assert_eq!(view.focused_setting(), Some(SettingId::LockAfter));

    press(&mut view, Key::Left);
    press(&mut view, Key::Left);

    assert_eq!(
        changes(&mut view),
        [
            (SettingId::LockAfter, Value::Seconds(300)),
            (SettingId::DimAfter, Value::Seconds(60)),
            (SettingId::LockAfter, Value::Seconds(60)),
        ]
    );
}

#[test]
fn changes_are_written_through_org_unbound_config() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("unbound").join("config.kdl");
    let service_connection = bus.connect().expect("service connection");
    let _service = ConfigService::start(
        &service_connection,
        &path,
        Arc::new(ConfigStore::default()),
        |_| {},
    )
    .expect("config service");
    let client = SettingsClient::connect(&bus.connect().expect("client connection"))
        .expect("settings client");

    assert_eq!(
        client.get(SettingId::SleepAfter).expect("sleep"),
        keyword("with-display")
    );
    for (id, value) in [
        (SettingId::Appearance, keyword("dark")),
        (SettingId::DockEdge, keyword("bottom")),
        (SettingId::SleepAfter, Value::Seconds(0)),
        (SettingId::KeyRepeatDelay, Value::Number(300)),
        (SettingId::DockIconSize, Value::Number(64)),
        (SettingId::DockAutoHide, Value::Flag(true)),
        (SettingId::ReduceMotion, Value::Flag(true)),
        (SettingId::WallpaperFit, keyword("fit")),
        (
            SettingId::InputSources,
            Value::Codes(vec!["se".to_owned(), "us(dvorak)".to_owned()]),
        ),
        (SettingId::CapsLock, keyword("escape")),
        (SettingId::TrackpadSpeed, Value::Number(8)),
        (SettingId::TrackpadSecondaryClick, keyword("corner")),
        (SettingId::MouseNaturalScrolling, Value::Flag(true)),
        (
            SettingId::Wallpaper,
            Value::Pictures {
                light: "/usr/share/unbound/wallpapers/aurora-light.svg".to_owned(),
                dark: "/usr/share/unbound/wallpapers/aurora-dark.svg".to_owned(),
            },
        ),
    ] {
        client.set(id, &value).expect("set");
    }

    let written =
        Config::parse(&std::fs::read_to_string(&path).expect("config.kdl")).expect("valid config");
    assert_eq!(written.appearance, Appearance::Dark);
    assert_eq!(written.dock.edge, DockEdge::Bottom);
    assert_eq!(written.idle.suspend, Suspend::Never);
    assert_eq!(written.keyboard.repeat_delay, Duration::from_millis(300));
    assert_eq!((written.dock.icon_size, written.dock.auto_hide), (64, true));
    assert!(written.reduce_motion);
    assert_eq!(written.wallpaper.fit, WallpaperFit::Fit);
    assert_eq!(
        written
            .keyboard
            .input_sources
            .iter()
            .map(|source| source.code())
            .collect::<Vec<_>>(),
        ["se", "us(dvorak)"]
    );
    assert_eq!(written.keyboard.caps_lock, CapsLock::Escape);
    assert!((written.trackpad.speed - 0.6).abs() < 1e-9);
    assert_eq!(written.trackpad.secondary_click, SecondaryClick::Corner);
    assert!(written.mouse.natural_scrolling);
    assert_eq!(
        client.get(SettingId::TrackpadSpeed).expect("speed"),
        Value::Number(8)
    );
    assert_eq!(
        written.wallpaper.dark.as_deref(),
        Some(std::path::Path::new(
            "/usr/share/unbound/wallpapers/aurora-dark.svg"
        ))
    );

    client
        .set(SettingId::SleepAfter, &keyword("with-display"))
        .expect("with display");
    assert_eq!(
        client.get(SettingId::SleepAfter).expect("sleep"),
        keyword("with-display")
    );
    assert_eq!(
        client.read(&unbound_settings::CONFIG_SETTINGS).len(),
        unbound_settings::CONFIG_SETTINGS.len()
    );
}

#[test]
fn every_pane_opens_by_its_keyword() {
    let mut view = view();
    for pane in panes() {
        assert_eq!(PaneId::from_keyword(pane.id.keyword()), Some(pane.id));
        view.select_pane(pane.id);
        assert_eq!(view.selected_pane(), pane.id);
    }
    assert_eq!(PaneId::from_keyword("printers"), None);
}
