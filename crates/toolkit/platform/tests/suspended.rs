#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_toolkit_platform::WindowEvent;

#[allow(dead_code)]
#[path = "support/desktop.rs"]
mod desktop;
#[allow(dead_code)]
#[path = "support/note.rs"]
mod note;

use desktop::Desktop;
use note::note;

const OUTPUT: (i32, i32) = (640, 400);
const KEY_LEFTMETA: u32 = 125;
const KEY_M: u32 = 50;

fn suspended(events: &[WindowEvent]) -> Option<bool> {
    events.iter().rev().find_map(|event| match event {
        WindowEvent::ToplevelChanged(state) => Some(state.suspended),
        _ => None,
    })
}

#[test]
fn a_minimised_window_is_told_it_is_suspended() {
    let mut desktop = Desktop::sized(OUTPUT);
    let mut client = desktop.open_named("hidden", note("", 0..0));
    let before = suspended(&client.seen);

    desktop.server.key(KEY_LEFTMETA, true, 1);
    desktop.server.key(KEY_M, true, 2);
    desktop.server.key(KEY_M, false, 3);
    desktop.server.key(KEY_LEFTMETA, false, 4);
    desktop.wait_for(
        &mut [&mut client],
        |events| suspended(events) == Some(true),
        |_| true,
    );

    assert_ne!(before, Some(true));
}
