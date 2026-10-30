#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use unbound_config::{ConfigService, ConfigStore, EDIT_ACTION, ErrorNotifier};
use unbound_notify::{Notification, NotificationServer};
use unbound_test_bus::SessionBus;

const WAIT: Duration = Duration::from_secs(5);

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
    _service: ConfigService,
    notifications: NotificationServer,
    opened: Receiver<PathBuf>,
    _bus: SessionBus,
}

fn replace_file(path: &Path, contents: &str) {
    let staging = path.with_extension("kdl.tmp");
    std::fs::write(&staging, contents).expect("write staging file");
    std::fs::rename(&staging, path).expect("rename into place");
}

fn start(initial: &str) -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let notifications = NotificationServer::start(&bus.connect().expect("notification connection"))
        .expect("notification server");
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("config.kdl");
    std::fs::write(&path, initial).expect("initial config");

    let connection = bus.connect().expect("service connection");
    let (sender, opened) = mpsc::channel();
    let notifier = ErrorNotifier::start(&connection, &path, move |path| {
        let _ = sender.send(path.to_path_buf());
    })
    .expect("notifier");
    let service = ConfigService::start(
        &connection,
        &path,
        Arc::new(ConfigStore::default()),
        move |result| notifier.report(&result.map(drop)),
    )
    .expect("config service");
    Fixture {
        _directory: directory,
        path,
        _service: service,
        notifications,
        opened,
        _bus: bus,
    }
}

impl Fixture {
    fn wait_for(&self, done: impl Fn(&[Notification]) -> bool) -> Vec<Notification> {
        let deadline = Instant::now() + WAIT;
        loop {
            let active = self.notifications.active();
            if done(&active) {
                return active;
            }
            assert!(Instant::now() < deadline, "still showing {active:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn an_invalid_file_at_start_is_reported_with_its_line_and_column() {
    let fixture = start("dock {\n    edge \"middle\"\n}\n");

    let active = fixture.wait_for(|active| active.len() == 1);
    let shown = &active[0];
    assert_eq!(shown.summary, "config.kdl has an error");
    assert!(
        shown.body.starts_with("Line 2, column 10: "),
        "{}",
        shown.body
    );
    assert_eq!(
        shown.actions,
        [(EDIT_ACTION.to_owned(), "Open in Editor".to_owned())]
    );
}

#[test]
fn a_new_error_replaces_the_notification_and_a_fix_closes_it() {
    let fixture = start("");
    replace_file(&fixture.path, "dock {\n    edge \"middle\"\n}\n");
    let first = fixture.wait_for(|active| active.len() == 1)[0].id;

    replace_file(&fixture.path, "appearance \"dark\"\ntheme \"graphite\"\n");
    let replaced = fixture.wait_for(|active| {
        active
            .iter()
            .any(|shown| shown.body.starts_with("Line 2, column 1: "))
    });
    assert_eq!(replaced.len(), 1);
    assert_eq!(replaced[0].id, first);

    replace_file(&fixture.path, "appearance \"dark\"\n");
    fixture.wait_for(<[Notification]>::is_empty);
}

#[test]
fn open_in_editor_opens_the_configuration_file() {
    let fixture = start("dock {\n    edge \"middle\"\n}\n");
    let id = fixture.wait_for(|active| active.len() == 1)[0].id;

    assert!(
        fixture
            .notifications
            .invoke_action(id, EDIT_ACTION)
            .expect("invoke")
    );
    assert_eq!(
        fixture.opened.recv_timeout(WAIT).expect("opened"),
        fixture.path
    );
}
