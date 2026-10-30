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

fn compositor_active(person: &str) -> bool {
    output(
        "systemctl",
        &[
            "--user",
            &format!("--machine={person}@"),
            "is-active",
            "unbound-compositor.service",
        ],
    )
    .is_some_and(|state| state == "active")
}

#[test]
#[ignore = "needs the ci/vm guest booted with systemd, greetd and the test person"]
fn greetd_signs_a_person_in_to_the_unbound_session_with_the_system_keyboard_layout() {
    wait_for("the login window", STARTUP, || running("unbound-greeter"));
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
        running("unbound-greeter"),
        "the login window left after a wrong password"
    );

    for key in [
        KEY_K,
        KEY_LEFTBRACE,
        KEY_APOSTROPHE,
        KEY_SEMICOLON,
        KEY_1,
        KEY_ENTER,
    ] {
        keyboard.tap(key);
    }
    wait_for("the Unbound session", STARTUP, || compositor_active(PERSON));
    let session = graphical_session(PERSON).expect("a graphical session");
    assert_eq!(
        session_property(&session, "Type").as_deref(),
        Some("wayland")
    );
    assert_eq!(
        session_property(&session, "Desktop").as_deref(),
        Some("unbound")
    );
}
