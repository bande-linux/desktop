use crate::support::*;
use unbound_compositor::{Compositor, Rect, Renderer};
use unbound_test_wayland::{
    Anchor, BLUE, ConstraintAdjustment, GREEN, KeyboardInteractivity, Layer, LayerRequest,
    PopupRequest, RED, ResizeEdge, TestClient, ToplevelState, roundtrip,
};

const WINDOW: (i32, i32) = (64, 48);
const CENTERED: (i32, i32) = (96, 72);
const WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];

fn drag_with(
    server: &mut Compositor,
    client: &mut TestClient,
    from: (i32, i32),
    by: (i32, i32),
    request: impl FnOnce(&mut Compositor, &mut TestClient),
) {
    server.move_pointer_to(at(from.0, from.1), 0);
    server.press_button(BTN_LEFT, true, 0);
    roundtrip(server, &mut [client]);
    request(server, client);
    server.move_pointer_to(at(from.0 + by.0, from.1 + by.1), 1);
    roundtrip(server, &mut [client]);
}

#[test]
fn a_moved_window_follows_the_pointer_and_stays_put_after_release() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    assert_eq!(geometry(&server).location, CENTERED.into());

    drag_with(
        &mut server,
        &mut editor,
        (110, 80),
        (20, 30),
        |server, client| client.request_move(server, 0),
    );
    assert_eq!(geometry(&server).location, (116, 102).into());

    server.press_button(BTN_LEFT, false, 2);
    server.move_pointer_to(at(10, 10), 3);
    roundtrip(&mut server, &mut [&mut editor]);
    assert_eq!(geometry(&server).location, (116, 102).into());
}

#[test]
fn a_window_cannot_be_dragged_under_the_top_bar() {
    let mut server = server();
    let mut shell = top_bar(&mut server);
    let mut editor = window(&mut server, RED, WINDOW);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    let start = geometry(&server).location;

    drag_with(
        &mut server,
        &mut editor,
        (start.x + 10, start.y + 5),
        (0, -150),
        |server, client| client.request_move(server, 0),
    );

    assert_eq!(geometry(&server).location, (start.x, BAR).into());
}

#[test]
fn a_move_without_a_held_button_is_ignored() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    click(&mut server, 110, 80);
    roundtrip(&mut server, &mut [&mut editor]);

    editor.request_move(&mut server, 0);
    server.move_pointer_to(at(150, 150), 1);
    roundtrip(&mut server, &mut [&mut editor]);

    assert_eq!(geometry(&server).location, CENTERED.into());
}

#[test]
fn resizing_from_the_bottom_right_corner_grows_the_window_toward_the_pointer() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);

    drag_with(
        &mut server,
        &mut editor,
        (158, 118),
        (20, 10),
        |server, client| client.request_resize(server, 0, ResizeEdge::BottomRight),
    );
    let configured = editor.configured(0).expect("a configure");
    assert_eq!((configured.width, configured.height), (84, 58));
    assert!(configured.states.contains(&ToplevelState::Resizing));
    editor.fill_configured_size(&mut server, 0, RED);
    assert_eq!(
        geometry(&server),
        Rect {
            location: CENTERED.into(),
            size: (84, 58).into(),
        }
    );

    server.press_button(BTN_LEFT, false, 2);
    roundtrip(&mut server, &mut [&mut editor]);
    let configured = editor.configured(0).expect("a configure");
    assert!(!configured.states.contains(&ToplevelState::Resizing));
}

#[test]
fn resizing_from_the_left_edge_keeps_the_right_edge_in_place() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    let right = CENTERED.0 + WINDOW.0;

    drag_with(
        &mut server,
        &mut editor,
        (97, 90),
        (-20, 0),
        |server, client| client.request_resize(server, 0, ResizeEdge::Left),
    );
    editor.fill_configured_size(&mut server, 0, RED);

    let window = geometry(&server);
    assert_eq!(window.size, (84, 48).into());
    assert_eq!(window.location.x + window.size.w, right);
    assert_eq!(window.location.y, CENTERED.1);
}

