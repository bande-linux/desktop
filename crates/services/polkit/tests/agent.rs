#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use unbound_polkit::agent::{Agent, Conversation, Notice, OBJECT_PATH, Request, Subject};
use unbound_test_bus::SessionBus;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

const WAIT: Duration = Duration::from_secs(5);
const COOKIE: &str = "3-cookie-1";

#[derive(Debug, PartialEq)]
enum Seen {
    Prompt(String),
    Notice(Notice),
    Closed,
}

struct Scripted {
    answers: Mutex<Vec<Option<String>>>,
    seen: Mutex<Sender<Seen>>,
    hold: Option<Mutex<Receiver<()>>>,
}

impl Conversation for Scripted {
    fn respond(&self, _request: &Request, prompt: &str, _echo: bool) -> Option<String> {
        if let Ok(seen) = self.seen.lock() {
            let _ = seen.send(Seen::Prompt(prompt.to_owned()));
        }
        if let Some(hold) = &self.hold {
            let _ = hold.lock().ok()?.recv_timeout(WAIT);
        }
        let mut answers = self.answers.lock().ok()?;
        if answers.is_empty() {
            None
        } else {
            answers.remove(0)
        }
    }

    fn notice(&self, _request: &Request, notice: Notice) {
        if let Ok(seen) = self.seen.lock() {
            let _ = seen.send(Seen::Notice(notice));
        }
    }

    fn close(&self, _request: &Request) {
        if let Ok(seen) = self.seen.lock() {
            let _ = seen.send(Seen::Closed);
        }
    }
}

struct Registration {
    kind: String,
    object_path: String,
    agent: String,
}

struct FakeAuthority {
    registered: Mutex<Sender<Registration>>,
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.Authority")]
impl FakeAuthority {
    fn register_authentication_agent(
        &self,
        subject: (String, HashMap<String, OwnedValue>),
        _locale: String,
        object_path: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) {
        let agent = header.sender().map(ToString::to_string).unwrap_or_default();
        if let Ok(registered) = self.registered.lock() {
            let _ = registered.send(Registration {
                kind: subject.0,
                object_path,
                agent,
            });
        }
    }

