#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use bande_config::SoftwareUpdates;
use bande_config::{Config, ConfigService, ConfigStore};
use bande_settings::{
    Command, Display, Mode, PaneId, Problem, ScreenSharing, SettingId, SettingsClient,
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
    view.set_displays(vec![Display::default()]);
    view.set_value(SettingId::BarFloating, Value::Flag(false));
    view.set_value(SettingId::NightLight, Value::Flag(false));
    view.scene(SIZE.0, SIZE.1);
    for character in query.chars() {
        press(&mut view, Key::Character(character));
    }
    view
}

#[test]
fn moved_settings_are_found_in_their_pane() {
    for (query, pane) in [
        ("sleep after", PaneId::Power),
        ("lock screen", PaneId::PrivacySecurity),
        ("recent files", PaneId::PrivacySecurity),
        ("send report", PaneId::Diagnostics),
        ("saved passwords", PaneId::Passwords),
        ("automatic updates", PaneId::SoftwareUpdate),
        ("primary button", PaneId::Mouse),
        ("tap to click", PaneId::Trackpad),
        ("show desktop", PaneId::Trackpad),
        ("spacing", PaneId::Bar),
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
    let mut spacing = search("spacing");
    press(&mut spacing, Key::Enter);
    assert_eq!(spacing.focused_setting(), Some(SettingId::BarFloating));
    let panes = search("").visible_panes();
    for (first, next) in [
        (PaneId::Users, PaneId::Passwords),
        (PaneId::Keyboard, PaneId::Mouse),
        (PaneId::Mouse, PaneId::Trackpad),
    ] {
        assert!(
            panes
                .windows(2)
                .any(|pair| pair == [first, next].as_slice()),
            "{first:?} comes right before {next:?}"
        );
    }
    let mut desktop = bande_settings_panes::panes();
    bande_settings_panes::without_battery(&mut desktop);
    for (query, setting) in [
        ("sleep on battery", SettingId::BatterySleepAfter),
        ("lid is closed", SettingId::LidOnPower),
        ("battery level", SettingId::BatteryLevel),
    ] {
        assert!(
            bande_settings_panes::search(&desktop, query)
                .iter()
                .all(|hit| hit.setting(setting).is_none()),
            "{query}"
        );
    }
    assert_eq!(
        PaneId::from_keyword("software-update"),
        Some(PaneId::SoftwareUpdate)
    );
    assert_eq!(PaneId::from_keyword("passwords"), Some(PaneId::Passwords));
}

#[test]
fn a_report_is_described_while_the_pane_updates_and_sent_from_the_keyboard() {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.scene(SIZE.0, SIZE.1);
    view.select_pane(PaneId::Diagnostics);
    view.scene(SIZE.0, SIZE.1);
    let (field, _) = node_labelled(&view, "What happened? (optional)").expect("a description");
    view.perform(&accesskit::ActionRequest {
        action: accesskit::Action::Focus,
        target_tree: accesskit::TreeId::ROOT,
        target_node: field,
        data: None,
    });
    for character in "It".chars() {
        press(&mut view, Key::Character(character));
    }
    view.apply(vec![Update::ReportFailed]);
    for character in " hung".chars() {
        press(&mut view, Key::Character(character));
    }
    assert!(shows(&view, "Couldn’t prepare the report. Try again."));
    view.take_commands();
    press(&mut view, Key::Tab);
    press(&mut view, Key::Enter);
    let commands = format!("{:?}", view.take_commands());
    assert!(!commands.contains("Create"), "{commands}");
    press(&mut view, Key::Tab);
    press(&mut view, Key::Space);
    let commands = format!("{:?}", view.take_commands());
    assert!(commands.contains("Create(\"It hung\", Send)"), "{commands}");
    assert!(shows(&view, "Preparing the report…"));
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

#[test]
fn two_displays_of_one_model_are_told_apart_and_arranged_from_the_keyboard() {
    let mode = Mode {
        width: 1920,
        height: 1080,
        millihertz: 60_000,
    };
    let display = |connector: &str, x| Display {
        connector: connector.to_owned(),
        identity: format!("Example 24 {connector}"),
        make: "Example".to_owned(),
        model: "24".to_owned(),
        modes: vec![mode],
        current: Some(mode),
        position: (x, 0),
        scale: 1.0,
        rotation: 0,
        hdr: false,
    };
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.set_displays(vec![display("DP-1", 0), display("DP-2", 1920)]);
    view.select_pane(PaneId::Displays);
    view.take_commands();
    view.scene(SIZE.0, SIZE.1);
    assert!(shows(&view, "Example 24 (DP-1)") && shows(&view, "Example 24 (DP-2)"));
    let uses = view
        .accessibility()
        .nodes
        .iter()
        .filter(|(_, node)| node.label() == Some("Use as"))
        .count();
    assert!(uses >= 2, "each display has its own choice of use");

    let (arrangement, _) = node_labelled(&view, "Arrangement").expect("the arrangement");
    view.perform(&accesskit::ActionRequest {
        action: accesskit::Action::Focus,
        target_tree: accesskit::TreeId::ROOT,
        target_node: arrangement,
        data: None,
    });
    press(&mut view, Key::Down);
    let moved = view
        .take_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Arrangement(placements) => Some(placements),
            _ => None,
        });
    let places: Vec<(i32, i32, bool)> = moved
        .expect("the second display moves")
        .iter()
        .map(|placement| (placement.x, placement.y, placement.main))
        .collect();
    assert_eq!(places, [(0, 0, true), (0, 1080, false)]);
}
