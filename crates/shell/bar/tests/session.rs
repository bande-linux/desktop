#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use unbound_bar::dialogs::{CONFIRMATION_SIZE, session_ending_scene};
use unbound_status::session::{Ending, QUIT_PATIENCE, SessionEnding};
use unbound_toolkit::{CpuRenderer, Palette, Scheme};

fn check(session: &SessionEnding, scheme: Scheme, now: Instant, name: &str) {
    let scene = session_ending_scene(session, Palette::for_scheme(scheme), CONFIRMATION_SIZE, now);
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    let (width, height) = CONFIRMATION_SIZE;
    unbound_golden::assert_golden!(name, &unbound_golden::from_rgba(width, height, frame.rgba));
}

#[test]
fn the_shut_down_dialog_counts_down_then_names_the_app_that_keeps_it_waiting() {
    let start = Instant::now();
    let mut shut_down = SessionEnding::new(Ending::ShutDown, start);
    check(
        &shut_down,
        Scheme::Light,
        start + Duration::from_millis(17_500),
        "shut-down-light",
    );

    shut_down.confirm(start);
    shut_down.apps_running("Terminal".to_owned(), 1, start);
    shut_down.tick(start + QUIT_PATIENCE);
    check(&shut_down, Scheme::Dark, start, "shut-down-blocked-dark");
}
