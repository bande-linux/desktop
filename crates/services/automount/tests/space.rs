#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use bande_automount::space::{Space, SpaceWatch, is_low, watch_space};
use bande_notify::client::NotificationCenter;
use bande_notify::{NotificationServer, Presentation};
use bande_test_bus::SessionBus;

const GB: u64 = 1_000_000_000;

#[test]
fn space_runs_low_under_a_gigabyte_or_under_five_percent_and_ten_gigabytes() {
    assert!(is_low(2_000 * GB, GB - 1));
    assert!(is_low(2_000 * GB, 9 * GB));
    assert!(!is_low(2_000 * GB, 11 * GB));
    assert!(is_low(64 * GB, 3 * GB));
    assert!(!is_low(64 * GB, 4 * GB));
    assert!(!is_low(0, 0));
}

fn notices(center: &NotificationCenter<'_>) -> Vec<(u32, String, String)> {
    let presentation = Presentation::from_wire(center.get_presentation().expect("presentation"));
    let mut shown: Vec<(u32, String, String)> = presentation
        .center
        .iter()
        .chain(&presentation.banners)
        .flat_map(|stack| &stack.notifications)
        .map(|notice| (notice.id, notice.summary.clone(), notice.body.clone()))
        .collect();
    shown.sort();
    shown.dedup();
    shown
}

#[test]
fn a_full_disk_warns_once_until_space_recovers_and_manage_storage_opens_settings() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let _server = NotificationServer::start(&bus.connect().expect("server")).expect("server");
    let center_bus = bus.connect().expect("center");
    let center = NotificationCenter::new(&center_bus).expect("center");
    let available = Arc::new(AtomicU64::new(50 * GB));
    let measured = Arc::clone(&available);
    let (opened, opens) = mpsc::channel();
    let watch = watch_space(
        SpaceWatch::new(
            &bus.connect().expect("session"),
            &[PathBuf::from("/"), PathBuf::from("/")],
            move |_| {
                Some(Space {
                    total: 100 * GB,
                    available: measured.load(Ordering::SeqCst),
                })
            },
        ),
        move || {
            let _ = opened.send(());
        },
    );
    watch.check();
    assert!(notices(&center).is_empty());

    available.store(GB / 2, Ordering::SeqCst);
    watch.check();
    watch.check();
    let shown = notices(&center);
    assert_eq!(shown.len(), 1);
    let (id, summary, body) = &shown[0];
    assert_eq!(summary, "Your disk is almost full");
    assert!(body.contains("500 MB"), "{body}");

    center.activate(*id, "manage-storage", "").expect("click");
    opens
        .recv_timeout(Duration::from_secs(3))
        .expect("Storage settings opened");

    available.store(50 * GB, Ordering::SeqCst);
    watch.check();
    available.store(GB / 2, Ordering::SeqCst);
    watch.check();
    assert!(notices(&center).iter().any(|(again, ..)| again != id));
}
