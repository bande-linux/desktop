#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Desktop, OUTPUT, WINDOW, rgba_at};
use unbound_screenshot::engine::{Frame, Source};

#[test]
fn the_engine_lists_windows_with_their_names_and_frames() {
    let mut desktop = Desktop::start();

    let windows = desktop.wait(|engine| engine.windows());

    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].app_id, "org.example.Red");
    let frame = windows[0].frame.expect("shown on the screen");
    assert_eq!((frame.width, frame.height), WINDOW);
}

#[test]
fn a_screen_shot_holds_the_window_where_it_is() {
    let mut desktop = Desktop::start();
    let windows = desktop.wait(|engine| engine.windows());
    let Frame { x, y, .. } = windows[0].frame.expect("shown");

    let shot = desktop
        .wait(|engine| engine.shoot(Source::Screen, false))
        .expect("a screen shot");

    assert_eq!(
        (shot.width, shot.height),
        (OUTPUT.0 as u32, OUTPUT.1 as u32)
    );
    assert_eq!(
        rgba_at(&shot, (x + WINDOW.0 / 2, y + WINDOW.1 / 2)),
        [0xff, 0, 0, 0xff]
    );
}

#[test]
fn a_window_shot_holds_only_the_window() {
    let mut desktop = Desktop::start();
    let windows = desktop.wait(|engine| engine.windows());
    let identifier = windows[0].identifier.clone();

    let shot = desktop
        .wait(move |engine| engine.shoot(Source::Window(identifier), false))
        .expect("a window shot");

    assert_eq!(
        (shot.width, shot.height),
        (WINDOW.0 as u32, WINDOW.1 as u32)
    );
    assert_eq!(
        rgba_at(&shot, (WINDOW.0 / 2, WINDOW.1 / 2)),
        [0xff, 0, 0, 0xff]
    );
    assert_eq!(rgba_at(&shot, (0, 0))[3], 0);
}

#[test]
fn a_shot_of_a_window_that_is_gone_comes_back_empty() {
    let mut desktop = Desktop::start();

    let shot = desktop.wait(|engine| engine.shoot(Source::Window("gone".to_owned()), false));

    assert_eq!(shot, None);
    assert!(desktop.events.drain().is_empty());
}
