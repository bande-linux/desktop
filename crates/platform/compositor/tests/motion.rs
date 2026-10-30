#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use smithay::utils::Size;
use unbound_compositor::{Headless, Renderer, Snapshot};
use unbound_test_wayland::{
    Anchor, GREEN, KeyboardInteractivity, Layer, LayerRequest, RED, ShmImage, TestClient, roundtrip,
};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const WINDOW: (i32, i32) = (64, 48);
const KEY_M: u32 = 50;
const KEY_LEFTMETA: u32 = 125;
const SETTLED: Duration = Duration::from_secs(2);

type Bounds = (i32, i32, i32, i32);

fn server() -> Headless {
    Headless::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn app(server: &mut Headless) -> TestClient {
    let mut client = TestClient::connect(server);
    client.use_seat(server);
    let pixels = RED.repeat((WINDOW.0 * WINDOW.1) as usize);
    client.show_window(
        server,
        &ShmImage {
            width: WINDOW.0,
            height: WINDOW.1,
            stride: WINDOW.0 * 4,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
    );
    client
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn frame(renderer: &mut Renderer, server: &mut Headless, time: Duration) -> Snapshot {
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

fn minimize(server: &mut Headless, client: &mut TestClient) {
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
    let mut editor = app(&mut server);
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

fn settled_window() -> (Headless, Renderer, TestClient) {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = app(&mut server);
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
    editor.set_app_id(&mut server, 0, "org.unbound.Editor");
    let start = SETTLED + ms(100);
    minimize(&mut server, &mut editor);
    frame(&mut renderer, &mut server, start);
    let fleeing = reddish(&frame(&mut renderer, &mut server, start + ms(120))).expect("shown");

    shell.activate_app(&mut server, "org.unbound.Editor");
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
    editor.set_app_id(&mut server, 0, "org.unbound.Editor");
    dock.set_icon_rect(&mut server, "org.unbound.Editor", 0, (8, 140, 32, 32));
    let start = SETTLED + ms(100);

    minimize(&mut server, &mut editor);
    frame(&mut renderer, &mut server, start);
    let arriving = reddish(&frame(&mut renderer, &mut server, start + ms(350))).expect("shown");

    assert!(arriving.0 >= 6 && arriving.2 <= 42, "{arriving:?}");
    assert!(arriving.1 >= 138 && arriving.3 <= 174, "{arriving:?}");
}
