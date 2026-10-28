use smithay::utils::{Point, Size};
use unbound_compositor::{Headless, Renderer};
use unbound_test_wayland::{
    Anchor, BLUE, GREEN, Layer, LayerRequest, RED, ShmImage, TestClient, roundtrip,
};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const BTN_LEFT: u32 = 0x110;

fn server() -> Headless {
    Headless::new(Size::from(OUTPUT)).expect("headless compositor")
}

fn red(width: i32, height: i32) -> Vec<u8> {
    RED.repeat((width * height) as usize)
}

fn image(pixels: &[u8], width: i32, height: i32) -> ShmImage<'_> {
    ShmImage {
        width,
        height,
        stride: width * 4,
        format: Format::Xrgb8888,
        bytes: pixels,
    }
}

fn panel(layer: Layer, anchor: Anchor, size: (u32, u32), exclusive_zone: i32) -> LayerRequest {
    LayerRequest {
        layer,
        anchor,
        size,
        exclusive_zone,
        color: GREEN,
    }
}

fn pixel(renderer: &mut Renderer, server: &mut Headless, x: i32, y: i32) -> [u8; 3] {
    renderer.render(server).expect("frame");
    let snapshot = renderer.snapshot(server).expect("read back");
    let at = ((y * snapshot.size.w + x) * 4) as usize;
    [
        snapshot.rgba[at],
        snapshot.rgba[at + 1],
        snapshot.rgba[at + 2],
    ]
}

fn rgb(bgra: [u8; 4]) -> [u8; 3] {
    [bgra[2], bgra[1], bgra[0]]
}

#[test]
fn a_left_panel_reserves_its_exclusive_zone_and_windows_are_centered_beside_it() {
    let mut server = server();
    let mut shell = TestClient::connect(&mut server);
    let size = shell.show_layer(
        &mut server,
        &panel(
            Layer::Top,
            Anchor::Left | Anchor::Top | Anchor::Bottom,
            (40, 0),
            40,
        ),
    );
    assert_eq!(size, (40, OUTPUT.1 as u32));

    let mut app = TestClient::connect(&mut server);
    let pixels = red(96, 64);
    app.show_window(&mut server, &image(&pixels, 96, 64));

    assert_eq!(
        server.windows()[0].location(),
        Point::from((40 + (216 - 96) / 2, (192 - 64) / 2))
    );
}

#[test]
fn a_top_bar_pushes_windows_below_it() {
    let mut server = server();
    let mut shell = TestClient::connect(&mut server);
    let size = shell.show_layer(
        &mut server,
        &panel(
            Layer::Top,
            Anchor::Top | Anchor::Left | Anchor::Right,
            (0, 24),
            24,
        ),
    );
    assert_eq!(size, (OUTPUT.0 as u32, 24));

    let mut app = TestClient::connect(&mut server);
    let pixels = red(96, 64);
    app.show_window(&mut server, &image(&pixels, 96, 64));

    assert_eq!(
        server.windows()[0].location(),
        Point::from(((256 - 96) / 2, 24 + (168 - 64) / 2))
    );
}

#[test]
fn background_layers_are_drawn_below_windows_and_top_layers_above() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut app = TestClient::connect(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    let mut shell = TestClient::connect(&mut server);
    let everywhere = Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right;
    shell.show_layer(
        &mut server,
        &panel(Layer::Background, everywhere, (0, 0), 0),
    );
    let center = (OUTPUT.0 / 2, OUTPUT.1 / 2);

    assert_eq!(
        pixel(&mut renderer, &mut server, center.0, center.1),
        rgb(RED)
    );
    assert_eq!(pixel(&mut renderer, &mut server, 2, 2), rgb(GREEN));

    shell.show_layer(
        &mut server,
        &LayerRequest {
            color: BLUE,
            ..panel(Layer::Overlay, everywhere, (0, 0), 0)
        },
    );

    assert_eq!(
        pixel(&mut renderer, &mut server, center.0, center.1),
        rgb(BLUE)
    );
}

#[test]
fn clicking_a_panel_keeps_the_focused_window_focused() {
    let mut server = server();
    let mut shell = TestClient::connect(&mut server);
    shell.show_layer(
        &mut server,
        &panel(
            Layer::Top,
            Anchor::Left | Anchor::Top | Anchor::Bottom,
            (40, 0),
            40,
        ),
    );
    let mut app = TestClient::connect(&mut server);
    app.use_seat(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));
    assert!(app.keyboard_focus_on(0));

    server.move_pointer_to(Point::from((10.0, 10.0)), 1);
    server.press_button(BTN_LEFT, true, 2);
    server.press_button(BTN_LEFT, false, 3);
    roundtrip(&mut server, &mut [&mut shell, &mut app]);

    assert!(app.keyboard_focus_on(0));
}

#[test]
fn the_focused_app_can_hand_activation_to_another_app() {
    let mut server = server();
    let mut other = TestClient::connect(&mut server);
    other.use_seat(&mut server);
    let small = red(32, 32);
    other.show_window(&mut server, &image(&small, 32, 32));
    let mut focused = TestClient::connect(&mut server);
    focused.use_seat(&mut server);
    let large = red(96, 96);
    focused.show_window(&mut server, &image(&large, 96, 96));
    roundtrip(&mut server, &mut [&mut other, &mut focused]);
    assert!(focused.keyboard_focus_on(0));

    let own_token = other.activation_token(&mut server);
    other.activate(&mut server, 0, &own_token);
    roundtrip(&mut server, &mut [&mut other, &mut focused]);
    assert!(focused.keyboard_focus_on(0));

    let handed = focused.activation_token(&mut server);
    other.activate(&mut server, 0, &handed);
    roundtrip(&mut server, &mut [&mut other, &mut focused]);
    assert!(other.keyboard_focus_on(0));
    assert!(!focused.keyboard_focus_on(0));
}

#[test]
fn a_new_window_activated_with_a_handed_token_takes_focus() {
    let mut server = server();
    let mut other = TestClient::connect(&mut server);
    other.use_seat(&mut server);
    let small = red(32, 32);
    other.show_window(&mut server, &image(&small, 32, 32));
    let mut focused = TestClient::connect(&mut server);
    focused.use_seat(&mut server);
    let large = red(96, 96);
    focused.show_window(&mut server, &image(&large, 96, 96));
    roundtrip(&mut server, &mut [&mut other, &mut focused]);

    let handed = focused.activation_token(&mut server);
    other.show_window_activated(&mut server, &image(&small, 32, 32), &handed);
    roundtrip(&mut server, &mut [&mut other, &mut focused]);

    assert!(other.keyboard_focus_on(1));
}

#[test]
fn a_viewport_destination_sets_the_window_size() {
    let mut server = server();
    let mut app = TestClient::connect(&mut server);
    let pixels = red(32, 32);
    app.show_window_scaled(&mut server, &image(&pixels, 32, 32), (64, 48));

    let [window] = server.windows() else {
        panic!("one window");
    };
    assert_eq!(window.size(), Some(Size::from((64, 48))));
    assert_eq!(
        window.location(),
        Point::from(((256 - 64) / 2, (192 - 48) / 2))
    );
}

#[test]
fn surfaces_are_told_to_render_at_scale_one() {
    let mut server = server();
    let mut app = TestClient::connect(&mut server);
    let pixels = red(32, 32);
    app.show_window(&mut server, &image(&pixels, 32, 32));

    assert_eq!(app.preferred_scale(&mut server, 0), Some(120));
}
