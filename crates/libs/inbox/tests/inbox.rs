use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use unbound_inbox::Inbox;

fn wait_readable(inbox: &Inbox<u32>) -> bool {
    let fd = inbox.fd();
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    let timeout = Timespec {
        tv_sec: 5,
        tv_nsec: 0,
    };
    poll(&mut fds, Some(&timeout)).is_ok_and(|ready| ready > 0)
}

#[test]
fn values_sent_from_another_thread_wake_the_loop_and_arrive_in_order() {
    let mut inbox = Inbox::new().expect("inbox");
    let outbox = inbox.outbox();
    std::thread::spawn(move || {
        for value in [1, 2, 3] {
            assert!(outbox.send(value));
        }
    })
    .join()
    .expect("sender");

    assert!(wait_readable(&inbox));
    assert_eq!(inbox.drain(), [1, 2, 3]);
    assert!(inbox.drain().is_empty());
}

#[test]
fn a_watch_reports_only_changes() {
    let mut inbox = Inbox::new().expect("inbox");
    let started = Instant::now();
    inbox.outbox().watch(Duration::from_millis(5), move || {
        u32::from(started.elapsed() > Duration::from_millis(60))
    });

    let mut seen = Vec::new();
    while seen.len() < 2 && wait_readable(&inbox) {
        seen.extend(inbox.drain());
    }
    assert_eq!(seen, [0, 1]);
}
