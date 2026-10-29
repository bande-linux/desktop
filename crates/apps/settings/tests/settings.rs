use std::sync::Arc;

use unbound_config::{Appearance, Config, ConfigService, ConfigStore, PanelEdge};
use unbound_settings::{Key, SettingsClient, SettingsView, Value, panes, search};
use unbound_test_bus::SessionBus;
use unbound_toolkit::ui::{Event, Key as Pressed, Modifiers};
use unbound_toolkit::{Palette, Point, Scheme, TextSystem};

fn view() -> SettingsView {
    SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new())
}

fn press(view: &mut SettingsView, key: Pressed) -> bool {
    view.handle(&Event::Key {
        key,
        modifiers: Modifiers::default(),
    })
}

fn type_text(view: &mut SettingsView, text: &str) {
    for character in text.chars() {
        press(view, Pressed::Character(character));
    }
}

fn pane_names(view: &SettingsView) -> Vec<&'static str> {
    view.visible_panes()
        .into_iter()
        .map(|pane| view.panes()[pane].name)
        .collect()
}

#[test]
fn search_finds_settings_by_label_keyword_and_choice() {
    let panes = panes();
    let found = |query: &str| {
        search(&panes, query)
            .into_iter()
            .map(|found| panes[found.pane].name)
            .collect::<Vec<_>>()
    };

    assert_eq!(found("dark"), ["Appearance"]);
    assert_eq!(found("Dock"), ["Desktop & Dock"]);
    assert_eq!(found("password"), ["Lock Screen"]);
    assert_eq!(found("sleep never"), ["Lock Screen"]);
    assert!(found("bluetooth").is_empty());
}

#[test]
fn typing_narrows_the_sidebar_and_highlights_the_matching_controls() {
    let mut view = view();
    assert_eq!(
        pane_names(&view),
        ["Appearance", "Desktop & Dock", "Lock Screen"]
    );

    type_text(&mut view, "suspend");

    assert_eq!(view.query(), "suspend");
    assert_eq!(pane_names(&view), ["Lock Screen"]);
    assert_eq!(view.selected_pane().name, "Lock Screen");
    assert_eq!(view.highlighted_controls(), [2]);

    press(&mut view, Pressed::Escape);
    assert_eq!(pane_names(&view).len(), 3);
    assert!(view.highlighted_controls().is_empty());
}

#[test]
fn arrows_change_the_focused_setting_and_report_the_change() {
    let mut view = view();
    view.set_value(Key::Appearance, Value::Keyword("light".to_owned()));

    press(&mut view, Pressed::Tab);
    assert_eq!(
        view.focused_control().map(|control| control.key),
        Some(Key::Appearance)
    );
    press(&mut view, Pressed::Right);

    assert_eq!(
        view.take_changes(),
        [(Key::Appearance, Value::Keyword("dark".to_owned()))]
    );
    press(&mut view, Pressed::Right);
    press(&mut view, Pressed::Right);
    assert_eq!(
        view.take_changes(),
        [(Key::Appearance, Value::Keyword("automatic".to_owned()))]
    );

    press(&mut view, Pressed::Down);
    assert_eq!(view.selected_pane().name, "Desktop & Dock");
    assert!(view.focused_control().is_none());
}

#[test]
fn clicking_a_choice_selects_it() {
    let mut view = view();
    view.scene(900, 540);
    press(&mut view, Pressed::Down);
    view.set_value(Key::DockPosition, Value::Keyword("left".to_owned()));
    view.scene(900, 540);

    let mut changed = Vec::new();
    for x in (260..700).step_by(4) {
        view.handle(&Event::PointerDown(Point::new(x as f32, 105.0)));
        changed.extend(view.take_changes());
    }

    let chosen = changed
        .iter()
        .map(|(_, value)| value.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        chosen,
        ["bottom", "right", "top"].map(|edge| Value::Keyword(edge.to_owned()))
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

    client
        .set(Key::Appearance, &Value::Keyword("dark".to_owned()))
        .expect("appearance");
    client
        .set(Key::DockPosition, &Value::Keyword("bottom".to_owned()))
        .expect("dock");
    client
        .set(Key::SleepAfter, &Value::Seconds(0))
        .expect("sleep");

    let written =
        Config::parse(&std::fs::read_to_string(&path).expect("config.kdl")).expect("valid config");
    assert_eq!(written.appearance, Appearance::Dark);
    assert_eq!(written.panel_edge, PanelEdge::Bottom);
    assert_eq!(written.idle.suspend_after(), None);
    assert_eq!(
        client.get(Key::Appearance).expect("read back"),
        Value::Keyword("dark".to_owned())
    );
}
