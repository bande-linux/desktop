#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Role, TreeId};
use bande_packages::Client;
use bande_settings::{
    Command, Feed, Package, PaneId, Refresh, SettingsView, SoftwareCommand, SoftwareState,
    UpdateNotice, announce_updates, check, feed, follow_software, open_updates,
};
use bande_test_bus::SessionBus;
use bande_test_bus::packagekit::{
    FakePackage, FakePackageKit, ONLY_DOWNLOAD, ONLY_TRUSTED, Script,
};
use bande_toolkit::{Palette, Scheme, TextSystem};
use support::{WAIT, find, has_text};

const FIREFOX: &str = "firefox-esr;128.4.0;arm64;debian-security";
const KERNEL: &str = "linux-image-asahi;6.12.9;arm64;asahi";

fn updates() -> Script {
    Script {
        updates: vec![
            FakePackage::new(8, FIREFOX, "Web browser", 70_000_000),
            FakePackage::new(5, KERNEL, "Linux kernel", 120_000_000),
        ],
        since_refresh: u32::MAX,
        ..Script::default()
    }
}

struct Settings {
    view: SettingsView,
    feed: Feed,
    sender: Sender<SoftwareCommand>,
}

impl Settings {
    fn open(system: &zbus::blocking::Connection) -> Self {
        let (feed, updates) = feed().expect("feed");
        let sender = follow_software(system, updates);
        let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
        view.select_pane(PaneId::SoftwareUpdate);
        Self { view, feed, sender }
    }

    fn until(&mut self, done: impl Fn(&mut SettingsView) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            for update in self.feed.drain() {
                self.view.apply(update);
            }
            for command in self.view.take_commands() {
                if let Command::Software(command) = command {
                    self.sender.send(command).expect("software worker");
                }
            }
            if done(&mut self.view) {
                return;
            }
            assert!(Instant::now() < deadline, "the view never got there");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn row(&mut self, label: &str) {
        self.until(|view| find(view, Role::Row, label).is_some());
    }

    fn click(&mut self, button: &str) {
        let target = find(&mut self.view, Role::Button, button).expect(button);
        self.view.perform(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: target,
            data: None,
        });
    }
}

fn start(script: Script) -> (SessionBus, FakePackageKit, zbus::blocking::Connection) {
    let bus = SessionBus::start().expect("dbus-daemon");
    let fake = FakePackageKit::start(&bus, script).expect("fake PackageKit");
    let system = bus.connect().expect("connection");
    (bus, fake, system)
}

#[test]
fn updates_download_then_wait_for_a_restart_that_only_the_user_starts() {
    let (_bus, fake, system) = start(updates());
    assert_eq!(
        check(&Client::new(&system), Refresh::Always),
        SoftwareState::Available(vec![
            Package {
                id: FIREFOX.to_owned(),
                name: "firefox-esr".to_owned(),
                version: "128.4.0".to_owned(),
                summary: "Web browser".to_owned(),
                size: 70_000_000,
            },
            Package {
                id: KERNEL.to_owned(),
                name: "linux-image-asahi".to_owned(),
                version: "6.12.9".to_owned(),
                summary: "Linux kernel".to_owned(),
                size: 120_000_000,
            },
        ])
    );

    let mut settings = Settings::open(&system);
    settings.row("2 updates are available, 190 MB to download");
    assert!(find(&mut settings.view, Role::Row, "linux-image-asahi, 6.12.9").is_some());
    settings.click("Download Updates");
    settings.row("Restart to install 2 updates, Updates install while the computer restarts");
    let downloads = fake.called("UpdatePackages");
    assert_eq!(downloads.len(), 1);
    assert_eq!(downloads[0].flags, ONLY_TRUSTED | ONLY_DOWNLOAD);
    assert_eq!(downloads[0].args, [FIREFOX.to_owned(), KERNEL.to_owned()]);
    assert_eq!(fake.called("Trigger").len(), 1);
    assert_eq!(fake.called("Trigger")[0].args, ["reboot".to_owned()]);
    assert_eq!(
        settings.view.take_commands(),
        Vec::<Command>::new(),
        "nothing restarts without asking"
    );
    settings.click("Restart Now");
    assert_eq!(
        settings.view.take_commands(),
        vec![Command::Software(SoftwareCommand::Restart)]
    );
}

#[test]
fn a_prepared_update_shows_as_prepared_when_settings_opens() {
    let (_bus, fake, system) = start(Script {
        prepared: vec![FIREFOX.to_owned(), KERNEL.to_owned()],
        triggered: true,
        ..updates()
    });
    let mut settings = Settings::open(&system);
    settings.row("Restart to install 2 updates, Updates install while the computer restarts");
    assert!(fake.called("RefreshCache").is_empty());
}

