#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use bande_backup::client::{DiskChoice, Status, phase, problem};
use bande_config::{Config, ConfigService, ConfigStore};
use bande_settings::{
    BackupUpdate, Command, PaneId, SettingId, SettingsClient, SettingsView, Update, Value,
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
}

#[test]
fn the_backup_pane_offers_disks_then_says_what_is_wrong() {
    let shows = |view: &mut SettingsView, text: &str| {
        view.scene(SIZE.0, SIZE.1);
        view.accessibility()
            .nodes
            .iter()
            .any(|(_, node)| node.label().is_some_and(|label| label.contains(text)))
    };
    let status = |phase: &str, problem: &str| {
        Update::Backup(BackupUpdate::Status(Status {
            phase: phase.to_owned(),
            problem: problem.to_owned(),
            disk_name: "Passport".to_owned(),
            ..Status::default()
        }))
    };
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.select_pane(PaneId::Backup);
    view.apply(vec![
        status(phase::NO_DISK, ""),
        Update::Backup(BackupUpdate::Disks(vec![DiskChoice {
            uuid: "1234".to_owned(),
            name: "Passport".to_owned(),
            filesystem: "ext4".to_owned(),
            size: 1_000_000_000_000,
        }])),
    ]);
    assert!(shows(&mut view, "Use This Disk"));

    view.apply(vec![status(phase::DISK_AWAY, problem::DISK_FULL)]);
    assert!(shows(&mut view, "The disk is too small"));
    assert!(shows(&mut view, "Passport isn't connected"));
}
