use std::time::Duration;

use crate::support::*;
use bande_compositor::{Compositor, Renderer, Snapshot};
use bande_test_wayland::{
    Anchor, GREEN, KeyboardInteractivity, Layer, LayerRequest, RED, TestClient, roundtrip,
};

const WINDOW: (i32, i32) = (64, 48);
const SETTLED: Duration = Duration::from_secs(2);

type Bounds = (i32, i32, i32, i32);

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn frame(renderer: &mut Renderer, server: &mut Compositor, time: Duration) -> Snapshot {
    server.animate(time);
    renderer.render(server).expect("frame");
    renderer.snapshot(server).expect("read back")
}

fn red_at(snapshot: &Snapshot, x: i32, y: i32) -> u8 {
    snapshot.rgba[((y * snapshot.size.w + x) * 4) as usize]
}

fn reddish(snapshot: &Snapshot) -> Option<Bounds> {
    let mut bounds: Option<Bounds> = None;
    for (index, rgba) in snapshot.rgba.as_chunks::<4>().0.iter().enumerate() {
        if rgba[0] < rgba[1].saturating_add(0x40) {
            continue;
        }
        let (x, y) = (
            index as i32 % snapshot.size.w,
            index as i32 / snapshot.size.w,
        );
        bounds = Some(match bounds {
            None => (x, y, x, y),
            Some((left, top, right, bottom)) => {
                (left.min(x), top.min(y), right.max(x), bottom.max(y))
            }
        });
    }
    bounds
}

fn minimize(server: &mut Compositor, client: &mut TestClient) {
    for (key, pressed) in [
        (KEY_LEFTMETA, true),
        (KEY_M, true),
        (KEY_M, false),
        (KEY_LEFTMETA, false),
    ] {
        server.key(key, pressed, 0);
    }
    roundtrip(server, &mut [client]);
}

const FULL: Bounds = (96, 72, 96 + WINDOW.0 - 1, 72 + WINDOW.1 - 1);

#[test]
fn a_new_window_takes_focus_at_once_and_grows_and_fades_in() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window(&mut server, RED, WINDOW);
    roundtrip(&mut server, &mut [&mut editor]);
    assert!(editor.keyboard_focus_on(0));

    let first = frame(&mut renderer, &mut server, ms(0));
    let center = red_at(&first, 128, 96);
    assert!(center > 0x20 && center < 0xff, "{center}");
    assert!(red_at(&first, 128, 72) < center);

    let settled = frame(&mut renderer, &mut server, SETTLED);
    assert_eq!(red_at(&settled, 128, 96), 0xff);
    assert_eq!(red_at(&settled, 128, 72), 0xff);
    assert!(!server.is_animating());
}

fn settled_window() -> (Compositor, Renderer, TestClient) {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window(&mut server, RED, WINDOW);
    frame(&mut renderer, &mut server, ms(0));
    frame(&mut renderer, &mut server, SETTLED);
    roundtrip(&mut server, &mut [&mut editor]);
    (server, renderer, editor)
}

#[test]
fn minimizing_moves_focus_at_once_and_shrinks_the_window_toward_the_dock_edge() {
    let (mut server, mut renderer, mut editor) = settled_window();
    let start = SETTLED + ms(100);

    minimize(&mut server, &mut editor);
    assert!(server.windows()[0].is_minimized());
    assert!(!editor.keyboard_focus_on(0));

    let early = reddish(&frame(&mut renderer, &mut server, start)).expect("still shown");
    let later = reddish(&frame(&mut renderer, &mut server, start + ms(150))).expect("shown");
    assert!(later.1 > early.1 && later.2 - later.0 < early.2 - early.0);
    assert_eq!(
        reddish(&frame(&mut renderer, &mut server, start + SETTLED)),
        None
    );
    assert!(!server.is_animating());
}

#[test]
fn restoring_mid_flight_turns_back_from_where_the_window_is() {
    let (mut server, mut renderer, mut editor) = settled_window();
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);
    editor.set_app_id(&mut server, 0, "org.bande.Editor");
    let start = SETTLED + ms(100);
    minimize(&mut server, &mut editor);
    frame(&mut renderer, &mut server, start);
    let fleeing = reddish(&frame(&mut renderer, &mut server, start + ms(120))).expect("shown");

    shell.activate_app(&mut server, "org.bande.Editor");
    let turning = reddish(&frame(&mut renderer, &mut server, start + ms(136))).expect("shown");

    assert!(
        (turning.1 - fleeing.1).abs() <= 4,
        "{fleeing:?} {turning:?}"
    );
    assert!(
        (turning.0 - fleeing.0).abs() <= 4,
        "{fleeing:?} {turning:?}"
    );
    assert_eq!(
        reddish(&frame(&mut renderer, &mut server, start + SETTLED)),
        Some(FULL)
    );
}

