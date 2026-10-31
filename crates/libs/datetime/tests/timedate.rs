#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::mpsc;
use std::time::Duration;

use unbound_datetime::timedate::{TimedateProxyBlocking, zone_changes};
use unbound_test_bus::SessionBus;
use unbound_test_bus::timedate::{FakeTimedate, REFUSED_ZONE};

const WAIT: Duration = Duration::from_secs(5);

#[test]
fn a_new_time_zone_is_announced_and_a_refused_one_is_not() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let _timedated = FakeTimedate::start(&bus, "Europe/Stockholm").expect("fake timedated");
    let zones = zone_changes(&bus.connect().expect("connection")).expect("follow");
    let (sender, announced) = mpsc::channel();
    std::thread::spawn(move || {
        for zone in zones {
            if sender.send(zone).is_err() {
                return;
            }
        }
    });

    let client = bus.connect().expect("connection");
    let timedate = TimedateProxyBlocking::new(&client).expect("proxy");
    assert!(timedate.set_timezone(REFUSED_ZONE, true).is_err());
    timedate.set_ntp(false, true).expect("SetNTP");
    timedate
        .set_timezone("Europe/Paris", true)
        .expect("SetTimezone");

    assert_eq!(announced.recv_timeout(WAIT).as_deref(), Ok("Europe/Paris"));
}