#[test]
fn resizing_stops_at_the_minimum_size_the_app_sets() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    editor.set_min_size(&mut server, 0, 50, 40);

    drag_with(
        &mut server,
        &mut editor,
        (158, 118),
        (-40, -40),
        |server, client| client.request_resize(server, 0, ResizeEdge::BottomRight),
    );

    let configured = editor.configured(0).expect("a configure");
    assert_eq!((configured.width, configured.height), (50, 40));
}

#[test]
fn zoom_fills_the_area_beside_the_bar_and_unzoom_restores_the_window() {
    let mut server = server();
    let mut shell = top_bar(&mut server);
    let mut editor = window(&mut server, RED, WINDOW);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    let before = geometry(&server);

    editor.set_maximized(&mut server, 0, true);
    let configured = editor.configured(0).expect("a configure");
    assert_eq!(
        (configured.width, configured.height),
        (OUTPUT.0, OUTPUT.1 - BAR)
    );
    assert!(configured.states.contains(&ToplevelState::Maximized));
    editor.fill_configured_size(&mut server, 0, RED);
    assert_eq!(
        geometry(&server),
        Rect {
            location: (0, BAR).into(),
            size: (OUTPUT.0, OUTPUT.1 - BAR).into(),
        }
    );

    editor.set_maximized(&mut server, 0, false);
    let configured = editor.configured(0).expect("a configure");
    assert!(!configured.states.contains(&ToplevelState::Maximized));
    assert_eq!((configured.width, configured.height), WINDOW);
    editor.fill_configured_size(&mut server, 0, RED);
    assert_eq!(geometry(&server), before);
}

#[test]
fn a_fullscreen_window_covers_the_output_including_the_bar() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut shell = top_bar(&mut server);
    let mut editor = window(&mut server, RED, WINDOW);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    let before = geometry(&server);

    editor.set_fullscreen(&mut server, 0, true);
    let configured = editor.configured(0).expect("a configure");
    assert_eq!((configured.width, configured.height), OUTPUT);
    assert!(configured.states.contains(&ToplevelState::Fullscreen));
    editor.fill_configured_size(&mut server, 0, BLUE);
    assert_eq!(geometry(&server).location, (0, 0).into());
    assert_eq!(pixel(&mut renderer, &mut server, 10, 5), rgb(BLUE));

    editor.set_fullscreen(&mut server, 0, false);
    let configured = editor.configured(0).expect("a configure");
    assert!(!configured.states.contains(&ToplevelState::Fullscreen));
    editor.fill_configured_size(&mut server, 0, RED);
    assert_eq!(geometry(&server), before);
    assert_eq!(pixel(&mut renderer, &mut server, 10, 5), rgb(GREEN));
}

#[test]
fn only_the_key_window_is_told_it_is_activated() {
    let mut server = server();
    let mut first = window(&mut server, RED, WINDOW);
    let mut second = window(&mut server, BLUE, WINDOW);
    click(&mut server, 110, 80);
    roundtrip(&mut server, &mut [&mut first, &mut second]);

    let activated = |client: &TestClient| {
        client
            .configured(0)
            .expect("a configure")
            .states
            .contains(&ToplevelState::Activated)
    };
    let focused_first = first.keyboard_focus_on(0);
    assert_eq!(activated(&first), focused_first);
    assert_eq!(activated(&second), !focused_first);
}

#[test]
fn minimizing_hides_the_window_keeps_the_app_active_and_the_shell_brings_it_back() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.join_shell(&mut server);
    let mut behind = window(&mut server, RED, WINDOW);
    let mut front = window(&mut server, BLUE, WINDOW);
    front.set_app_id(&mut server, 0, "org.example.Front");
    shell.activate_app(&mut server, "org.example.Front");
    roundtrip(&mut server, &mut [&mut shell, &mut behind, &mut front]);
    assert!(front.keyboard_focus_on(0));
    assert_eq!(pixel(&mut renderer, &mut server, 110, 80), rgb(BLUE));

    front.set_minimized(&mut server, 0);
    roundtrip(&mut server, &mut [&mut shell, &mut behind, &mut front]);
    assert_eq!(pixel(&mut renderer, &mut server, 110, 80), rgb(RED));
    assert!(!front.keyboard_focus_on(0));
    assert!(!behind.keyboard_focus_on(0));

    shell.activate_app(&mut server, "org.example.Front");
    roundtrip(&mut server, &mut [&mut shell, &mut behind, &mut front]);
    assert_eq!(pixel(&mut renderer, &mut server, 110, 80), rgb(BLUE));
    assert!(front.keyboard_focus_on(0));
}

