#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::time::Duration;

use bande_login::manager::{Action, GREETER_USER, Manager};
use bande_login::protocol::{
    AuthMessageType, Class, ErrorType, FromWorker, Request, Response, ToWorker, read_message,
    write_message,
};
use bande_login::{DESKTOP_SHOWN, Relay, ShownWatch};
use bande_pam::Conversation;
use zeroize::Zeroizing;

const GREETER: u64 = 1;
const ADA: u64 = 2;

fn at(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

fn greeter_spawned(worker: u64) -> Action {
    Action::Spawn {
        worker,
        class: Class::Greeter,
        username: GREETER_USER.to_owned(),
    }
}

fn signed_in(manager: &mut Manager) {
    assert_eq!(manager.start(at(0)), vec![greeter_spawned(GREETER)]);
    let created = manager.request(Request::CreateSession {
        username: "ada".to_owned(),
    });
    assert_eq!(
        created,
        vec![Action::Spawn {
            worker: ADA,
            class: Class::User,
            username: "ada".to_owned()
        }]
    );
    let fingerprint = FromWorker::Show {
        text: "Place your finger on the reader".to_owned(),
        error: false,
    };
    assert_eq!(
        manager.worker_said(ADA, fingerprint),
        vec![Action::Reply(Response::AuthMessage {
            auth_message_type: AuthMessageType::Info,
            auth_message: "Place your finger on the reader".to_owned(),
        })]
    );
    let password = FromWorker::Ask {
        text: "Password:".to_owned(),
        visible: false,
    };
    assert!(
        manager.worker_said(ADA, password).is_empty(),
        "the prompt waits until the login window has acknowledged the message before it"
    );
    let acknowledged = manager.request(Request::PostAuthMessageResponse { response: None });
    assert_eq!(
        acknowledged,
        vec![Action::Reply(Response::AuthMessage {
            auth_message_type: AuthMessageType::Secret,
            auth_message: "Password:".to_owned(),
        })],
        "acknowledging a message is never passed to PAM as an answer"
    );
    let secret = Some(Zeroizing::new("hunter2".to_owned()));
    let answered = manager.request(Request::PostAuthMessageResponse {
        response: secret.clone(),
    });
    assert_eq!(
        answered,
        vec![Action::Tell(ADA, ToWorker::Answer { response: secret })]
    );
    assert_eq!(
        manager.worker_said(ADA, FromWorker::Authenticated),
        vec![Action::Reply(Response::Success)]
    );
    let started = manager.request(Request::StartSession {
        env: vec![
            "BANDE_LOGIN_ACCESSIBILITY=screen-reader".to_owned(),
            "LD_PRELOAD=/tmp/evil.so".to_owned(),
            "XDG_SESSION_CLASS=manager".to_owned(),
        ],
    });
    assert_eq!(
        started,
        vec![Action::Tell(
            ADA,
            ToWorker::Launch {
                env: vec!["BANDE_LOGIN_ACCESSIBILITY=screen-reader".to_owned()]
            }
        )],
        "the login window may only pass on its own BANDE_LOGIN_ variables"
    );
}

#[test]
fn the_login_window_stays_until_the_desktop_is_shown_and_returns_after_logging_out() {
    let mut manager = Manager::default();
    signed_in(&mut manager);
    assert_eq!(
        manager.worker_said(ADA, FromWorker::Shown),
        vec![Action::Reply(Response::Success), Action::Stop(GREETER)],
        "the login window is stopped only once the desktop's first frame is on screen"
    );
    assert!(manager.worker_ended(GREETER, at(20)).is_empty());
    assert_eq!(
        manager.worker_ended(ADA, at(3600)),
        vec![greeter_spawned(3)],
        "logging out brings the login window back"
    );

    let mut manager = Manager::default();
    signed_in(&mut manager);
    assert_eq!(
        manager.worker_ended(ADA, at(30)),
        vec![
            Action::ShowGreeter,
            Action::Reply(Response::error(
                ErrorType::Error,
                "the session ended before it was shown"
            )),
        ],
        "a session that never shows hands the screen back to the login window, still running"
    );

    let mut manager = Manager::default();
    manager.start(at(0));
    manager.request(Request::CreateSession {
        username: "ada".to_owned(),
    });
    assert_eq!(
        manager.request(Request::CancelSession),
        vec![Action::Kill(ADA), Action::Reply(Response::Success)],
        "cancelling ends a conversation at once, even while a PAM module is busy"
    );
    assert_eq!(
        manager.worker_ended(GREETER, at(1)),
        vec![greeter_spawned(3)]
    );
    assert_eq!(manager.worker_ended(3, at(2)), vec![greeter_spawned(4)]);
    assert_eq!(
        manager.worker_ended(4, at(3)),
        vec![Action::Exit { success: false }],
        "a login window that keeps failing hands over to the console fallback"
    );
}

#[test]
fn the_pam_conversation_crosses_the_socket_and_oversized_messages_are_refused() {
    let (worker, daemon) = UnixStream::pair().unwrap();
    let login = std::thread::spawn(move || {
        let mut daemon = daemon;
        let shown: FromWorker = read_message(&mut daemon).unwrap();
        let asked: FromWorker = read_message(&mut daemon).unwrap();
        let answer = Some(Zeroizing::new("hunter2".to_owned()));
        write_message(&mut daemon, &ToWorker::Answer { response: answer }).unwrap();
        (shown, asked)
    });
    let mut relay = Relay::new(&worker);
    relay.show("  Your password expires in 3 days\n", false);
    let answer = relay.ask("Password: ", false);
    assert_eq!(answer.as_deref().map(String::as_str), Some("hunter2"));
    let (shown, asked) = login.join().unwrap();
    assert_eq!(
        shown,
        FromWorker::Show {
            text: "Your password expires in 3 days".to_owned(),
            error: false
        }
    );
    assert_eq!(
        asked,
        FromWorker::Ask {
            text: "Password:".to_owned(),
            visible: false
        }
    );
    assert!(
        relay.ask("Password: ", false).is_none(),
        "a daemon that has gone away ends the conversation"
    );

    let mut oversized = Vec::new();
    oversized.extend_from_slice(&(1u32 << 20).to_ne_bytes());
    assert!(read_message::<Request>(&mut oversized.as_slice()).is_err());
    let huge = Request::CreateSession {
        username: "a".repeat(1 << 20),
    };
    assert!(write_message(&mut Vec::new(), &huge).is_err());
}

#[test]
fn the_desktops_first_frame_is_seen_but_a_mark_left_by_an_earlier_session_is_not() {
    let runtime = tempfile::tempdir().unwrap();
    let mark = runtime.path().join(DESKTOP_SHOWN);
    std::fs::write(&mark, b"").unwrap();
    let watch = ShownWatch::new(runtime.path()).unwrap();
    assert!(!mark.exists());
    assert!(!watch.is_shown().unwrap());
    std::fs::write(&mark, b"").unwrap();
    let mut ready = [rustix::event::PollFd::from_borrowed_fd(
        watch.fd(),
        rustix::event::PollFlags::IN,
    )];
    let timeout = rustix::event::Timespec {
        tv_sec: 5,
        tv_nsec: 0,
    };
    assert_eq!(rustix::event::poll(&mut ready, Some(&timeout)).unwrap(), 1);
    assert!(watch.is_shown().unwrap());
}
