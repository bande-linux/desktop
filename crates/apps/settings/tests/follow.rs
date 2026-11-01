#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use bande_config::{ConfigService, ConfigStore};
use bande_settings::{
    Command, Feed, PointingDevices, SettingId, Update, Value, feed, follow_config, follow_pointing,
    worker,
};
use bande_test_bus::SessionBus;

const WAIT: Duration = Duration::from_secs(5);

fn wait_for(feed: &Feed, wanted: impl Fn(&Update) -> bool) -> Update {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(update) = feed.drain().into_iter().find(|update| wanted(update)) {
            return update;
        }
        assert!(Instant::now() < deadline, "no matching update arrived");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn has(update: &Update, id: SettingId, value: &Value) -> bool {
    matches!(update, Update::Values(values) if values.contains(&(id, value.clone())))
}

#[test]
fn edits_from_anywhere_arrive_live_and_failed_writes_are_reported() {
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
    let session = bus.connect().expect("client connection");
    let (feed, updates) = feed().expect("feed");
    follow_config(&session, updates.clone());
    let commands = worker(&session, updates);

    let dark = Value::Keyword("dark".to_owned());
    let light = Value::Keyword("light".to_owned());
    wait_for(&feed, |update| {
        has(
            update,
            SettingId::Appearance,
            &Value::Keyword("automatic".to_owned()),
        )
    });

    std::fs::write(&path, "appearance \"dark\"\n").expect("edit by hand");
    wait_for(&feed, |update| has(update, SettingId::Appearance, &dark));

    commands
        .send(Command::Set(SettingId::Appearance, light.clone()))
        .expect("worker");
    wait_for(&feed, |update| has(update, SettingId::Appearance, &light));

    commands
        .send(Command::Set(SettingId::KeyRepeatRate, Value::Number(500)))
        .expect("worker");
    wait_for(&feed, |update| {
        matches!(update, Update::Failed(Some(SettingId::KeyRepeatRate), _))
    });
}

#[test]
fn pointing_devices_are_found_in_udev_data_and_followed_when_plugged_in() {
    let udev = tempfile::tempdir().expect("temp dir");
    let entry = |name: &str, properties: &str| {
        std::fs::write(udev.path().join(name), properties).expect("udev entry")
    };
    entry(
        "c13:64",
        "E:ID_INPUT=1\nE:ID_INPUT_KEY=1\nE:ID_INPUT_KEYBOARD=1\n",
    );
    entry("c189:3", "E:ID_INPUT_TOUCHPAD=1\n");
    assert_eq!(
        PointingDevices::scan(udev.path()),
        PointingDevices::default()
    );
    entry("c13:65", "E:ID_INPUT=1\nE:ID_INPUT_POINTINGSTICK=1\n");
    assert_eq!(
        PointingDevices::scan(udev.path()),
        PointingDevices {
            trackpad: false,
            mouse: true
        }
    );

    let (feed, updates) = feed().expect("feed");
    let _watch = follow_pointing(udev.path().to_path_buf(), updates).expect("watch");
    entry("c13:66", "E:ID_INPUT=1\nE:ID_INPUT_TOUCHPAD=1\n");
    let plugged = wait_for(&feed, |update| matches!(update, Update::Pointing(_)));
    assert_eq!(
        plugged,
        Update::Pointing(PointingDevices {
            trackpad: true,
            mouse: true
        })
    );
}