#[test]
fn results_after_the_restart_are_shown_once_and_cleared() {
    let (_bus, fake, system) = start(Script {
        results: Some((
            true,
            vec![FIREFOX.to_owned(), KERNEL.to_owned()],
            0,
            String::new(),
        )),
        ..updates()
    });
    let client = Client::new(&system);
    assert_eq!(open_updates(&system, &client), SoftwareState::Installed(2));
    assert_eq!(fake.called("ClearResults").len(), 1);
    assert!(matches!(
        open_updates(&system, &client),
        SoftwareState::Available(_)
    ));
    assert_eq!(fake.called("ClearResults").len(), 1);

    fake.script(|script| script.results = Some((false, Vec::new(), 46, "disk full".to_owned())));
    let mut settings = Settings::open(&system);
    settings.row(
        "Could not install updates, There isn't enough free space. Free up some space and try again.",
    );
    assert!(!has_text(&mut settings.view, "disk full"));
    assert_eq!(fake.called("ClearResults").len(), 2);
}

#[test]
fn opening_settings_refreshes_only_a_stale_cache_and_check_now_always_does() {
    let (_bus, fake, system) = start(Script {
        since_refresh: u32::MAX,
        ..Script::default()
    });
    let client = Client::new(&system);
    assert_eq!(open_updates(&system, &client), SoftwareState::UpToDate);
    assert_eq!(open_updates(&system, &client), SoftwareState::UpToDate);
    assert_eq!(fake.called("RefreshCache").len(), 1);
    assert_eq!(check(&client, Refresh::Always), SoftwareState::UpToDate);
    assert_eq!(fake.called("RefreshCache").len(), 2);
}

#[test]
fn without_packagekit_the_page_says_so_plainly_and_offers_no_retry() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let system = bus.connect().expect("connection");
    assert_eq!(
        check(&Client::new(&system), Refresh::Always),
        SoftwareState::Unavailable
    );

    let mut settings = Settings::open(&system);
    settings.row("Updates aren't available on this computer, Install the packagekit package");
    assert!(find(&mut settings.view, Role::Button, "Try Again").is_none());
    assert!(!has_text(&mut settings.view, "org.freedesktop"));
}

#[test]
fn a_failed_check_and_packagekit_stopping_mid_download_read_as_plain_sentences() {
    let (_bus, fake, system) = start(updates());
    fake.script(|script| {
        script
            .failures
            .insert("RefreshCache".to_owned(), (2, "Network is down".to_owned()));
    });
    let mut settings = Settings::open(&system);
    settings
        .row("Could not check for updates, You're offline. Connect to the internet and try again.");
    fake.script(|script| {
        script.failures.clear();
        script.hang = true;
    });
    settings.click("Try Again");
    settings.until(|view| find(view, Role::Button, "Download Updates").is_some());
    settings.click("Download Updates");
    settings.until(|_| !fake.called("UpdatePackages").is_empty());
    settings.until(|view| has_text(view, "Downloading updates"));
    drop(fake);
    settings.row("Could not download updates, The update service stopped. Try again in a moment.");
    assert!(find(&mut settings.view, Role::Button, "Try Again").is_some());
}

fn click(notifications: &zbus::blocking::Connection, id: u32) {
    notifications
        .emit_signal(
            None::<&str>,
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "ActionInvoked",
            &(id, "default"),
        )
        .expect("click");
}

#[test]
fn a_click_right_after_the_daily_announcement_is_heard() {
    let (bus, _fake, system) = start(updates());
    let notifications = bus.connect().expect("notifications connection");
    let server =
        bande_notify::NotificationServer::start(&notifications, &bus.connect().expect("center"))
            .expect("server");
    let session = bus.connect().expect("session");
    let notice = UpdateNotice::listen(&session).expect("listen");
    let started = Instant::now();
    let id = announce_updates(&session, &system)
        .expect("announce")
        .expect("a notification");
    assert!(started.elapsed() < WAIT);
    let shown = server.active();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].summary, "Software updates are available");

    click(&notifications, id);
    assert!(notice.clicked(id).expect("clicked"));
}

#[test]
fn a_newer_announcement_retires_the_older_notice_and_its_waiter() {
    let (bus, _fake, system) = start(updates());
    let notifications = bus.connect().expect("notifications connection");
    let server =
        bande_notify::NotificationServer::start(&notifications, &bus.connect().expect("center"))
            .expect("server");
    let yesterday = bus.connect().expect("yesterday");
    let older = UpdateNotice::listen(&yesterday).expect("listen");
    let old_id = announce_updates(&yesterday, &system)
        .expect("announce")
        .expect("a notification");
    let waiting = std::thread::spawn(move || older.clicked(old_id).expect("clicked"));

    let today = bus.connect().expect("today");
    let newer = UpdateNotice::listen(&today).expect("listen");
    let new_id = announce_updates(&today, &system)
        .expect("announce")
        .expect("a notification");

    let deadline = Instant::now() + WAIT;
    while !waiting.is_finished() {
        assert!(Instant::now() < deadline, "the older waiter never left");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!waiting.join().expect("join"));
    let deadline = Instant::now() + WAIT;
    while server.active().len() != 1 {
        assert!(
            Instant::now() < deadline,
            "the older notice stayed: {:?}",
            server.active()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    click(&notifications, new_id);
    assert!(newer.clicked(new_id).expect("clicked"));
}