fn menu(parent: usize, anchor_rect: (i32, i32, i32, i32), size: (i32, i32)) -> PopupRequest {
    PopupRequest {
        parent,
        anchor_rect,
        size,
        adjustment: ConstraintAdjustment::empty(),
        grab: false,
        color: WHITE,
    }
}

#[test]
fn a_popup_is_placed_at_its_anchor_relative_to_the_window() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window(&mut server, RED, WINDOW);

    let popup = editor.open_popup(&mut server, &menu(0, (10, 10, 1, 1), (30, 20)));
    roundtrip(&mut server, &mut [&mut editor]);

    assert_eq!(editor.popup_geometry(popup), Some((11, 11, 30, 20)));
    let (x, y) = (CENTERED.0 + 11, CENTERED.1 + 11);
    assert_eq!(pixel(&mut renderer, &mut server, x + 1, y + 1), rgb(WHITE));
    assert_eq!(pixel(&mut renderer, &mut server, x - 1, y - 1), rgb(RED));
    assert_eq!(server.popup_count(), 1);
}

#[test]
fn a_popup_that_would_leave_the_screen_flips_to_the_other_side_of_its_anchor() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);

    let request = PopupRequest {
        adjustment: ConstraintAdjustment::FlipX,
        ..menu(0, (60, 10, 1, 1), (120, 20))
    };
    let popup = editor.open_popup(&mut server, &request);
    roundtrip(&mut server, &mut [&mut editor]);

    assert_eq!(editor.popup_geometry(popup), Some((-60, 11, 120, 20)));
}

#[test]
fn a_click_outside_a_grabbing_popup_dismisses_it_and_is_not_delivered() {
    let mut server = server();
    let mut bystander = window(&mut server, BLUE, WINDOW);
    bystander.set_maximized(&mut server, 0, true);
    bystander.fill_configured_size(&mut server, 0, BLUE);
    let mut editor = window(&mut server, RED, WINDOW);
    click(&mut server, 110, 80);
    roundtrip(&mut server, &mut [&mut bystander, &mut editor]);
    assert!(editor.keyboard_focus_on(0));

    server.press_button(BTN_LEFT, true, 1);
    roundtrip(&mut server, &mut [&mut bystander, &mut editor]);
    let request = PopupRequest {
        grab: true,
        ..menu(0, (10, 10, 1, 1), (30, 20))
    };
    let popup = editor.open_popup(&mut server, &request);
    server.press_button(BTN_LEFT, false, 2);
    roundtrip(&mut server, &mut [&mut bystander, &mut editor]);
    let delivered = bystander.buttons().len();

    click(&mut server, 5, 5);
    roundtrip(&mut server, &mut [&mut bystander, &mut editor]);

    assert!(editor.popup_dismissed(popup));
    assert_eq!(bystander.buttons().len(), delivered);
    assert!(editor.keyboard_focus_on(0));
    assert_eq!(server.popup_count(), 0);
}

fn grabbing_menu(server: &mut Compositor, client: &mut TestClient, x: i32, y: i32) -> usize {
    server.move_pointer_to(at(x, y), 0);
    server.press_button(BTN_LEFT, true, 1);
    roundtrip(server, &mut [&mut *client]);
    let request = PopupRequest {
        grab: true,
        ..menu(0, (10, 10, 1, 1), (30, 20))
    };
    let popup = client.open_popup(server, &request);
    server.press_button(BTN_LEFT, false, 2);
    roundtrip(server, &mut [&mut *client]);
    popup
}

