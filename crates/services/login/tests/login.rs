#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use bande_login::manager::{Action, GREETER_USER, Manager};
use bande_login::protocol::{
    AuthMessageType, Class, Deadline, ErrorType, FromWorker, Request, Response, ToWorker,
    read_message, write_message,
};
use bande_login::{Relay, ShownWatch, login_shell, refusal};
use bande_pam::Conversation;
use bande_xdg::DESKTOP_SHOWN;
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
fn letting_go_of_a_signed_in_person_deletes_their_credentials() {
    let mut manager = Manager::default();
    signed_in(&mut manager);
    assert_eq!(
        manager.request(Request::CancelSession),
        vec![Action::Stop(ADA), Action::Reply(Response::Success)],
        "a session that is starting is ended in order"
    );

    let mut manager = Manager::default();
    manager.start(at(0));
    manager.request(Request::CreateSession {
        username: "ada".to_owned(),
    });
    manager.worker_said(ADA, FromWorker::Authenticated);
    assert_eq!(
        manager.request(Request::CancelSession),
        vec![Action::Stop(ADA), Action::Reply(Response::Success)],
        "a worker that has established credentials is let go, so it deletes them, not killed"
    );

    let mut manager = Manager::default();
    manager.start(at(0));
    manager.request(Request::CreateSession {
        username: "ada".to_owned(),
    });
    manager.worker_said(ADA, FromWorker::Authenticated);
    assert_eq!(
        manager.disconnected(),
        vec![Action::Stop(ADA)],
        "a login window that goes away after signing in lets the worker clean up too"
    );
    assert_eq!(
        manager.terminate(),
        vec![Action::Stop(GREETER)],
        "stopping the login manager ends the login window in order"
    );
}

#[test]
fn a_desktop_that_never_appears_gives_the_screen_back_and_allows_signing_in_again() {
    let mut manager = Manager::default();
    signed_in(&mut manager);
    let gave_up = FromWorker::Failed {
        error_type: ErrorType::Error,
        description: "the desktop did not appear within 20 s".to_owned(),
    };
    assert_eq!(
        manager.worker_said(ADA, gave_up),
        vec![
            Action::Reply(Response::error(
                ErrorType::Error,
                "the desktop did not appear within 20 s"
            )),
            Action::ShowGreeter,
        ]
    );
    assert_eq!(
        manager.request(Request::CreateSession {
            username: "ada".to_owned(),
        }),
        vec![Action::Spawn {
            worker: 3,
            class: Class::User,
            username: "ada".to_owned()
        }],
        "the login window accepts a new attempt while the old session is still ending"
    );
}

#[test]
fn root_and_accounts_without_a_login_shell_get_no_desktop() {
    let shells = "# /etc/shells: valid login shells\n/bin/sh\n/usr/bin/bash\n/usr/bin/fish\n";
    let bash = Path::new("/usr/bin/bash");
    assert_eq!(refusal(1000, bash, Some(shells)), None);
    assert!(
        refusal(0, bash, Some(shells)).is_some(),
        "uid 0 under any name"
    );
    for shell in ["/usr/sbin/nologin", "/bin/false", "/usr/bin/bash-evil"] {
        assert!(
            refusal(1000, Path::new(shell), Some(shells)).is_some(),
            "{shell}"
        );
    }
    assert_eq!(login_shell(Path::new("")), Path::new("/bin/sh"));
    assert_eq!(refusal(1000, Path::new(""), Some(shells)), None);
    assert_eq!(refusal(1000, Path::new("/bin/sh"), None), None);
    assert!(refusal(1000, bash, None).is_some());
}

#[test]
fn a_login_window_that_sends_slowly_cannot_hold_bande_login() {
    let (daemon, greeter) = UnixStream::pair().unwrap();
    daemon.set_nonblocking(true).unwrap();
    let trickle = std::thread::spawn(move || {
        let mut greeter = greeter;
        for byte in 0u32.to_ne_bytes() {
            greeter.write_all(&[byte]).unwrap();
            std::thread::sleep(Duration::from_millis(150));
        }
    });
    let started = Instant::now();
    let read = read_message::<Request>(&mut Deadline::within(&daemon, Duration::from_millis(300)));
    assert_eq!(read.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the deadline covers the whole message, not each read"
    );
    trickle.join().unwrap();
    let (daemon, greeter) = UnixStream::pair().unwrap();
    daemon.set_nonblocking(true).unwrap();
    write_message(&mut &greeter, &Request::CancelSession).unwrap();
    assert_eq!(
        read_message::<Request>(&mut Deadline::within(&daemon, Duration::from_secs(5))).unwrap(),
        Request::CancelSession,
        "a message that arrives in time is read whole"
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
