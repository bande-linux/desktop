use std::sync::mpsc;
use std::time::Duration;

use bande_inbox::Inbox;
use rustix::event::{PollFd, PollFlags, Timespec, poll};

const SENDS: u32 = 200_000;

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
fn a_loop_that_is_not_draining_never_blocks_the_sender() {
    let mut inbox = Inbox::new().expect("inbox");
    let outbox = inbox.outbox();
    let (finished, sent) = mpsc::channel();
    std::thread::spawn(move || {
        let delivered = (0..SENDS).all(|value| outbox.send(value));
        let _ = finished.send(delivered);
    });

    assert_eq!(sent.recv_timeout(Duration::from_secs(5)), Ok(true));
    assert!(wait_readable(&inbox));
    assert_eq!(inbox.drain().len(), SENDS as usize);
}
