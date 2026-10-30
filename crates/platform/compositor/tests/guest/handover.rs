#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::session::Session;

#[test]
#[ignore = "needs the ci/vm guest with vkms"]
fn leaving_the_screen_keeps_the_last_frame_lit_for_the_next_compositor() {
    let session = Session::open();
    let shown = session.scanout();
    let Session { output, card, .. } = session;
    output.leave_on_screen();
    let after = card
        .current_scanout()
        .expect("the display still shows a frame after the hand-over");
    assert_eq!((after.width, after.height), (shown.width, shown.height));
    assert!(after.rgb == shown.rgb, "the last frame is kept as it was");
    Session::open().present();
}
