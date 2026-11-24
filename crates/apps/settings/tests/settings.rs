#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use bande_config::SoftwareUpdates;
use bande_config::{Config, ConfigService, ConfigStore};
use bande_settings::{
    Command, Display, PaneId, PointingDevices, Problem, ScreenSharing, SettingId, SettingsClient,
    SettingsView, SharingCommand, SharingState, Update, Value,
};
use bande_test_bus::SessionBus;
use bande_toolkit::ui::{Event, Key, Modifiers};
use bande_toolkit::{Palette, Scheme, TextSystem};

const SIZE: (u32, u32) = (900, 720);

fn press(view: &mut SettingsView, key: Key) {
    view.handle(&Event::Key {
        key,
        modifiers: Modifiers::default(),
    });
    view.scene(SIZE.0, SIZE.1);
}

fn search(query: &str) -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.set_pointing_devices(PointingDevices {
        mouse: true,
        trackpad: false,
    });
    view.set_displays(vec![Display::default()]);
    view.set_value(SettingId::BarFloating, Value::Flag(true));
    view.set_value(SettingId::NightLight, Value::Flag(false));
    view.scene(SIZE.0, SIZE.1);
    for character in query.chars() {
        press(&mut view, Key::Character(character));
    }
    view
}

#[test]
fn moved_settings_are_found_in_their_pane_and_absent_devices_are_not() {
    for (query, pane) in [
        ("sleep after", PaneId::Power),
        ("lock screen", PaneId::PrivacySecurity),
        ("recent files", PaneId::PrivacySecurity),
        ("send report", PaneId::Diagnostics),
        ("saved passwords", PaneId::Passwords),
        ("automatic updates", PaneId::SoftwareUpdate),
        ("primary button", PaneId::Pointing),
        ("spacing", PaneId::BarMore),
        ("variable refresh", PaneId::Displays),
        ("warmth", PaneId::Displays),
    ] {
        assert_eq!(
            search(query).visible_panes().first(),
            Some(&pane),
            "{query}"
        );
    }
    let mut warmth = search("warmth");
    press(&mut warmth, Key::Enter);
    assert_eq!(warmth.focused_setting(), Some(SettingId::NightLight));
    let mut panes = bande_settings_panes::panes();
    bande_settings_panes::with_pointers(&mut panes, true, false);
    assert!(
        bande_settings_panes::search(&panes, "tap to click")
            .iter()
            .all(|hit| hit.setting(SettingId::TrackpadTapToClick).is_none())
    );
    assert_eq!(
        PaneId::from_keyword("software-update"),
        Some(PaneId::SoftwareUpdate)
    );
    assert_eq!(PaneId::SoftwareUpdate.parent(), None);
}

#[test]
fn a_setting_found_by_search_and_changed_from_the_keyboard_is_written_to_the_config_file() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("bande").join("config.kdl");
    let _service = ConfigService::start(
        &bus.connect().expect("service connection"),
        &path,
        Arc::new(ConfigStore::default()),
        |_| {},
    )
    .expect("config service");
    let client = SettingsClient::connect(&bus.connect().expect("client connection"))
        .expect("settings client");
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.scene(SIZE.0, SIZE.1);
    view.set_value(
        SettingId::KeyRepeatRate,
        client.get(SettingId::KeyRepeatRate).expect("repeat rate"),
    );

    for character in "repeat rate".chars() {
        press(&mut view, Key::Character(character));
    }
    assert_eq!(view.visible_panes(), [PaneId::Keyboard]);
    press(&mut view, Key::Enter);
    assert_eq!(view.focused_setting(), Some(SettingId::KeyRepeatRate));
    press(&mut view, Key::Right);
    for command in view.take_commands() {
        if let Command::Set(id, value) = command {
            client.set(id, &value).expect("set");
        }
    }

    let written =
        Config::parse(&std::fs::read_to_string(&path).expect("config.kdl")).expect("valid config");
    assert_eq!(written.keyboard.repeat_rate, 30);
    assert_eq!(
        client.get(SettingId::KeyRepeatRate).expect("repeat rate"),
        Value::Number(30)
    );

    client
        .set(
            SettingId::AutomaticUpdates,
            &Value::Keyword("install".to_owned()),
        )
        .expect("set automatic updates");
    let written =
        Config::parse(&std::fs::read_to_string(&path).expect("config.kdl")).expect("valid config");
    assert_eq!(written.software_updates, SoftwareUpdates::Install);
}

fn node_labelled(view: &SettingsView, label: &str) -> Option<(accesskit::NodeId, accesskit::Role)> {
    view.accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| node.label() == Some(label))
        .map(|(id, node)| (id, node.role()))
}

fn shows(view: &SettingsView, text: &str) -> bool {
    view.accessibility().nodes.iter().any(|(_, node)| {
        [node.label(), node.value(), node.description()]
            .into_iter()
            .flatten()
            .any(|shown| shown.contains(text))
    })
}

#[test]
fn screen_sharing_without_a_password_asks_for_one_and_a_failure_stays_next_to_the_switch() {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.select_pane(PaneId::Sharing);
    view.take_commands();
    view.apply(vec![Update::Sharing(SharingState {
        screen: Some(ScreenSharing::default()),
        ..SharingState::default()
    })]);
    view.scene(SIZE.0, SIZE.1);

    let (switch, role) = node_labelled(&view, "Screen Sharing").expect("the switch");
    assert_eq!(role, accesskit::Role::Switch);
    view.perform(&accesskit::ActionRequest {
        action: accesskit::Action::Click,
        target_tree: accesskit::TreeId::ROOT,
        target_node: switch,
        data: None,
    });
    view.scene(SIZE.0, SIZE.1);
    assert!(
        shows(&view, "Choose a password for Screen Sharing"),
        "switching on without a password asks for one"
    );
    assert!(view.take_commands().is_empty(), "nothing is turned on yet");

    view.handle(&Event::Commit("short".to_owned()));
    press(&mut view, Key::Enter);
    assert!(shows(&view, "Use at least 8 characters."));
    assert!(view.take_commands().is_empty());

    view.handle(&Event::Commit("long enough".to_owned()));
    press(&mut view, Key::Enter);
    assert_eq!(
        view.take_commands(),
        [Command::Sharing(SharingCommand::TurnOnWithPassword(
            "long enough".to_owned()
        ))]
    );
    assert!(shows(&view, "Turning on…"));

    view.apply(vec![Update::Sharing(SharingState {
        screen: Some(ScreenSharing {
            password_set: true,
            ..ScreenSharing::default()
        }),
        screen_problem: Some(Problem::NotStarted),
        ..SharingState::default()
    })]);
    view.scene(SIZE.0, SIZE.1);
    assert!(
        shows(&view, "Screen Sharing didn’t start."),
        "the reason stays next to the switch"
    );
    let switch = view
        .accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| {
            node.label() == Some("Screen Sharing") && node.role() == accesskit::Role::Switch
        })
        .map(|(_, node)| node.toggled());
    assert_eq!(
        switch,
        Some(Some(accesskit::Toggled::False)),
        "the switch is off again"
    );
}
