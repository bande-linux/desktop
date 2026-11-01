#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bande_inbox::Inbox;
use bande_polkit::agent::{Answer, Conversation, Request};
use bande_polkit::bridge::Bridge;
use bande_polkit::dialog::Message;

const WAIT: Duration = Duration::from_secs(5);

fn request(cookie: &str) -> Request {
    Request {
        cookie: cookie.to_owned(),
        action_id: "org.example.same".to_owned(),
        message: "Same action".to_owned(),
        icon_name: String::new(),
        details: HashMap::new(),
        identities: Vec::new(),
        user: "ada".to_owned(),
    }
}

fn next_message(inbox: &mut Inbox<Message>) -> Message {
    let until = Instant::now() + WAIT;
    loop {
        if let Some(message) = inbox.drain().into_iter().next() {
            return message;
        }
        assert!(Instant::now() < until, "no message from the bridge");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_request_withdrawn_while_waiting_its_turn_never_blocks_the_next_one() {
    let mut inbox = Inbox::new().expect("inbox");
    let bridge = Arc::new(Bridge::new(inbox.outbox()));
    let (first, second, third) = (request("first"), request("second"), request("third"));

    let asking = Arc::clone(&bridge);
    let shown = first.clone();
    let first_answer = thread::spawn(move || asking.respond(&shown, "ada", "Password:", false));
    let Message::Prompt { reply, .. } = next_message(&mut inbox) else {
        panic!("the first request is shown");
    };

    let (answered, second_answer) = mpsc::channel();
    let waiting = Arc::clone(&bridge);
    let queued = second.clone();
    thread::spawn(move || {
        let answer = waiting.respond(&queued, "ada", "Password:", false);
        let _ = answered.send(matches!(answer, Answer::Cancel));
    });
    thread::sleep(Duration::from_millis(100));
    bridge.close(&second);
    assert_eq!(second_answer.recv_timeout(WAIT), Ok(true));

    reply.send(Answer::Cancel).expect("answer");
    first_answer.join().expect("first");
    bridge.close(&first);
    assert!(matches!(next_message(&mut inbox), Message::Close));

    let asking = Arc::clone(&bridge);
    thread::spawn(move || asking.respond(&third, "ada", "Password:", false));
    assert!(matches!(
        next_message(&mut inbox),
        Message::Prompt { .. }
    ));
}
