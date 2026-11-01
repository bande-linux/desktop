#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use bande_bar::dialogs::session_ending_scene;
use bande_status::session::{Ending, QUIT_PATIENCE, SessionEnding};
use bande_toolkit::{Appearance, CpuRenderer, Scheme};

fn check(session: &SessionEnding, scheme: Scheme, now: Instant, name: &str) {
    let appearance = Appearance {
        scheme,
        ..Appearance::default()
    };
    let scene = session_ending_scene(session, appearance, now);
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    bande_golden::assert_golden_rgba!(name, (scene.width, scene.height), frame.rgba);
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