#[test]
fn a_closed_window_fades_out_from_its_last_frame() {
    let (mut server, mut renderer, mut editor) = settled_window();

    editor.close_window(&mut server, 0);
    let closing = frame(&mut renderer, &mut server, SETTLED + ms(16));

    let shown = reddish(&closing).expect("still shown");
    assert!(shown.0 >= FULL.0 && shown.2 <= FULL.2);
    assert!(red_at(&closing, 128, 96) < 0xff);
    assert_eq!(
        reddish(&frame(&mut renderer, &mut server, SETTLED * 2)),
        None
    );
    assert!(!server.is_animating());
}

#[test]
fn a_window_closed_after_another_renderer_drew_it_still_renders() {
    let (mut server, mut renderer, mut editor) = settled_window();
    let mut other = Renderer::new().expect("second renderer");
    frame(&mut other, &mut server, SETTLED + ms(8));

    editor.close_window(&mut server, 0);
    frame(&mut renderer, &mut server, SETTLED + ms(16));
    frame(&mut other, &mut server, SETTLED + ms(24));
    assert_eq!(
        reddish(&frame(&mut renderer, &mut server, SETTLED * 2)),
        None
    );
}

#[test]
fn with_reduce_motion_minimizing_fades_in_place() {
    let (mut server, mut renderer, mut editor) = settled_window();
    server.set_reduce_motion(true);

    minimize(&mut server, &mut editor);
    let fading = frame(&mut renderer, &mut server, SETTLED + ms(50));

    assert_eq!(reddish(&fading), Some(FULL));
    assert!(red_at(&fading, 128, 96) < 0xff);
    assert_eq!(
        reddish(&frame(&mut renderer, &mut server, SETTLED * 2)),
        None
    );
}

#[test]
fn zooming_stretches_the_current_frame_toward_the_new_size_before_the_app_redraws() {
    let (mut server, mut renderer, mut editor) = settled_window();

    editor.set_maximized(&mut server, 0, true);
    let growing = reddish(&frame(&mut renderer, &mut server, SETTLED + ms(50))).expect("shown");

    assert!(growing.0 < FULL.0 && growing.2 > FULL.2);
    assert!(growing.0 > 0 && growing.2 < OUTPUT.0 - 1);

    editor.fill_configured_size(&mut server, 0, RED);
    assert_eq!(
        reddish(&frame(&mut renderer, &mut server, SETTLED * 2)),
        Some((0, 0, OUTPUT.0 - 1, OUTPUT.1 - 1))
    );
}

#[test]
fn a_window_minimizes_into_its_apps_dock_icon() {
    let (mut server, mut renderer, mut editor) = settled_window();
    let mut dock = TestClient::connect_as_shell(&mut server);
    dock.join_shell(&mut server);
    dock.show_layer(
        &mut server,
        &LayerRequest {
            layer: Layer::Top,
            anchor: Anchor::Left | Anchor::Top | Anchor::Bottom,
            size: (48, 0),
            exclusive_zone: 0,
            keyboard: KeyboardInteractivity::None,
            color: GREEN,
        },
    );
    editor.set_app_id(&mut server, 0, "org.bande.Editor");
    dock.set_icon_rect(&mut server, "org.bande.Editor", 0, (8, 140, 32, 32));
    let start = SETTLED + ms(100);

    minimize(&mut server, &mut editor);
    frame(&mut renderer, &mut server, start);
    let arriving = reddish(&frame(&mut renderer, &mut server, start + ms(350))).expect("shown");

    assert!(arriving.0 >= 6 && arriving.2 <= 42, "{arriving:?}");
    assert!(arriving.1 >= 138 && arriving.3 <= 174, "{arriving:?}");
}

#[test]
fn a_display_sampled_at_an_earlier_vblank_than_another_shows_that_earlier_moment() {
    let mut alone = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut client = window(&mut alone, RED, WINDOW);
    roundtrip(&mut alone, &mut [&mut client]);
    alone.animate(ms(8));
    let expected = frame(&mut renderer, &mut alone, ms(16));

    let mut shared = server();
    let mut other = window(&mut shared, RED, WINDOW);
    roundtrip(&mut shared, &mut [&mut other]);
    shared.animate(ms(8));
    shared.animate(ms(24));
    let sampled = frame(&mut renderer, &mut shared, ms(16));

    assert_eq!(sampled.rgba, expected.rgba);
}

#[test]
fn a_window_dropped_on_a_tile_is_nearly_in_place_after_120_ms() {
    let (mut server, mut renderer, mut editor) = settled_window();
    server.move_pointer_to(at(110, 80), 0);
    server.press_button(BTN_LEFT, true, 0);
    roundtrip(&mut server, &mut [&mut editor]);
    editor.request_move(&mut server, 0);
    server.move_pointer_to(at(0, 100), 1000);
    let start = SETTLED + ms(100);
    frame(&mut renderer, &mut server, start);
    server.press_button(BTN_LEFT, false, 1000);
    roundtrip(&mut server, &mut [&mut editor]);

    frame(&mut renderer, &mut server, start + ms(16));
    let (left, top, right, bottom) =
        reddish(&frame(&mut renderer, &mut server, start + ms(136))).expect("shown");

    assert!(left <= 8 && top <= 8, "{left} {top}");
    assert!(right >= 127 - 8 && bottom >= 191 - 8, "{right} {bottom}");
}