    fn unregister_authentication_agent(
        &self,
        _subject: (String, HashMap<String, OwnedValue>),
        _object_path: String,
    ) {
    }
}

fn fake_helper(directory: &Path) -> PathBuf {
    let path = directory.join("polkit-agent-helper-1");
    let script = format!(
        "#!/bin/sh\n\
         read cookie\n\
         [ \"$cookie\" = \"{COOKIE}\" ] || {{ echo FAILURE; exit 1; }}\n\
         echo 'PAM_TEXT_INFO Authenticating\\040'\"$1\"\n\
         echo 'PAM_PROMPT_ECHO_OFF Password: '\n\
         read password\n\
         if [ \"$password\" = right ]; then echo SUCCESS; else echo FAILURE; fi\n"
    );
    std::fs::write(&path, script).expect("helper script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

struct Fixture {
    _agent: Agent,
    polkitd: Connection,
    seen: Receiver<Seen>,
    agent_name: String,
    release: Option<Sender<()>>,
    _directory: tempfile::TempDir,
    bus: SessionBus,
}

fn start(answers: &[Option<&str>], hold: bool) -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let polkitd = bus.connect().expect("polkitd connection");
    let (registered_sender, registered) = mpsc::channel();
    polkitd
        .object_server()
        .at(
            "/org/freedesktop/PolicyKit1/Authority",
            FakeAuthority {
                registered: Mutex::new(registered_sender),
            },
        )
        .expect("authority");
    polkitd
        .request_name("org.freedesktop.PolicyKit1")
        .expect("polkit name");
    let directory = tempfile::tempdir().expect("temp dir");
    let (seen_sender, seen) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let conversation = Scripted {
        answers: Mutex::new(
            answers
                .iter()
                .map(|answer| answer.map(str::to_owned))
                .collect(),
        ),
        seen: Mutex::new(seen_sender),
        hold: hold.then(|| Mutex::new(held)),
    };
    let agent = Agent::register(
        &bus.connect().expect("agent connection"),
        Subject::Session("c2".to_owned()),
        "C",
        fake_helper(directory.path()),
        Arc::new(conversation),
    )
    .expect("register");
    let registration = registered.recv_timeout(WAIT).expect("registration");
    assert_eq!(registration.kind, "unix-session");
    assert_eq!(registration.object_path, OBJECT_PATH);
    Fixture {
        _agent: agent,
        polkitd,
        seen,
        agent_name: registration.agent,
        release: hold.then_some(release),
        _directory: directory,
        bus,
    }
}

impl Fixture {
    fn begin(&self) -> std::thread::JoinHandle<zbus::Result<()>> {
        self.begin_from(&self.polkitd)
    }

    fn begin_from(&self, caller: &Connection) -> std::thread::JoinHandle<zbus::Result<()>> {
        let polkitd = caller.clone();
        let agent_name = self.agent_name.clone();
        std::thread::spawn(move || {
            let uid = Value::from(rustix::process::getuid().as_raw());
            let identities = vec![("unix-user", HashMap::from([("uid", uid)]))];
            polkitd
                .call_method(
                    Some(agent_name.as_str()),
                    OBJECT_PATH,
                    Some("org.freedesktop.PolicyKit1.AuthenticationAgent"),
                    "BeginAuthentication",
                    &(
                        "org.example.reboot",
                        "Authentication is needed to reboot",
                        "system-reboot",
                        HashMap::<&str, &str>::new(),
                        COOKIE,
                        identities,
                    ),
                )
                .map(drop)
        })
    }

    fn next(&self) -> Seen {
        self.seen.recv_timeout(WAIT).expect("conversation event")
    }
}

fn cancelled(result: zbus::Result<()>) -> bool {
    matches!(
        result,
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.freedesktop.PolicyKit1.Error.Cancelled"
    )
}

fn assert_info(seen: Seen) {
    assert!(
        matches!(&seen, Seen::Notice(Notice::Info(text)) if text.starts_with("Authenticating ")),
        "{seen:?}"
    );
}

#[test]
fn only_polkitd_can_start_an_authentication() {
    let fixture = start(&[Some("right")], false);
    let impostor = fixture.bus.connect().expect("impostor connection");
    let result = fixture.begin_from(&impostor).join().expect("caller");
    assert!(result.is_err());
    assert!(
        fixture
            .seen
            .recv_timeout(Duration::from_millis(300))
            .is_err()
    );
}

#[test]
fn the_right_password_authorizes_and_closes_the_dialog() {
    let fixture = start(&[Some("right")], false);
    let begun = fixture.begin();

    assert_info(fixture.next());
    assert_eq!(fixture.next(), Seen::Prompt("Password: ".to_owned()));
    assert_eq!(fixture.next(), Seen::Closed);
    begun.join().expect("thread").expect("authorized");
}

#[test]
fn a_wrong_password_is_reported_and_asked_again() {
    let fixture = start(&[Some("wrong"), Some("right")], false);
    let begun = fixture.begin();

    assert_info(fixture.next());
    assert_eq!(fixture.next(), Seen::Prompt("Password: ".to_owned()));
    assert_eq!(fixture.next(), Seen::Notice(Notice::WrongPassword));
    assert_info(fixture.next());
    assert_eq!(fixture.next(), Seen::Prompt("Password: ".to_owned()));
    assert_eq!(fixture.next(), Seen::Closed);
    begun.join().expect("thread").expect("authorized");
}

#[test]
fn cancelling_the_dialog_returns_the_cancelled_error() {
    let fixture = start(&[None], false);
    let begun = fixture.begin();

    assert_info(fixture.next());
    assert_eq!(fixture.next(), Seen::Prompt("Password: ".to_owned()));
    assert_eq!(fixture.next(), Seen::Closed);
    assert!(cancelled(begun.join().expect("thread")));
}

#[test]
fn polkitd_can_cancel_a_running_authentication() {
    let mut fixture = start(&[Some("right")], true);
    let begun = fixture.begin();
    assert_info(fixture.next());
    assert_eq!(fixture.next(), Seen::Prompt("Password: ".to_owned()));

    fixture
        .polkitd
        .call_method(
            Some(fixture.agent_name.as_str()),
            OBJECT_PATH,
            Some("org.freedesktop.PolicyKit1.AuthenticationAgent"),
            "CancelAuthentication",
            &COOKIE,
        )
        .expect("CancelAuthentication");
    assert_eq!(fixture.next(), Seen::Closed);
    if let Some(release) = fixture.release.take() {
        let _ = release.send(());
    }
    assert!(cancelled(begun.join().expect("thread")));
}
