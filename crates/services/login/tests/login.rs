#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use bande_login::manager::{Action, FAILED_EXIT_LIMIT, GREETER_USER, Manager, RESPAWN_PAUSE};
use bande_login::protocol::{
    AuthMessageType, Class, Deadline, ErrorType, FromWorker, Request, Response, ToWorker,
    read_message, write_message,
};
use bande_login::{FirstFrame, Progress, Relay, ShownWatch, login_shell, refusal};
use bande_pam::Conversation;
use bande_xdg::DESKTOP_SHOWN;
use zeroize::Zeroizing;

const GREETER: u64 = 1;
const ADA: u64 = 2;

fn at(seconds: u64) -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now) + Duration::from_secs(seconds)
}

fn superseded() -> Action {
    Action::Reply(Response::error(
        ErrorType::Error,
        "another request took its place",
    ))
}

fn greeter_spawned(worker: u64) -> Action {
    Action::Spawn {
        worker,
        class: Class::Greeter,
        username: GREETER_USER.to_owned(),
    }
}

fn signed_in(manager: &mut Manager) {
    assert_eq!(manager.start(), vec![greeter_spawned(GREETER)]);
    let created = manager.request(
        Request::CreateSession {
            username: "ada".to_owned(),
        },
        at(1),
    );
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
        manager.worker_said(ADA, fingerprint, at(2)),
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
        manager.worker_said(ADA, password, at(2)).is_empty(),
        "the prompt waits until the login window has acknowledged the message before it"
    );
    let acknowledged = manager.request(Request::PostAuthMessageResponse { response: None }, at(1));
    assert_eq!(
        acknowledged,
        vec![Action::Reply(Response::AuthMessage {
            auth_message_type: AuthMessageType::Secret,
            auth_message: "Password:".to_owned(),
        })],
        "acknowledging a message is never passed to PAM as an answer"
    );
    manager.worker_said(
        ADA,
        FromWorker::Account {
            name: "ada".to_owned(),
        },
        at(2),
    );
    let secret = Some(Zeroizing::new("hunter2".to_owned()));
    let answered = manager.request(
        Request::PostAuthMessageResponse {
            response: secret.clone(),
        },
        at(1),
    );
    assert_eq!(
        answered,
        vec![Action::Tell(ADA, ToWorker::Answer { response: secret })]
    );
    assert_eq!(
        manager.worker_said(ADA, FromWorker::Authenticated, at(2)),
        vec![Action::Reply(Response::Success)]
    );
    let started = manager.request(
        Request::StartSession {
            env: vec![
                "BANDE_LOGIN_ACCESSIBILITY=screen-reader".to_owned(),
                "LD_PRELOAD=/tmp/evil.so".to_owned(),
                "XDG_SESSION_CLASS=manager".to_owned(),
            ],
        },
        at(1),
    );
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
        manager.worker_said(ADA, FromWorker::Shown, at(2)),
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
    manager.start();
    manager.request(
        Request::CreateSession {
            username: "ada".to_owned(),
        },
        at(1),
    );
    assert_eq!(
        manager.request(Request::CancelSession, at(1)),
        vec![
            superseded(),
            Action::Kill(ADA),
            Action::Reply(Response::Success)
        ],
        "cancelling ends a conversation at once, even while a PAM module is busy"
    );
}

fn give_up_after(lives: &[(u64, Option<u64>)]) -> Option<usize> {
    let mut manager = Manager::default();
    let mut actions = manager.start();
    let mut now = 0;
    for (index, (life, shown)) in lives.iter().enumerate() {
        let Some(Action::Spawn { worker, .. }) = actions.pop() else {
            panic!("no login window was started: {actions:?}");
        };
        manager.request(
            Request::CreateSession {
                username: "ada".to_owned(),
            },
            at(now),
        );
        let conversation = worker + 1;
        if let Some(shown) = shown {
            manager.worker_said(worker, FromWorker::Shown, at(now + shown));
        }
        now += life;
        actions = manager.worker_ended(worker, at(now));
        let reaped = manager.worker_ended(conversation, at(now));
        if reaped == vec![Action::Exit { success: false }] {
            return Some(index + 1);
        }
        assert!(reaped.is_empty(), "{reaped:?}");
        if shown.is_none() {
            now += RESPAWN_PAUSE.as_secs();
            actions = manager.tick(at(now));
        }
    }
    None
}