#[test]
fn a_click_on_the_menus_own_window_closes_it_and_only_the_next_click_reaches_the_window() {
    let mut server = server();
    let mut shell = top_bar(&mut server);
    let mut editor = window(&mut server, RED, WINDOW);
    let origin = geometry(&server).location;
    let popup = grabbing_menu(&mut server, &mut editor, origin.x + 5, origin.y + 5);
    let delivered = editor.buttons().len();

    server.move_pointer_to(at(origin.x + 5, origin.y + 40), 3);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    assert_eq!(
        editor.pointer_focus(),
        None,
        "hovering beside the menu reaches nothing"
    );

    click(&mut server, origin.x + 5, origin.y + 40);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    assert!(editor.popup_dismissed(popup));
    assert_eq!(server.popup_count(), 0);
    assert_eq!(
        editor.buttons().len(),
        delivered,
        "the closing click is consumed"
    );

    click(&mut server, origin.x + 5, origin.y + 40);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);
    assert_eq!(editor.buttons().len(), delivered + 2);
    assert_eq!(editor.pointer_focus(), Some((0, 5.0, 40.0)));
}

#[test]
fn a_hidden_overlay_never_takes_the_pointer() {
    let mut server = server();
    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.use_seat(&mut server);
    shell.show_layer(
        &mut server,
        &LayerRequest {
            layer: Layer::Overlay,
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            size: (0, 0),
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::Exclusive,
            color: GREEN,
        },
    );
    shell.hide_layer(&mut server, 0);
    let mut editor = window(&mut server, RED, WINDOW);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);

    click(&mut server, CENTERED.0 + 5, CENTERED.1 + 5);
    roundtrip(&mut server, &mut [&mut shell, &mut editor]);

    assert!(shell.buttons().is_empty());
    assert_eq!(editor.buttons().len(), 2);
    assert!(editor.keyboard_focus_on(0));
}

#[test]
fn a_grabbing_popup_that_opens_under_a_still_pointer_has_it_at_once() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    let (x, y) = (CENTERED.0 + 11, CENTERED.1 + 11);
    click(&mut server, x - 6, y - 6);
    roundtrip(&mut server, &mut [&mut editor]);
    server.move_pointer_to(at(x + 5, y + 5), 1);
    server.press_button(BTN_LEFT, true, 2);
    roundtrip(&mut server, &mut [&mut editor]);

    let request = PopupRequest {
        grab: true,
        ..menu(0, (10, 10, 1, 1), (30, 20))
    };
    let popup = editor.open_popup(&mut server, &request);
    roundtrip(&mut server, &mut [&mut editor]);

    assert!(editor.pointer_on_popup(popup));
}

#[test]
fn ctrl_super_f_toggles_full_screen_for_the_key_window() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    click(&mut server, 110, 80);
    roundtrip(&mut server, &mut [&mut editor]);

    chord(&mut server, &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_F]);
    roundtrip(&mut server, &mut [&mut editor]);
    let configured = editor.configured(0).expect("a configure");
    assert!(configured.states.contains(&ToplevelState::Fullscreen));
    assert_eq!((configured.width, configured.height), OUTPUT);
    assert!(!editor.keys().iter().any(|(key, _)| *key == KEY_F));

    chord(&mut server, &[KEY_LEFTCTRL, KEY_LEFTMETA, KEY_F]);
    roundtrip(&mut server, &mut [&mut editor]);
    let configured = editor.configured(0).expect("a configure");
    assert!(!configured.states.contains(&ToplevelState::Fullscreen));
}

#[test]
fn super_f_without_ctrl_reaches_the_app() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    click(&mut server, 110, 80);
    roundtrip(&mut server, &mut [&mut editor]);

    chord(&mut server, &[KEY_LEFTMETA, KEY_F]);
    roundtrip(&mut server, &mut [&mut editor]);

    let configured = editor.configured(0).expect("a configure");
    assert!(!configured.states.contains(&ToplevelState::Fullscreen));
    assert!(editor.keys().contains(&(KEY_F, true)));
}

