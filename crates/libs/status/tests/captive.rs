#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bande_notify::client::NotificationCenter;
use bande_notify::{NotificationServer, Presentation};
use bande_status::captive::follow_captive_portal;
use bande_test_bus::SessionBus;
use bande_test_bus::networkmanager::{CHECK_URI, CONNECTIVITY_PORTAL, FakeNetworkManager};

const PROMPTLY: Duration = Duration::from_secs(3);
const CONNECTIVITY_FULL: u32 = 4;

fn summaries(center: &NotificationCenter<'_>) -> Vec<(u32, String)> {
    let presentation = Presentation::from_wire(center.get_presentation().expect("presentation"));
    let mut shown: Vec<(u32, String)> = presentation
        .center
        .iter()
        .chain(&presentation.banners)
        .flat_map(|stack| &stack.notifications)
        .map(|notification| (notification.id, notification.summary.clone()))
        .collect();
    shown.sort();
    shown.dedup();
    shown
}

fn until<T>(mut found: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + PROMPTLY;
    loop {
        if let Some(value) = found() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_captive_portal_posts_one_sign_in_notice_that_opens_the_page_and_goes_away_once_online() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let network = FakeNetworkManager::start(&bus).expect("fake NetworkManager");
    let _server = NotificationServer::start(&bus.connect().expect("server")).expect("server");
    let client = bus.connect().expect("client");
    let center = NotificationCenter::new(&client).expect("center");
    let (opened, pages) = mpsc::channel();
    follow_captive_portal(
        &bus.connect().expect("system"),
        &bus.connect().expect("session"),
        move |page| {
            let _ = opened.send(page.to_owned());
        },
    );
    thread::sleep(Duration::from_millis(200));
    assert!(summaries(&center).is_empty());

    network
        .set_connectivity(CONNECTIVITY_PORTAL)
        .expect("portal");
    let (id, summary) = until(|| summaries(&center).into_iter().next());
    assert_eq!(summary, "Sign in to Home");
    network
        .set_connectivity(CONNECTIVITY_PORTAL)
        .expect("still portal");
    thread::sleep(Duration::from_millis(200));
    assert_eq!(summaries(&center).len(), 1);

    center.activate(id, "default", "").expect("click");
    assert_eq!(
        pages.recv_timeout(PROMPTLY).expect("page opened"),
        CHECK_URI
    );

    network.set_connectivity(CONNECTIVITY_FULL).expect("online");
    thread::sleep(Duration::from_millis(200));
    network
        .set_connectivity(CONNECTIVITY_PORTAL)
        .expect("portal again");
    until(|| (!summaries(&center).is_empty()).then_some(()));
    network
        .set_connectivity(CONNECTIVITY_FULL)
        .expect("online again");
    until(|| summaries(&center).is_empty().then_some(()));
}