#[test]
fn a_login_window_that_keeps_failing_hands_over_to_the_console_however_slowly_it_fails() {
    for life in [0, 1, 3, 9, 20, 300] {
        assert_eq!(
            give_up_after(&[(life, None); 5]),
            Some(3),
            "never shown, {life} s"
        );
    }
    assert_eq!(
        give_up_after(&[(4, Some(2)); 5]),
        Some(3),
        "shown for an instant each time"
    );
    assert_eq!(
        give_up_after(&[(3600, Some(2)); 5]),
        None,
        "one that stays on screen and ends now and then is brought back each time"
    );
    let mut manager = Manager::default();
    signed_in(&mut manager);
    manager.worker_said(ADA, FromWorker::Shown, at(10));
    manager.worker_ended(GREETER, at(11));
    assert_eq!(
        manager.worker_ended(ADA, at(4000)),
        vec![greeter_spawned(3)]
    );
    for (worker, second) in [(3, 4001), (4, 4004)] {
        assert!(manager.worker_ended(worker, at(second)).is_empty());
        assert_eq!(
            manager.tick(at(second + 2)),
            vec![greeter_spawned(worker + 1)],
            "signing in starts the count again"
        );
    }
}

fn tried_as(
    manager: &mut Manager,
    worker: u64,
    (typed, account): (&str, Option<&str>),
    now: Instant,
) -> Vec<Action> {
    manager.request(
        Request::CreateSession {
            username: typed.to_owned(),
        },
        now,
    );
    if let Some(account) = account {
        let name = account.to_owned();
        manager.worker_said(worker, FromWorker::Account { name }, now);
    }
    let prompt = FromWorker::Ask {
        text: "Password:".to_owned(),
        visible: false,
    };
    manager.worker_said(worker, prompt, now);
    manager.request(
        Request::PostAuthMessageResponse {
            response: Some(Zeroizing::new("guess".to_owned())),
        },
        now,
    )
}

fn tried(manager: &mut Manager, worker: u64, now: Instant) -> Vec<Action> {
    tried_as(manager, worker, ("ada", Some("ada")), now)
}

fn is_answer(action: &Action) -> bool {
    matches!(action, Action::Tell(_, ToWorker::Answer { .. }))
}

#[test]
fn guessing_is_slowed_down_whether_the_answer_is_waited_for_or_cancelled() {
    type End = fn(&mut Manager, u64);
    let ends: [(&str, End); 4] = [
        ("refused", |manager, worker| {
            let refused = FromWorker::Failed {
                error_type: ErrorType::AuthError,
                description: "pam_authenticate: AUTH_ERR".to_owned(),
            };
            manager.worker_said(worker, refused, at(0));
        }),
        ("cancelled", |manager, _| {
            manager.request(Request::CancelSession, at(0));
        }),
        ("disconnected", |manager, _| {
            manager.disconnected(at(0));
        }),
        ("crashed", |manager, worker| {
            manager.worker_ended(worker, at(0));
        }),
    ];
    for (how, end) in ends {
        let mut manager = Manager::default();
        manager.start();
        for worker in 2..7 {
            let actions = tried(&mut manager, worker, at(0));
            assert!(actions.iter().any(is_answer), "{how}: {actions:?}");
            end(&mut manager, worker);
        }
        let throttled = tried(&mut manager, 7, at(1));
        assert!(!throttled.iter().any(is_answer), "{how}: {throttled:?}");
        assert!(
            throttled.contains(&Action::Reply(Response::error(
                ErrorType::AuthError,
                "too many attempts: MAXTRIES"
            ))),
            "{how}: {throttled:?}"
        );
        let later = tried(&mut manager, 8, at(120));
        assert!(
            later.iter().any(is_answer),
            "{how}: the wait ends: {later:?}"
        );
        assert_eq!(
            manager.worker_said(8, FromWorker::Accepted, at(120)),
            vec![Action::Tell(8, ToWorker::Proceed)]
        );
        let cancelled = manager.request(Request::CancelSession, at(120));
        assert!(
            cancelled.contains(&Action::Stop(8)),
            "{how}: a worker PAM has accepted is let go, never killed: {cancelled:?}"
        );
        let after = tried(&mut manager, 9, at(120));
        assert!(
            after.iter().any(is_answer),
            "{how}: signing in forgets the attempts"
        );
    }
}

#[test]
fn guesses_are_counted_for_accounts_under_their_own_name_and_never_forgotten() {
    let mut manager = Manager::default();
    manager.start();
    let mut worker = 1;
    let mut guess = |manager: &mut Manager, names: (&str, Option<&str>), now| {
        worker += 1;
        let actions = tried_as(manager, worker, names, now);
        manager.request(Request::CancelSession, now);
        actions.iter().any(is_answer)
    };
    for typed in ["ada", "Ada", "ada@example.org", "ADA", "ada"] {
        assert!(guess(&mut manager, (typed, Some("ada")), at(0)), "{typed}");
    }
    for round in 0..5 {
        for number in 0..300 {
            let name = format!("nobody{number}");
            assert!(guess(&mut manager, (&name, None), at(0)), "{round} {name}");
            let name = format!("someone{number}");
            guess(&mut manager, (&name, Some(&name)), at(0));
        }
    }
    assert!(
        !guess(&mut manager, ("Ada", Some("ada")), at(1)),
        "no number of other names makes the wait go away"
    );
    for name in ["-x", "+x"] {
        assert_eq!(
            manager.request(
                Request::CreateSession {
                    username: name.to_owned()
                },
                at(2)
            ),
            vec![Action::Reply(Response::error(
                ErrorType::AuthError,
                "pam_start: USER_UNKNOWN"
            ))]
        );
    }
}

