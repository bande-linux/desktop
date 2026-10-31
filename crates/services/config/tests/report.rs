#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use bande_config::{ConfigService, ConfigStore, EDIT_ACTION, ErrorNotifier};
use bande_notify::client::NotificationCenter;
use bande_notify::{Notification, NotificationServer};
use bande_test_bus::SessionBus;
use support::replace_file;

const WAIT: Duration = Duration::from_secs(5);

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
    _service: ConfigService,
    notifications: NotificationServer,
    center: NotificationCenter<'static>,
    opened: Receiver<PathBuf>,
    _bus: SessionBus,
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
        move |result| notifier.report(result.as_ref().map(|loaded| loaded.problems.as_slice())),
    )
    .expect("config service");
    Fixture {
        _directory: directory,
        path,
        _service: service,
        notifications,
        center: NotificationCenter::new(&connection).expect("center proxy"),
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
    let fixture = start("bar {\n    edge \"middle\"\n}\n");

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
    replace_file(&fixture.path, "bar {\n    edge \"middle\"\n}\n");
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
fn unknown_settings_are_reported_once_while_the_rest_applies() {
    let fixture = start(
        "appearance \"dark\"\ndisplay \"Virtual-1\" {\n    scale 1.5\n}\nsparkle #true\nbar {\n    size 44\n}\n",
    );

    let shown = fixture.wait_for(|active| active.len() == 1)[0].clone();
    assert_eq!(shown.summary, "config.kdl has an error");
    assert_eq!(
        shown.body,
        "Line 5, column 1: unknown setting `sparkle`. Every other setting is in effect."
    );
    assert!(fixture.notifications.dismiss(shown.id).expect("dismiss"));
    fixture.wait_for(<[Notification]>::is_empty);

    replace_file(&fixture.path, "appearance \"light\"\nsparkle #true\n");
    replace_file(
        &fixture.path,
        "appearance \"light\"\nsparkle #true\ntheme \"graphite\"\n",
    );
    let more = fixture.wait_for(|active| active.len() == 1);
    assert_eq!(more[0].summary, "config.kdl has 2 errors");
    assert_eq!(
        more[0].body,
        "Line 2, column 1: unknown setting `sparkle`. One more line has an error. Every other setting is in effect."
    );
}

#[test]
fn open_in_editor_opens_the_configuration_file() {
    let fixture = start("bar {\n    edge \"middle\"\n}\n");
    let id = fixture.wait_for(|active| active.len() == 1)[0].id;

    assert!(
        fixture
            .center
            .activate(id, EDIT_ACTION, "")
            .expect("activate")
    );
    assert_eq!(
        fixture.opened.recv_timeout(WAIT).expect("opened"),
        fixture.path
    );
}
