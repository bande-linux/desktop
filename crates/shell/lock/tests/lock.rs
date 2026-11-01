#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use bande_datetime::{DateFormats, posix_locale};
use bande_lock::{LockScreen, SHAKE_DURATION, User};
use bande_toolkit::ui::{Event, Key, Modifiers};
use bande_toolkit::{Appearance, TextSystem};

fn enter(screen: &mut LockScreen, text: &str) {
    for key in text.chars().map(Key::Character).chain([Key::Enter]) {
        screen.handle(&Event::Key {
            key,
            modifiers: Modifiers::default(),
        });
    }
}

#[test]
fn a_password_is_submitted_once_and_a_rejected_one_is_cleared_for_another_try() {
    let user = User {
        uid: 1000,
        login: "ada".to_owned(),
        full_name: "Ada Lovelace".to_owned(),
    };
    let mut screen = LockScreen::new(
        &user,
        DateFormats::for_locale(&posix_locale("en_GB.UTF-8")),
        &"2026-10-28T09:41:00[UTC]".parse().expect("a zoned time"),
        Appearance::default(),
        TextSystem::new(),
    );
    enter(&mut screen, "");
    assert_eq!(screen.take_password(), None);

    enter(&mut screen, "wrong");
    assert_eq!(
        screen.take_password().as_deref().map(String::as_str),
        Some("wrong")
    );
    enter(&mut screen, "more");
    assert_eq!(screen.take_password(), None);
    let rejected = Instant::now();
    screen.reject(rejected);
    screen.advance(rejected + SHAKE_DURATION);
    assert!(!screen.is_animating());

    enter(&mut screen, "right");
    assert_eq!(
        screen.take_password().as_deref().map(String::as_str),
        Some("right")
    );
}
