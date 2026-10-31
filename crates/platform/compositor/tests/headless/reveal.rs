use crate::support::*;
use std::time::Duration;

use unbound_compositor::{BACKGROUND, Headless, Renderer};
use unbound_test_wayland::{
    Anchor, BLUE, KeyboardInteractivity, Layer, LayerRequest, RED, TestClient,
};

const STARTED: Duration = Duration::from_secs(1);
const DEADLINE: Duration = Duration::from_secs(4);

fn holding_server() -> Headless {
    let mut server = server();
    server.hold_until_shell_ready(STARTED, DEADLINE);
    server
}

fn shell(server: &mut Headless) -> TestClient {
    let mut shell = TestClient::connect_as_shell(server);
    shell.join_shell(server);
    shell
}

fn show_wallpaper(shell: &mut TestClient, server: &mut Headless) -> (u32, u32) {
    shell.show_named_layer(
        server,
        &LayerRequest {
            layer: Layer::Background,
            anchor: Anchor::all(),
            size: (0, 0),
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::None,
            color: BLUE,
        },
        "desktop",
    )
}

fn show_bar(shell: &mut TestClient, server: &mut Headless) {
    shell.show_named_layer(
        server,
        &LayerRequest {
            layer: Layer::Top,
            anchor: Anchor::Top | Anchor::Left | Anchor::Right,
            size: (0, BAR as u32),
            exclusive_zone: BAR,
            keyboard: KeyboardInteractivity::OnDemand,
            color: RED,
        },
        "bar",
    );
}

fn bar_and_desktop(renderer: &mut Renderer, server: &mut Headless) -> ([u8; 3], [u8; 3]) {
    (
        pixel(renderer, server, 128, 8),
        pixel(renderer, server, 128, 120),
    )
}

fn close(left: [u8; 3], right: [u8; 3]) -> bool {
    left.iter()
        .zip(right)
        .all(|(left, right)| left.abs_diff(right) <= 2)
}

#[test]
fn the_first_frame_waits_for_an_opaque_wallpaper_and_the_bar_then_the_bar_fades_in() {
    let mut server = holding_server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut shell = shell(&mut server);
    let size = show_wallpaper(&mut shell, &mut server);
    show_bar(&mut shell, &mut server);
    assert_eq!(server.reveal_wake(STARTED), Some(DEADLINE));
    assert!(
        server.holds_frames(),
        "a wallpaper that may still be see-through is not shown"
    );

    shell.declare_layer_opaque(&mut server, 0, size);
    assert_eq!(server.reveal_wake(STARTED), None);
    assert!(!server.holds_frames());

    server.animate(STARTED);
    let (bar, desktop) = bar_and_desktop(&mut renderer, &mut server);
    assert!(close(desktop, [0, 0, 255]), "the wallpaper is fully shown");
    assert!(
        close(bar, [0, 0, 255]),
        "the bar starts invisible over the wallpaper, not {bar:?}"
    );

    server.animate(STARTED + Duration::from_millis(120));
    let (bar, _) = bar_and_desktop(&mut renderer, &mut server);
    assert!(bar[0] > 20 && bar[2] > 20, "the bar is fading in, {bar:?}");

    settle(&mut server, STARTED.as_millis() as u64);
    let (bar, desktop) = bar_and_desktop(&mut renderer, &mut server);
    assert!(close(bar, [255, 0, 0]), "the bar is fully shown, {bar:?}");
    assert!(close(desktop, [0, 0, 255]));
}

#[test]
fn the_desktop_is_shown_when_the_shell_misses_its_deadline() {
    let mut server = holding_server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut shell = shell(&mut server);
    show_bar(&mut shell, &mut server);
    assert_eq!(
        server.reveal_wake(DEADLINE - Duration::from_millis(1)),
        Some(DEADLINE)
    );
    assert_eq!(server.reveal_wake(DEADLINE), None);
    assert!(!server.holds_frames());
    settle(&mut server, DEADLINE.as_millis() as u64);
    let (bar, desktop) = bar_and_desktop(&mut renderer, &mut server);
    assert!(close(bar, [255, 0, 0]));
    assert!(close(desktop, BACKGROUND));
}
