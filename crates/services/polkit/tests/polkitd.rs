#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};

use unbound_polkit::agent::{Agent, Answer, Conversation, Notice, Request, Subject};
use unbound_polkit::helper::HELPER;
use zbus::blocking::Connection;
use zeroize::Zeroizing;

const ACTION: &str = "org.unbound.polkit-test";
const MESSAGE: &str = "Authentication is required to run the Unbound agent test";

struct Scripted {
    answers: Mutex<Vec<Option<String>>>,
    seen: Mutex<Sender<String>>,
}

impl Conversation for Scripted {
    fn respond(&self, request: &Request, _user: &str, _prompt: &str, _echo: bool) -> Answer {
        if let Ok(seen) = self.seen.lock() {
            let _ = seen.send(format!("prompt {} {}", request.action_id, request.message));
        }
        let Ok(mut answers) = self.answers.lock() else {
            return Answer::Cancel;
        };
        match answers.is_empty() {
            true => Answer::Cancel,
            false => answers.remove(0).map_or(Answer::Cancel, |password| {
                Answer::Password(Zeroizing::new(password))
            }),
        }
    }

    fn notice(&self, _request: &Request, notice: Notice) {
        if notice == Notice::WrongPassword
            && let Ok(seen) = self.seen.lock()
        {
            let _ = seen.send("wrong password".to_owned());
        }
    }

    fn close(&self, _request: &Request) {}
}

struct Target(Child);

impl Drop for Target {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn password() -> String {
    std::env::var("UNBOUND_POLKIT_PASSWORD").expect("UNBOUND_POLKIT_PASSWORD")
}

fn check(answers: Vec<Option<String>>) -> (bool, Receiver<String>) {
    let target = Target(
        Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("target process"),
    );
    let system = Connection::system().expect("system bus");
    let (sender, seen) = mpsc::channel();
    let _agent = Agent::register(
        &system,
        Subject::process(target.0.id()).expect("subject"),
        "C",
        HELPER.into(),
        Arc::new(Scripted {
            answers: Mutex::new(answers),
            seen: Mutex::new(sender),
        }),
    )
    .expect("register with polkitd");
    let authorized = Command::new("pkcheck")
        .args([
            "--action-id",
            ACTION,
            "--allow-user-interaction",
            "--process",
        ])
        .arg(target.0.id().to_string())
        .status()
        .expect("pkcheck")
        .success();
    (authorized, seen)
}

#[test]
#[ignore = "needs polkitd on the system bus; ci/polkit runs it in a Debian container"]
fn polkitd_authorizes_after_the_agent_authenticates_the_user() {
    let (authorized, seen) = check(vec![Some(password())]);
    assert!(authorized);
    assert_eq!(
        seen.try_recv().expect("prompt"),
        format!("prompt {ACTION} {MESSAGE}")
    );
}

#[test]
#[ignore = "needs polkitd on the system bus; ci/polkit runs it in a Debian container"]
fn a_wrong_password_is_retried_until_the_right_one() {
    let (authorized, seen) = check(vec![Some("not the password".to_owned()), Some(password())]);
    assert!(authorized);
    let seen: Vec<String> = seen.try_iter().collect();
    assert_eq!(seen.len(), 3, "{seen:?}");
    assert_eq!(seen[1], "wrong password");
}

#[test]
#[ignore = "needs polkitd on the system bus; ci/polkit runs it in a Debian container"]
fn cancelling_leaves_the_action_unauthorized() {
    let (authorized, _) = check(vec![None]);
    assert!(!authorized);
}