#[test]
fn the_console_does_not_wait_forever_for_a_worker_that_hangs() {
    let mut manager = Manager::default();
    let mut started = manager.start();
    let mut now = at(0);
    for _ in 0..2 {
        let Some(Action::Spawn { worker, .. }) = started.pop() else {
            panic!("no login window was started: {started:?}");
        };
        assert!(
            manager.worker_ended(worker, now).is_empty(),
            "a login window that was never shown is not started again at once"
        );
        now += RESPAWN_PAUSE;
        assert_eq!(manager.wake_at(), Some(now));
        started = manager.tick(now);
    }
    tried(&mut manager, 4, now);
    manager.worker_said(4, FromWorker::Accepted, now);
    assert_eq!(manager.worker_ended(3, now), vec![Action::Stop(4)]);
    now += FAILED_EXIT_LIMIT;
    assert_eq!(manager.wake_at(), Some(now));
    assert_eq!(manager.tick(now), vec![Action::Exit { success: false }]);
}

#[test]
fn someone_pam_asked_nothing_signs_in_only_once_they_say_so() {
    let mut manager = Manager::default();
    manager.start();
    manager.request(
        Request::CreateSession {
            username: "ada".to_owned(),
        },
        at(0),
    );
    assert_eq!(
        manager.worker_said(ADA, FromWorker::Accepted, at(0)),
        vec![Action::Reply(Response::AuthMessage {
            auth_message_type: AuthMessageType::Confirm,
            auth_message: String::new(),
        })],
        "an account without a password is not signed in by being shown"
    );
    assert_eq!(
        manager.request(
            Request::PostAuthMessageResponse { response: None },
            at(3600)
        ),
        vec![Action::Tell(ADA, ToWorker::Proceed)]
    );
    assert_eq!(
        manager.worker_said(ADA, FromWorker::Authenticated, at(3600)),
        vec![Action::Reply(Response::Success)]
    );
}

#[test]
fn every_request_is_answered_once_and_cancelling_does_not_wait_for_pam() {
    let mut manager = Manager::default();
    manager.start();
    tried(&mut manager, ADA, at(0));
    let replies = |actions: &[Action]| {
        actions
            .iter()
            .filter(|action| matches!(action, Action::Reply(_)))
            .count()
    };
    let cancelled = manager.request(Request::CancelSession, at(1));
    assert_eq!(
        replies(&cancelled),
        2,
        "the answer's reply and the cancel's"
    );
    assert!(cancelled.contains(&Action::Kill(ADA)), "{cancelled:?}");
    assert_eq!(cancelled.last(), Some(&Action::Reply(Response::Success)));
    assert!(
        manager
            .worker_said(ADA, FromWorker::Authenticated, at(1))
            .is_empty(),
        "what the cancelled worker still says reaches no one"
    );
}

#[test]
fn what_pam_says_last_does_not_hide_how_signing_in_ended() {
    let acknowledged = Request::PostAuthMessageResponse { response: None };
    for (ended, expected) in [
        (FromWorker::Authenticated, Response::Success),
        (
            FromWorker::Failed {
                error_type: ErrorType::AuthError,
                description: "pam_authenticate: AUTH_ERR".to_owned(),
            },
            Response::error(ErrorType::AuthError, "pam_authenticate: AUTH_ERR"),
        ),
    ] {
        let mut manager = Manager::default();
        manager.start();
        tried(&mut manager, ADA, at(0));
        for text in ["Your password expires in 3 days", "Change it soon"] {
            let said = FromWorker::Show {
                text: text.to_owned(),
                error: false,
            };
            manager.worker_said(ADA, said, at(1));
        }
        assert!(manager.worker_said(ADA, ended, at(1)).is_empty());
        assert_eq!(
            manager.request(acknowledged.clone(), at(2)),
            vec![Action::Reply(Response::AuthMessage {
                auth_message_type: AuthMessageType::Info,
                auth_message: "Change it soon".to_owned(),
            })]
        );
        assert_eq!(
            manager.request(acknowledged.clone(), at(2)),
            vec![Action::Reply(expected)]
        );
    }
}

