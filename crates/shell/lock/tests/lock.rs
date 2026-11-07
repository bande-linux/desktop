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

fn password(screen: &mut LockScreen) -> Option<String> {
    screen.take_password().map(|typed| typed.to_string())
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
fn a_password_is_submitted_once_and_a_rejected_one_is_cleared_and_a_resting_screen_keeps_a_letter()
{
    let mut screen = lock_screen();
    enter(&mut screen, "");
    assert_eq!(screen.take_password(), None);
    enter(&mut screen, "wrong");
    assert_eq!(password(&mut screen), Some("wrong".to_owned()));
    enter(&mut screen, "more");
    assert_eq!(screen.take_password(), None);
    let start = Instant::now();
    screen.reject(start);
    screen.advance(start + SHAKE_DURATION);
    assert!(!screen.is_animating());

    screen.start_resting();
    let click = Event::PointerDown(Pointer::at(Point::new(10.0, 10.0)));
    assert!(!screen.wake_on(&click, start));
    assert!(!screen.is_resting());
    let later = start + RESTS_AFTER;
    assert!(screen.rest_when_due(later));
    enter(&mut screen, "");
    assert_eq!(screen.take_password(), None);
    let letter = Event::Key {
        key: Key::Character('r'),
        modifiers: Modifiers::default(),
    };
    assert!(screen.wake_on(&letter, later));
    screen.handle(&letter);
    enter(&mut screen, "ight");
    assert_eq!(password(&mut screen), Some("right".to_owned()));

    screen.reject(later);
    screen.handle(&letter);
    assert!(screen.rest_when_due(later + RESTS_AFTER));
    enter(&mut screen, "");
    assert_eq!(screen.take_password(), None, "resting cleared the field");
}