#[test]
fn an_apps_next_window_cascades_from_its_frontmost_one_and_other_apps_stay_centred() {
    let pixels = solid(BLUE, WINDOW);
    let image = xrgb(&pixels, WINDOW);
    let mut server = server();
    let mut first = TestClient::connect(&mut server);
    first.show_window_of(&mut server, &image, "org.example.Terminal");
    first.show_window_of(&mut server, &image, "org.example.Terminal");
    let mut second_process = TestClient::connect(&mut server);
    second_process.show_window_of(&mut server, &image, "org.example.Terminal");
    let mut other = TestClient::connect(&mut server);
    other.show_window_of(&mut server, &image, "org.example.Editor");

    let locations: Vec<(i32, i32)> = server
        .windows()
        .iter()
        .map(|window| (window.location().x, window.location().y))
        .collect();
    assert_eq!(
        locations,
        [
            CENTERED,
            (CENTERED.0 + 28, CENTERED.1 + 28),
            (CENTERED.0 + 56, CENTERED.1 + 56),
            CENTERED
        ]
    );
}

#[test]
fn a_window_kept_on_top_stays_in_front_of_a_newer_window_until_released() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("renderer");
    let mut editor = window(&mut server, RED, WINDOW);
    editor.keep_on_top(&mut server, 0, true);
    let mut viewer = window(&mut server, BLUE, WINDOW);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);
    let middle = (CENTERED.0 + WINDOW.0 / 2, CENTERED.1 + WINDOW.1 / 2);

    let kept = pixel(&mut renderer, &mut server, middle.0, middle.1);
    server.move_pointer_to(at(middle.0, middle.1), 1);
    roundtrip(&mut server, &mut [&mut editor, &mut viewer]);
    let pointed = editor.pointer_focus().is_some() && viewer.pointer_focus().is_none();
    editor.keep_on_top(&mut server, 0, false);
    let released = pixel(&mut renderer, &mut server, middle.0 - 8, middle.1 - 8);

    assert_eq!(kept, rgb(RED));
    assert!(pointed);
    assert_eq!(released, rgb(BLUE));
}

fn escape(server: &mut Compositor, client: &mut TestClient) {
    server.key(KEY_ESC, true, 5);
    server.key(KEY_ESC, false, 6);
    roundtrip(server, &mut [client]);
}

fn assert_released(server: &mut Compositor, client: &mut TestClient) {
    server.press_button(BTN_LEFT, false, 9);
    roundtrip(server, &mut [&mut *client]);
    assert!(!server.pointer_grabbed());
    let before = client.buttons().len();
    click(server, 110, 80);
    roundtrip(server, &mut [&mut *client]);
    assert_eq!(client.buttons().len(), before + 2);
}

#[test]
fn escape_during_a_move_puts_the_window_back_and_ends_the_move() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    drag_with(
        &mut server,
        &mut editor,
        (110, 80),
        (20, 30),
        |server, client| client.request_move(server, 0),
    );
    assert_eq!(geometry(&server).location, (116, 102).into());

    escape(&mut server, &mut editor);
    assert_eq!(geometry(&server).location, CENTERED.into());
    server.move_pointer_to(at(150, 140), 7);
    roundtrip(&mut server, &mut [&mut editor]);

    assert_eq!(geometry(&server).location, CENTERED.into());
    assert!(editor.keys().is_empty());
    assert_released(&mut server, &mut editor);
    assert_eq!(geometry(&server).location, CENTERED.into());
}

#[test]
fn escape_during_a_resize_gives_back_the_starting_size() {
    let mut server = server();
    let mut editor = window(&mut server, RED, WINDOW);
    drag_with(
        &mut server,
        &mut editor,
        (158, 118),
        (20, 10),
        |server, client| client.request_resize(server, 0, ResizeEdge::BottomRight),
    );
    editor.fill_configured_size(&mut server, 0, RED);
    assert_eq!(geometry(&server).size, (84, 58).into());

    escape(&mut server, &mut editor);
    editor.fill_configured_size(&mut server, 0, RED);
    server.move_pointer_to(at(200, 160), 7);
    roundtrip(&mut server, &mut [&mut editor]);

    let configured = editor.configured(0).expect("a configure");
    assert_eq!((configured.width, configured.height), WINDOW);
    assert!(!configured.states.contains(&ToplevelState::Resizing));
    assert_eq!(
        geometry(&server),
        Rect {
            location: CENTERED.into(),
            size: WINDOW.into(),
        }
    );
    assert!(editor.keys().is_empty());
    assert_released(&mut server, &mut editor);
}