#[test]
fn a_session_called_off_while_starting_gives_the_screen_back_once_it_has_ended() {
    let mut manager = Manager::default();
    signed_in(&mut manager);
    manager.request(Request::CancelSession, at(3));
    assert_eq!(
        manager.worker_ended(ADA, at(4)),
        vec![Action::ShowGreeter],
        "the screen is not asked for while the worker still holds the terminal"
    );
}

#[test]
fn stopping_waits_for_workers_that_are_still_cleaning_up() {
    let mut manager = Manager::default();
    signed_in(&mut manager);
    manager.request(Request::CancelSession, at(3));
    assert_eq!(manager.terminate(at(4)), vec![Action::Stop(GREETER)]);
    assert!(manager.worker_ended(GREETER, at(5)).is_empty());
    assert_eq!(
        manager.worker_ended(ADA, at(6)),
        vec![Action::Exit { success: true }]
    );
}

#[test]
fn letting_go_of_a_signed_in_person_deletes_their_credentials() {
    let mut manager = Manager::default();
    signed_in(&mut manager);
    assert_eq!(
        manager.request(Request::CancelSession, at(1)),
        vec![
            superseded(),
            Action::Stop(ADA),
            Action::Reply(Response::Success)
        ],
        "a session that is starting is ended in order"
    );

    let mut manager = Manager::default();
    manager.start();
    manager.request(
        Request::CreateSession {
            username: "ada".to_owned(),
        },
        at(1),
    );
    manager.worker_said(ADA, FromWorker::Authenticated, at(2));
    assert_eq!(
        manager.request(Request::CancelSession, at(1)),
        vec![Action::Stop(ADA), Action::Reply(Response::Success)],
        "a worker that has established credentials is let go, so it deletes them, not killed"
    );

    let mut manager = Manager::default();
    manager.start();
    manager.request(
        Request::CreateSession {
            username: "ada".to_owned(),
        },
        at(1),
    );
    manager.worker_said(ADA, FromWorker::Authenticated, at(2));
    assert_eq!(
        manager.disconnected(at(5)),
        vec![Action::Stop(ADA)],
        "a login window that goes away after signing in lets the worker clean up too"
    );
    assert_eq!(
        manager.terminate(at(6)),
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
        description: "the desktop stopped starting before it appeared".to_owned(),
    };
    assert_eq!(
        manager.worker_said(ADA, gave_up.clone(), at(2)),
        vec![Action::Reply(Response::error(
            ErrorType::Error,
            "the desktop stopped starting before it appeared"
        )),]
    );
    assert_eq!(
        manager.request(
            Request::CreateSession {
                username: "ada".to_owned(),
            },
            at(1)
        ),
        vec![Action::Spawn {
            worker: 3,
            class: Class::User,
            username: "ada".to_owned()
        }],
        "the login window accepts a new attempt while the old session is still ending"
    );

    let mut manager = Manager::default();
    signed_in(&mut manager);
    manager.worker_said(ADA, gave_up, at(2));
    assert_eq!(
        manager.worker_ended(ADA, at(3)),
        vec![Action::ShowGreeter],
        "the screen is asked for again once the worker has let go of its terminal"
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
    let watch = ShownWatch::new(runtime.path(), DESKTOP_SHOWN).unwrap();
    assert!(!mark.exists());
    assert!(!watch.progress().unwrap().shown);
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
    let progress = watch.progress().unwrap();
    assert!(progress.shown && progress.changed && !progress.compositor_started);
    std::os::unix::net::UnixListener::bind(runtime.path().join("wayland-1")).unwrap();
    std::fs::write(runtime.path().join("wayland-1.lock"), b"").unwrap();
    assert!(watch.progress().unwrap().compositor_started);
}

#[test]
fn a_slow_first_login_is_waited_for_and_only_a_hung_compositor_is_given_up() {
    let spawned = Instant::now();
    let after = |seconds| spawned + Duration::from_secs(seconds);
    let mut first_frame = FirstFrame::new(spawned);
    let busy = Progress {
        changed: true,
        ..Progress::default()
    };
    first_frame.saw(busy, after(60));
    assert_eq!(
        first_frame.deadline(),
        spawned + FirstFrame::CEILING,
        "before the compositor starts, a slow profile has the whole ceiling"
    );
    let started = Progress {
        compositor_started: true,
        changed: true,
        shown: false,
    };
    first_frame.saw(started, after(90));
    assert_eq!(first_frame.deadline(), after(90) + FirstFrame::QUIET);
    first_frame.saw(busy, after(110));
    assert_eq!(
        first_frame.deadline(),
        after(110) + FirstFrame::QUIET,
        "a session that keeps showing progress is waited for"
    );
    first_frame.saw(busy, after(290));
    assert_eq!(first_frame.deadline(), spawned + FirstFrame::CEILING);
}
