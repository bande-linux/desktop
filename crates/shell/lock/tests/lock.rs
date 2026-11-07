#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use bande_datetime::{DateFormats, posix_locale};
use bande_lock::{LockScreen, RESTS_AFTER, SHAKE_DURATION, User};
use bande_toolkit::ui::{Event, Key, Modifiers, Pointer};
use bande_toolkit::{Appearance, Point, TextSystem};

fn enter(screen: &mut LockScreen, text: &str) {
    for key in text.chars().map(Key::Character).chain([Key::Enter]) {
        screen.handle(&Event::Key {
            key,
            modifiers: Modifiers::default(),
        });
    }
}

fn lock_screen() -> LockScreen {
    let user = User {
        uid: 1000,
        login: "ada".to_owned(),
        full_name: "Ada Lovelace".to_owned(),
    };
    LockScreen::new(
        &user,
        DateFormats::for_locale(&posix_locale("en_GB.UTF-8")),
        &"2026-10-28T09:41:00[UTC]".parse().expect("a zoned time"),
        Appearance::default(),
        TextSystem::new(),
    )
}

#[test]
fn a_password_is_submitted_once_and_a_rejected_one_is_cleared_for_another_try() {
    let mut screen = lock_screen();
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

#[test]
fn a_resting_lock_screen_wakes_on_the_first_input_keeps_a_typed_letter_and_rests_again() {
    let mut screen = lock_screen();
    let start = Instant::now();
    screen.start_resting();
    assert!(screen.is_resting());

    let click = Event::PointerDown(Pointer::at(Point::new(10.0, 10.0)));
    assert!(!screen.wake_on(&click, start));
    assert!(!screen.is_resting());

    let later = start + RESTS_AFTER;
    assert!(screen.rest_when_due(later));
    assert!(screen.is_resting());
    enter(&mut screen, "");
    assert_eq!(screen.take_password(), None);

    let letter = Event::Key {
        key: Key::Character('r'),
        modifiers: Modifiers::default(),
    };
    assert!(screen.wake_on(&letter, later));
    screen.handle(&letter);
    enter(&mut screen, "ight");
    assert_eq!(
        screen.take_password().as_deref().map(String::as_str),
        Some("right")
    );

    screen.reject(later);
    screen.handle(&letter);
    assert!(screen.rest_when_due(later + RESTS_AFTER));
    assert!(screen.is_resting());
    enter(&mut screen, "");
    assert_eq!(screen.take_password(), None);
}
