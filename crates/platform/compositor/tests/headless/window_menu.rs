use std::time::{Duration, Instant};

use crate::support::*;
use smithay::utils::{Logical, Point, Size};
use unbound_compositor::{Compositor, Renderer};
use unbound_graphics::TextSize;
use unbound_test_wayland::{BLUE, DecorationMode, TestClient, ToplevelState, roundtrip};

const SCREEN: (i32, i32) = (640, 480);
const WINDOW: (i32, i32) = (200, 120);
const TITLE_BAR: i32 = 40;
const ZOOM: (i32, i32) = (66, TITLE_BAR / 2);
const BTN_RIGHT: u32 = 0x111;
const KEY_ENTER: u32 = 28;
const LATER: Duration = Duration::from_millis(600);

fn server() -> Compositor {
    let mut server = Compositor::new(Size::from(SCREEN)).expect("headless compositor");
    server.set_reduce_motion(true);
    server
}

fn decorated(server: &mut Compositor) -> TestClient {
    let mut client = TestClient::connect(server);
    client.use_seat(server);
    let pixels = solid(BLUE, WINDOW);
    let image = xrgb(&pixels, WINDOW);
    client.show_decorated_window(server, &image, Some(DecorationMode::ServerSide));
    client.exchange(server);
    client.commit_buffer(0, &image, (0, 0, WINDOW.0, WINDOW.1));
    client.exchange(server);
    client
}

fn on_window(server: &Compositor, (x, y): (i32, i32)) -> Point<f64, Logical> {
    let location = geometry(server).location;
    at(location.x + x, location.y + y)
}

fn press(server: &mut Compositor, button: u32, location: Point<f64, Logical>) {
    server.move_pointer_to(location, 10);
    server.press_button(button, true, 10);
    server.press_button(button, false, 11);
}

fn choose(server: &mut Compositor, title: &str) {
    let item = server.window_menu_item(title).expect("a menu item");
    press(server, BTN_LEFT, item);
}

fn open_window_menu(server: &mut Compositor) {
    let title = on_window(server, (150, TITLE_BAR / 2));
    press(server, BTN_RIGHT, title);
}

fn zoomed(client: &TestClient) -> bool {
    client
        .configured(0)
        .is_some_and(|configured| configured.states.contains(&ToplevelState::Maximized))
}

fn tiling_items() -> Vec<String> {
    [
        "Left",
        "Right",
        "Top Left",
        "Top Right",
        "Bottom Left",
        "Bottom Right",
        "Fill",
        "Return to Previous Size",
    ]
    .map(str::to_owned)
    .to_vec()
}

#[test]
fn right_clicking_the_title_bar_opens_the_window_menu_and_escape_closes_it() {
    let mut server = server();
    let _client = decorated(&mut server);

    open_window_menu(&mut server);

    assert_eq!(
        server.window_menu(),
        vec![
            [
                "Minimize",
                "Zoom",
                "Move & Resize",
                "Move to Desktop",
                "Close"
            ]
            .map(str::to_owned)
            .to_vec()
        ]
    );
    tap(&mut server, KEY_ESC);
    assert!(server.window_menu().is_empty());
}

#[test]
fn an_open_menu_at_rest_asks_for_no_more_frames() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let _client = decorated(&mut server);
    let output = server.main_output();

    open_window_menu(&mut server);
    let at = settle(&mut server, 0);
    renderer.render(&mut server).expect("frame");
    while server.take_redraw_on(&output) {}
    server.animate(Duration::from_millis(at + 16));

    assert!(!server.take_redraw_on(&output));
}

#[test]
fn a_click_beside_the_menu_closes_it_without_touching_the_window() {
    let mut server = server();
    let _client = decorated(&mut server);
    let before = geometry(&server);

    open_window_menu(&mut server);
    press(&mut server, BTN_LEFT, at(SCREEN.0 - 5, SCREEN.1 - 5));

    assert!(server.window_menu().is_empty());
    assert_eq!(geometry(&server), before);
}

#[test]
fn the_keyboard_walks_into_move_and_resize_and_tiles_the_window_left() {
    let mut server = server();
    let mut client = decorated(&mut server);

    open_window_menu(&mut server);
    for _ in 0..3 {
        tap(&mut server, KEY_DOWN);
    }
    tap(&mut server, KEY_RIGHT);
    assert_eq!(server.window_menu().get(1), Some(&tiling_items()));
    tap(&mut server, KEY_ENTER);
    roundtrip(&mut server, &mut [&mut client]);

    assert_eq!(
        server.windows()[0].tile(),
        Some(rect(0, 0, SCREEN.0 / 2, SCREEN.1))
    );
}

