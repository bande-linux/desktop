#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::thread;
use std::time::Instant;

use bande_datetime::{DateFormats, posix_locale};
use bande_lock::{LockScreen, RESTS_AFTER, SHAKE_DURATION, User};
use bande_test_wayland::keys::{
    KEY_A, KEY_B, KEY_BACKSPACE, KEY_C, KEY_D, KEY_ENTER, KEY_LEFTCTRL, KEY_X,
};
use bande_test_wayland::{Frames, TestClient, chord, headless};
use bande_toolkit::ui::{Event, Key, Modifiers, Pointer};
use bande_toolkit::{App, Appearance, Point, Role, Scene, TextSystem, Window, WindowEvent};

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

struct Locked {
    screen: LockScreen,
    focused: Arc<AtomicBool>,
    passwords: Sender<String>,
    submitted: usize,
}

impl App for Locked {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.screen.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Focus(focused) => self.focused.store(focused, Ordering::SeqCst),
            WindowEvent::Input(input) => {
                self.screen.handle(&input);
            }
            _ => return,
        }
        if let Some(password) = self.screen.take_password() {
            self.submitted += 1;
            let _ = self.passwords.send(password.to_string());
            self.screen.reject(Instant::now());
        }
        window.request_redraw();
    }

    fn finished(&self) -> bool {
        self.submitted == 2
    }

    fn animating(&self) -> bool {
        self.screen.is_animating()
    }
}

#[test]
fn the_key_that_wakes_a_dark_display_is_not_typed_and_select_all_then_backspace_clears_the_field() {
    let mut server = headless(480, 360);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let focused = Arc::new(AtomicBool::new(false));
    let (passwords, typed) = channel();
    let app = {
        let focused = focused.clone();
        thread::spawn(move || {
            let mut window = Window::open_on(
                client_end,
                Role::toplevel("Locked", "bande-lock-test"),
                (480, 360),
            )
            .expect("window");
            let mut locked = Locked {
                screen: lock_screen(),
                focused,
                passwords,
                submitted: 0,
            };
            bande_toolkit::run(&mut window, &mut locked).expect("run the window");
        })
    };
    let mut frames = Frames::start();
    frames.run_until(&mut server, || focused.load(Ordering::SeqCst));
    let mut power = TestClient::connect(&mut server);

    chord(&mut server, &[KEY_A]);
    chord(&mut server, &[KEY_B]);
    power.turn_the_display_off(&mut server);
    chord(&mut server, &[KEY_X]);
    chord(&mut server, &[KEY_D]);
    let mut polls = 0;
    frames.run_until(&mut server, || {
        polls += 1;
        polls > 200
    });
    chord(&mut server, &[KEY_C]);
    chord(&mut server, &[KEY_ENTER]);
    let mut first = None;
    frames.run_until(&mut server, || {
        first = first.take().or_else(|| typed.try_recv().ok());
        first.is_some()
    });
    assert_eq!(
        first.as_deref(),
        Some("abc"),
        "the wake keys were not typed"
    );

    chord(&mut server, &[KEY_A]);
    chord(&mut server, &[KEY_B]);
    chord(&mut server, &[KEY_LEFTCTRL, KEY_A]);
    chord(&mut server, &[KEY_BACKSPACE]);
    chord(&mut server, &[KEY_D]);
    chord(&mut server, &[KEY_ENTER]);
    frames.run_until(&mut server, || app.is_finished());
    app.join().expect("app thread");
    assert_eq!(typed.try_recv().as_deref(), Ok("d"));
}
