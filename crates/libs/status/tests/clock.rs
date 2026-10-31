#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use unbound_datetime::timedate::TimedateProxyBlocking;
use unbound_inbox::Inbox;
use unbound_status::Update;
use unbound_status::clock::follow_zone;
use unbound_test_bus::SessionBus;
use unbound_test_bus::timedate::FakeTimedate;

const PROMPTLY: Duration = Duration::from_secs(2);

fn minute_within(inbox: &mut Inbox<Update>, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    loop {
        if inbox
            .drain()
            .into_iter()
            .any(|update| matches!(update, Update::Minute))
        {
            return true;
        }
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        poll(
            &mut [PollFd::new(&inbox.fd(), PollFlags::IN)],
            Some(&Timespec::try_from(left).unwrap()),
        )
        .unwrap();
    }
}

#[test]
fn the_clock_redraws_as_soon_as_the_time_zone_changes() {
    let bus = SessionBus::start().unwrap();
    let _timedated = FakeTimedate::start(&bus, "Europe/Stockholm").unwrap();
    let mut inbox = Inbox::new().unwrap();
    follow_zone(&bus.connect().unwrap(), inbox.outbox()).unwrap();
    let client = bus.connect().unwrap();
    let timedate = TimedateProxyBlocking::new(&client).unwrap();
    timedate.set_timezone("Europe/London", true).unwrap();
    assert!(minute_within(&mut inbox, PROMPTLY));
}