#[test]
fn resting_on_the_zoom_button_offers_the_tiling_menu() {
    let mut server = server();
    let mut client = decorated(&mut server);

    server.move_pointer_to(on_window(&server, ZOOM), 10);
    assert!(server.window_menu().is_empty());
    server.window_menu_tick(Instant::now() + LATER);
    assert_eq!(server.window_menu(), vec![tiling_items()]);

    choose(&mut server, "Right");
    roundtrip(&mut server, &mut [&mut client]);

    assert_eq!(
        server.windows()[0].tile(),
        Some(rect(SCREEN.0 / 2, 0, SCREEN.0 / 2, SCREEN.1))
    );
}

#[test]
fn a_quick_click_on_the_zoom_button_zooms_without_the_menu() {
    let mut server = server();
    let mut client = decorated(&mut server);

    let zoom = on_window(&server, ZOOM);
    press(&mut server, BTN_LEFT, zoom);
    server.window_menu_tick(Instant::now() + LATER);
    roundtrip(&mut server, &mut [&mut client]);

    assert!(server.window_menu().is_empty());
    assert!(zoomed(&client));
}

#[test]
fn holding_the_zoom_button_and_releasing_on_fill_zooms_the_window() {
    let mut server = server();
    let mut client = decorated(&mut server);

    server.move_pointer_to(on_window(&server, ZOOM), 10);
    server.press_button(BTN_LEFT, true, 10);
    server.window_menu_tick(Instant::now() + LATER);
    let fill = server.window_menu_item("Fill").expect("a Fill item");
    server.move_pointer_to(fill, 11);
    server.press_button(BTN_LEFT, false, 12);
    roundtrip(&mut server, &mut [&mut client]);

    assert!(zoomed(&client));
}

#[test]
fn move_to_desktop_carries_the_window_to_a_new_desktop() {
    let mut server = server();
    let mut client = decorated(&mut server);

    open_window_menu(&mut server);
    choose(&mut server, "Move to Desktop");
    assert_eq!(
        server.window_menu().get(1),
        Some(&["Desktop 1", "New Desktop"].map(str::to_owned).to_vec())
    );
    choose(&mut server, "New Desktop");
    roundtrip(&mut server, &mut [&mut client]);

    assert_eq!(server.workspace_count(), 2);
    assert_eq!(server.workspace_of(&server.windows()[0]), Some(1));
}

#[test]
fn the_window_menu_matches_its_golden() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let _client = decorated(&mut server);

    open_window_menu(&mut server);
    choose(&mut server, "Move & Resize");
    let left = server.window_menu_item("Left").expect("a Left item");
    server.move_pointer_to(left, 20);
    settle(&mut server, 0);

    renderer.render(&mut server).expect("frame");
    let snapshot = renderer.snapshot(&server).expect("read back");
    unbound_golden::assert_golden_rgba!(
        "window-menu",
        (
            snapshot.size.w.unsigned_abs(),
            snapshot.size.h.unsigned_abs()
        ),
        snapshot.rgba
    );
}

#[test]
fn the_window_menu_follows_the_text_size_and_matches_its_golden() {
    let mut server = server();
    server.set_text_size(TextSize::from_percent(150));
    let mut renderer = Renderer::new().expect("renderer");
    let _client = decorated(&mut server);

    open_window_menu(&mut server);
    let minimize = server
        .window_menu_item("Minimize")
        .expect("a Minimize item");
    let zoom = server.window_menu_item("Zoom").expect("a Zoom item");
    assert_eq!(zoom.y - minimize.y, 37.0);
    choose(&mut server, "Move & Resize");
    let left = server.window_menu_item("Left").expect("a Left item");
    server.move_pointer_to(left, 20);
    settle(&mut server, 0);

    renderer.render(&mut server).expect("frame");
    let snapshot = renderer.snapshot(&server).expect("read back");
    unbound_golden::assert_golden_rgba!(
        "window-menu-150",
        (
            snapshot.size.w.unsigned_abs(),
            snapshot.size.h.unsigned_abs()
        ),
        snapshot.rgba
    );
}
