#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[allow(dead_code)]
#[path = "../../../platform/compositor/tests/guest/uinput.rs"]
mod uinput;

use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use uinput::{KEY_ENTER, VirtualDevice};

const PERSON: &str = "ada";
const STARTUP: Duration = Duration::from_secs(90);
const TIME_TO_DESKTOP: Duration = Duration::from_secs(3);
const SUBMITTED: &str = "the password is submitted";
const SHOWN: &str = "the desktop is shown";
const REFUSAL: Duration = Duration::from_secs(6);
const KEY_1: u16 = 2;
const KEY_K: u16 = 37;
const KEY_SEMICOLON: u16 = 39;
const KEY_APOSTROPHE: u16 = 40;
const KEY_LEFTBRACE: u16 = 26;

fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < timeout, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(250));
    }
}

fn running(comm: &str) -> bool {
    std::fs::read_dir("/proc").is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            std::fs::read_to_string(entry.path().join("comm"))
                .is_ok_and(|name| name.trim_end() == comm)
        })
    })
}

fn output(program: &str, arguments: &[&str]) -> Option<String> {
    Command::new(program)
        .args(arguments)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|text| !text.is_empty())
}

fn graphical_session(person: &str) -> Option<String> {
    output(
        "loginctl",
        &["show-user", person, "--property=Display", "--value"],
    )
}

fn session_property(session: &str, property: &str) -> Option<String> {
    output(
        "loginctl",
        &[
            "show-session",
            session,
            &format!("--property={property}"),
            "--value",
        ],
    )
}

struct Entry {
    at: Duration,
    identifier: String,
    message: String,
}

fn journal() -> Vec<Entry> {
    let identifiers = [
        "bande-login-test",
        "bande-greeter",
        "bande-compositor",
        "systemd",
    ]
    .map(|identifier| format!("SYSLOG_IDENTIFIER={identifier}"));
    let mut arguments = vec!["--no-pager", "-b", "-o", "json"];
    arguments.extend(identifiers.iter().map(String::as_str));
    output("journalctl", &arguments)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|entry| {
            let field = |name: &str| entry.get(name)?.as_str().map(str::to_owned);
            Some(Entry {
                at: Duration::from_micros(field("__MONOTONIC_TIMESTAMP")?.parse().ok()?),
                identifier: field("SYSLOG_IDENTIFIER")?,
                message: field("MESSAGE")?,
            })
        })
        .collect()
}

fn after(entries: &[Entry], since: Duration, identifier: &str, start: &str) -> Option<Duration> {
    entries
        .iter()
        .find(|entry| {
            entry.at >= since && entry.identifier == identifier && entry.message.starts_with(start)
        })
        .map(|entry| entry.at - since)
}

fn compositor_active(person: &str) -> bool {
    output(
        "systemctl",
        &[
            "--user",
            &format!("--machine={person}@"),
            "is-active",
            "bande-compositor.service",
        ],
    )
    .is_some_and(|state| state == "active")
}

#[test]
#[ignore = "needs the ci/vm guest booted with systemd, greetd and the test person"]
fn greetd_signs_a_person_in_to_the_bande_session_with_the_system_keyboard_layout() {
    wait_for("the login window", STARTUP, || running("bande-greeter"));
    let mut keyboard = VirtualDevice::keyboard_and_mouse();
    thread::sleep(Duration::from_secs(3));

    keyboard.type_text("fel\n");
    thread::sleep(REFUSAL);
    assert_eq!(
        graphical_session(PERSON),
        None,
        "a wrong password signed in"
    );
    assert!(
        running("bande-greeter"),
        "the login window left after a wrong password"
    );

    for key in [KEY_K, KEY_LEFTBRACE, KEY_APOSTROPHE, KEY_SEMICOLON, KEY_1] {
        keyboard.tap(key);
    }
    Command::new("systemd-cat")
        .args(["--identifier=bande-login-test", "echo", SUBMITTED])
        .status()
        .expect("systemd-cat");
    keyboard.tap(KEY_ENTER);
    wait_for("the Bande session", STARTUP, || compositor_active(PERSON));
    wait_for("the desktop", STARTUP, || {
        journal()
            .iter()
            .any(|entry| entry.message.starts_with(SHOWN))
    });
    let entries = journal();
    let submitted = entries
        .iter()
        .find(|entry| entry.message == SUBMITTED)
        .expect("the submission is in the journal")
        .at;
    let milestone = |identifier: &str, start: &str| after(&entries, submitted, identifier, start);
    let signed_in = milestone("bande-greeter", "signed in");
    let handed_over = milestone("bande-compositor", "leaving the last frame");
    let user_manager = milestone("systemd", "Startup finished");
    let compositor = milestone("systemd", "Starting bande-compositor.service");
    let first_frame = milestone("bande-compositor", "the first frame is on");
    let shown = milestone("bande-compositor", SHOWN);
    for (what, at) in [
        ("greetd accepted the password", signed_in),
        (
            "the login window left its last frame on screen",
            handed_over,
        ),
        ("the user's service manager was up", user_manager),
        ("the session compositor was starting", compositor),
        ("the session put its first frame on screen", first_frame),
        ("the session showed the desktop", shown),
    ] {
        println!(
            "{what}: {}",
            at.map_or("never".to_owned(), |at| format!("{} ms", at.as_millis()))
        );
    }
    assert_eq!(
        milestone("bande-compositor", "the desktop is shown after"),
        None,
        "the session gave up waiting for its shell"
    );
    let shown = shown.expect("the session shows the desktop");
    assert!(
        shown <= TIME_TO_DESKTOP,
        "the desktop took {} ms to appear after the password, more than {} ms",
        shown.as_millis(),
        TIME_TO_DESKTOP.as_millis()
    );
    let session = graphical_session(PERSON).expect("a graphical session");
    assert_eq!(
        session_property(&session, "Type").as_deref(),
        Some("wayland")
    );
    assert_eq!(
        session_property(&session, "Desktop").as_deref(),
        Some("bande")
    );
}
