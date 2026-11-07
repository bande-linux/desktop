#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

use bande_xdg::{set_app_language, spawn_detached};

fn language_seen_by(app_id: &str, out: &Path) -> Vec<String> {
    let script = format!(
        "printf '%s\\n%s\\n' \"$LANGUAGE\" \"$BANDE_SESSION_LANGUAGE\" > {0}.new && mv {0}.new {0}",
        out.display()
    );
    spawn_detached(app_id, &["sh", "-c", &script], None, None).unwrap();
    let started = Instant::now();
    while !out.exists() && started.elapsed() < Duration::from_secs(10) {
        sleep(Duration::from_millis(10));
    }
    let text = std::fs::read_to_string(out).unwrap();
    std::fs::remove_file(out).unwrap();
    text.lines().map(str::to_owned).collect()
}

#[test]
fn an_app_starts_in_its_language_and_does_not_pass_it_on() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("seen");
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", dir.path());
        std::env::set_var("LANGUAGE", "session");
        std::env::remove_var("BANDE_SESSION_LANGUAGE");
    }
    set_app_language("org.example.Chosen", Some("xx_YY")).unwrap();

    assert_eq!(
        language_seen_by("org.example.Chosen", &out),
        ["xx_YY", "session"]
    );
    assert_eq!(language_seen_by("org.example.Other", &out), ["session", ""]);

    unsafe {
        std::env::set_var("LANGUAGE", "xx_YY");
        std::env::set_var("BANDE_SESSION_LANGUAGE", "session");
    }
    assert_eq!(language_seen_by("org.example.Other", &out), ["session", ""]);
}
