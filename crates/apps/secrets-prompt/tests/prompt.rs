#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

#[test]
fn a_request_written_after_the_prompt_started_is_still_read_and_a_failed_window_is_no_answer() {
    let mut prompt = Command::new(env!("CARGO_BIN_EXE_bande-secrets-prompt"))
        .arg("--unlock")
        .env_remove("WAYLAND_DISPLAY")
        .env("XDG_RUNTIME_DIR", "/nonexistent")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    sleep(Duration::from_millis(300));
    prompt
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"keychain\tLogin\n\n")
        .unwrap();

    let finished = prompt.wait_with_output().unwrap();

    assert!(
        !String::from_utf8_lossy(&finished.stderr).contains("usage:"),
        "the request was not read"
    );
    assert!(
        finished.stdout.is_empty(),
        "a prompt that never appeared must not answer as a cancel"
    );
}
