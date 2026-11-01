#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use accesskit::{Action, ActionRequest, Node, Role, TreeId};
use bande_config::{
    Appearance, BarEdge, CapsLock, Config, ConfigService, ConfigStore, MenuBarPlacement,
    SecondaryClick, Suspend, WallpaperFit,
};
use bande_settings::{Command, Idle, PaneId, SettingId, SettingsClient, SettingsView, Value};
use bande_settings_panes::panes;
use bande_test_bus::SessionBus;
use bande_toolkit::ui::{Event, Key, Modifiers, Pointer};
use bande_toolkit::{Item, Paint, Palette, Point, Scheme, TextSystem};

const SIZE: (u32, u32) = (900, 720);

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
    assert_eq!(view.visible_panes().len(), 7);

    type_text(&mut view, "suspend");

    assert!(view.search_focused());
    assert_eq!(view.query(), "suspend");
    assert_eq!(view.visible_panes(), [PaneId::Power]);
    assert_eq!(view.selected_pane(), PaneId::Power);
    assert_eq!(view.highlighted_settings(), [SettingId::SleepAfter]);

    press(&mut view, Key::Escape);
    assert_eq!(view.query(), "");
    assert_eq!(view.visible_panes().len(), 7);
    assert!(view.highlighted_settings().is_empty());
    assert_eq!(view.selected_pane(), PaneId::Power);
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
    view.set_value(SettingId::BarEdge, keyword("left"));
    let row = node(&mut view, Role::ListItem, "Taskbar")
        .bounds()
        .expect("bounds");
    click(
        &mut view,
        Point::new(row.x0 as f32 + 10.0, row.y0 as f32 + 10.0),
    );
    assert_eq!(view.selected_pane(), PaneId::Bar);

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
    assert_eq!(chosen, ["right", "bottom", "top"].map(keyword));
}

fn strokes_of(items: &[Item], color: &Paint) -> usize {
    items
        .iter()
        .map(|item| match item {
            Item::Stroke { paint, .. } => usize::from(paint == color),
            Item::Clip { items, .. }
            | Item::Translate { items, .. }
            | Item::Opacity { items, .. }
            | Item::Scale { items, .. } => strokes_of(items, color),
            _ => 0,
        })
        .sum()
}

#[test]
fn a_clicked_sidebar_row_takes_the_keys_without_a_focus_ring() {
    let mut view = view();
    let row = node(&mut view, Role::ListItem, "Bar")
        .bounds()
        .expect("bounds");
    click(
        &mut view,
        Point::new(row.x0 as f32 + 10.0, row.y0 as f32 + 10.0),
    );
    assert_eq!(focused_label(&view).as_deref(), Some("Bar"));

    let scene = view.scene(SIZE.0, SIZE.1);
    let focus = Paint::from(Palette::for_scheme(Scheme::Light).focus);
    assert_eq!(strokes_of(&scene.items, &focus), 0);
    assert!(press(&mut view, Key::Down));
    assert_ne!(view.selected_pane(), PaneId::Bar);
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

    for choice in ["1", "0"] {
        view.scene(SIZE.0, SIZE.1);
        let menu = view
            .accessibility()
            .nodes
            .into_iter()
            .find(|(_, node)| {
                node.role() == Role::ComboBox
                    && node.label() == Some("Turn off the display and lock after")
            })
            .map(|(id, _)| id)
            .expect("lock menu");
        view.perform(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: menu,
            data: None,
        });
        assert!(view.ui().menu_action(choice));
        view.settle();
    }

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
fn changes_are_written_through_org_bande_config() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("bande").join("config.kdl");
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
        (SettingId::BarEdge, keyword("bottom")),
        (SettingId::SleepAfter, Value::Seconds(0)),
        (SettingId::KeyRepeatDelay, Value::Number(300)),
        (SettingId::BarIconSize, Value::Number(52)),
        (SettingId::BarAutoHide, Value::Flag(true)),
        (SettingId::BarNames, Value::Flag(true)),
        (SettingId::BarMenuBar, keyword("separate")),
        (SettingId::ReduceMotion, Value::Flag(true)),
        (SettingId::Zoom, Value::Flag(true)),
        (SettingId::IncreaseContrast, Value::Flag(true)),
        (SettingId::CursorSize, Value::Number(48)),
        (SettingId::StickyKeys, Value::Flag(true)),
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
                light: "/usr/share/bande/wallpapers/aurora-light.svg".to_owned(),
                dark: "/usr/share/bande/wallpapers/aurora-dark.svg".to_owned(),
            },
        ),
    ] {
        client.set(id, &value).expect("set");
    }

    let written =
        Config::parse(&std::fs::read_to_string(&path).expect("config.kdl")).expect("valid config");
    assert_eq!(written.appearance, Appearance::Dark);
    assert_eq!(written.bar.edge, BarEdge::Bottom);
    assert_eq!(written.idle.suspend, Suspend::Never);
    assert_eq!(written.keyboard.repeat_delay, Duration::from_millis(300));
    assert_eq!((written.bar.icon_size, written.bar.auto_hide), (52, true));
    assert!(written.bar.names);
    assert_eq!(written.bar.menu_bar, MenuBarPlacement::Separate);
    let access = written.accessibility;
    assert!(access.reduce_motion && access.increase_contrast && access.sticky_keys);
    assert_eq!((access.zoom, access.cursor_size), (2.0, 48));
    assert_eq!(
        client.get(SettingId::Zoom).expect("zoom"),
        Value::Flag(true)
    );
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
            "/usr/share/bande/wallpapers/aurora-dark.svg"
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
        client.read(&bande_settings::CONFIG_SETTINGS).len(),
        bande_settings::CONFIG_SETTINGS.len()
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

