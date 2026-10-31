#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use unbound_config::{ConfigService, ConfigStore};
use unbound_settings::{Command, Feed, SettingId, Update, Value, feed, follow_config, worker};
use unbound_test_bus::SessionBus;

const WAIT: Duration = Duration::from_secs(5);

fn wait_for(feed: &mut Feed, wanted: impl Fn(&Update) -> bool) -> Update {
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
    let path = directory.path().join("unbound").join("config.kdl");
    let service_connection = bus.connect().expect("service connection");
    let _service = ConfigService::start(
        &service_connection,
        &path,
        Arc::new(ConfigStore::default()),
        |_| {},
    )
    .expect("config service");
    let session = bus.connect().expect("client connection");
    let (mut feed, updates) = feed().expect("feed");
    follow_config(&session, updates.clone());
    let commands = worker(&session, updates);

    let dark = Value::Keyword("dark".to_owned());
    let light = Value::Keyword("light".to_owned());
    wait_for(&mut feed, |update| {
        has(
            update,
            SettingId::Appearance,
            &Value::Keyword("automatic".to_owned()),
        )
    });

    std::fs::write(&path, "appearance \"dark\"\n").expect("edit by hand");
    wait_for(&mut feed, |update| {
        has(update, SettingId::Appearance, &dark)
    });

    commands
        .send(Command::Set(SettingId::Appearance, light.clone()))
        .expect("worker");
    wait_for(&mut feed, |update| {
        has(update, SettingId::Appearance, &light)
    });

    commands
        .send(Command::Set(SettingId::KeyRepeatRate, Value::Number(500)))
        .expect("worker");
    wait_for(&mut feed, |update| {
        matches!(update, Update::Failed(Some(SettingId::KeyRepeatRate), _))
    });
}