fn focused_label(view: &SettingsView) -> Option<String> {
    let tree = view.accessibility();
    tree.nodes
        .iter()
        .find(|(id, _)| *id == tree.focus)
        .and_then(|(_, node)| node.label().map(str::to_owned))
}

#[test]
fn tab_reaches_every_accessibility_control_and_space_switches_it() {
    let mut view = view();
    for id in [
        SettingId::ScreenReader,
        SettingId::Zoom,
        SettingId::IncreaseContrast,
        SettingId::ReduceTransparency,
        SettingId::ReduceMotion,
        SettingId::FlashScreen,
        SettingId::StickyKeys,
        SettingId::SlowKeys,
        SettingId::BounceKeys,
    ] {
        view.set_value(id, Value::Flag(false));
    }
    view.set_value(SettingId::CursorSize, Value::Number(24));
    view.select_pane(PaneId::Accessibility);
    view.scene(SIZE.0, SIZE.1);
    let mut reached = Vec::new();
    for _ in 0..40 {
        press(&mut view, Key::Tab);
        if let Some(label) = focused_label(&view)
            && !reached.contains(&label)
        {
            reached.push(label);
        }
    }
    for label in [
        "Screen reader",
        "Zoom",
        "Increase contrast",
        "Reduce transparency",
        "Reduce motion",
        "Pointer size",
        "Flash the screen when an alert sound plays",
        "Sticky keys",
        "Slow keys",
        "Bounce keys",
    ] {
        assert!(
            reached.iter().any(|known| known == label),
            "Tab never reached {label}: {reached:?}"
        );
    }
    while focused_label(&view).as_deref() != Some("Sticky keys") {
        press(&mut view, Key::Tab);
    }
    changes(&mut view);
    press(&mut view, Key::Space);
    assert_eq!(
        changes(&mut view),
        [(SettingId::StickyKeys, Value::Flag(true))]
    );
}

fn labelled(view: &mut SettingsView, label: &str) -> bool {
    view.scene(SIZE.0, SIZE.1);
    view.accessibility()
        .nodes
        .iter()
        .any(|(_, node)| node.label().is_some_and(|text| text == label))
}

#[test]
fn without_a_screen_reader_installed_its_switch_and_shortcut_are_not_offered() {
    let mut view = view();
    view.set_value(SettingId::ScreenReader, Value::Flag(false));
    view.set_value(SettingId::Zoom, Value::Flag(false));
    view.select_pane(PaneId::Accessibility);
    assert!(labelled(&mut view, "Screen reader"));
    view.select_pane(PaneId::KeyboardShortcuts);
    assert!(labelled(&mut view, "Turn the screen reader on or off"));

    view.hide_screen_reader();

    assert!(labelled(&mut view, "Turn zoom on or off"));
    assert!(!labelled(&mut view, "Turn the screen reader on or off"));
    view.select_pane(PaneId::Accessibility);
    assert!(labelled(&mut view, "Zoom"));
    assert!(!labelled(&mut view, "Screen reader"));
    type_text(&mut view, "screen reader");
    assert!(
        !view
            .highlighted_settings()
            .contains(&SettingId::ScreenReader)
    );
}
