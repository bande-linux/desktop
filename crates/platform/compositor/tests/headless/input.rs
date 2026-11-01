use crate::support::*;
use bande_compositor::{ARROW_FILL, ARROW_OUTLINE, BACKGROUND, Compositor, Renderer};
use bande_test_wayland::{RED, TestClient, roundtrip};
use smithay::utils::{Logical, Point, Size};

const KEY_C: u32 = 46;
const KEY_F4: u32 = 62;

fn client_with_window(server: &mut Compositor, side: i32) -> TestClient {
    window(server, RED, (side, side))
}

fn click(server: &mut Compositor, at: (f64, f64)) {
    server.move_pointer_to(Point::from(at), 1);
    server.press_button(BTN_LEFT, true, 2);
    server.press_button(BTN_LEFT, false, 3);
}

fn tap(server: &mut Compositor, key: u32) {
    server.key(key, true, 10);
    server.key(key, false, 11);
}

fn chord(server: &mut Compositor, key: u32) {
    server.key(KEY_LEFTMETA, true, 20);
    tap(server, key);
    server.key(KEY_LEFTMETA, false, 21);
}

#[test]
fn clicking_a_window_behind_focuses_and_raises_it() {
    let mut server = server();
    let mut big = client_with_window(&mut server, 128);
    let mut small = client_with_window(&mut server, 32);
    roundtrip(&mut server, &mut [&mut big, &mut small]);
    assert!(small.keyboard_focus_on(0));

    let corner = server.windows()[0].location();
    click(
        &mut server,
        (f64::from(corner.x) + 4.0, f64::from(corner.y) + 4.0),
    );
    roundtrip(&mut server, &mut [&mut big, &mut small]);

    assert!(big.keyboard_focus_on(0));
    assert!(!small.keyboard_focus_on(0));
    assert_eq!(
        server.windows().last().map(|window| window.size()),
        Some(Some(Size::from((128, 128))))
    );
}

#[test]
fn the_pointer_enters_the_surface_under_it_at_surface_coordinates() {
    let mut server = server();
    let mut client = client_with_window(&mut server, 64);
    let origin = server.windows()[0].location();

    server.move_pointer_to(
        Point::from((f64::from(origin.x) + 10.0, f64::from(origin.y) + 20.0)),
        1,
    );
    client.exchange(&mut server);

    assert_eq!(client.pointer_focus(), Some((0, 10.0, 20.0)));
}

#[test]
fn a_window_that_appears_under_a_still_pointer_receives_the_pointer() {
    let mut server = server();
    let still = (128.0, 96.0);
    server.move_pointer_to(Point::from(still), 1);

    let mut client = client_with_window(&mut server, 64);
    client.exchange(&mut server);

    let origin = server.windows()[0].location();
    assert_eq!(
        client.pointer_focus(),
        Some((
            0,
            still.0 - f64::from(origin.x),
            still.1 - f64::from(origin.y)
        ))
    );
}

#[test]
fn a_held_button_keeps_the_pointer_on_its_surface_without_extra_motion() {
    let mut server = server();
    let mut pressed = client_with_window(&mut server, 64);
    let origin = server.windows()[0].location();
    server.move_pointer_to(
        Point::from((f64::from(origin.x) + 10.0, f64::from(origin.y) + 10.0)),
        1,
    );
    server.press_button(BTN_LEFT, true, 2);
    pressed.exchange(&mut server);
    let motions = pressed.pointer_motions();

    let mut above = client_with_window(&mut server, 96);
    roundtrip(&mut server, &mut [&mut pressed, &mut above]);
    roundtrip(&mut server, &mut [&mut pressed, &mut above]);

    assert_eq!(pressed.pointer_motions(), motions);
    assert_eq!(pressed.pointer_focus().map(|(index, _, _)| index), Some(0));
    assert_eq!(above.pointer_focus(), None);
}

#[test]
fn keys_reach_only_the_focused_client() {
    let mut server = server();
    let mut first = client_with_window(&mut server, 128);
    let mut second = client_with_window(&mut server, 32);
    roundtrip(&mut server, &mut [&mut first, &mut second]);

    tap(&mut server, KEY_A);
    roundtrip(&mut server, &mut [&mut first, &mut second]);

    assert_eq!(second.keys(), [(KEY_A, true), (KEY_A, false)]);
    assert!(first.keys().is_empty());
}

#[test]
fn clicking_the_empty_desktop_leaves_no_window_focused() {
    let mut server = server();
    let mut client = client_with_window(&mut server, 32);
    assert!(client.keyboard_focus_on(0));

    click(&mut server, (1.0, 1.0));
    client.exchange(&mut server);

    assert!(!client.keyboard_focus_on(0));
}

#[test]
fn super_w_super_q_and_super_c_never_close_or_quit_the_app() {
    let mut server = server();
    let mut client = client_with_window(&mut server, 32);

    for key in [KEY_Q, KEY_C, KEY_W] {
        chord(&mut server, key);
    }
    client.exchange(&mut server);

    assert!(!client.close_requested(0));
    assert!(client.keys().contains(&(KEY_Q, true)));
    assert!(client.keys().contains(&(KEY_C, true)));
}

#[test]
fn ctrl_shortcuts_reach_the_app_and_alt_f4_closes_it() {
    let mut server = server();
    let mut client = client_with_window(&mut server, 32);

    for key in [KEY_W, KEY_Q, KEY_C] {
        server.key(KEY_LEFTCTRL, true, 20);
        tap(&mut server, key);
        server.key(KEY_LEFTCTRL, false, 21);
    }
    client.exchange(&mut server);

    assert!(!client.close_requested(0));
    for key in [KEY_W, KEY_Q, KEY_C] {
        assert!(client.keys().contains(&(key, true)), "{key}");
    }

    server.key(KEY_LEFTALT, true, 30);
    tap(&mut server, KEY_F4);
    server.key(KEY_LEFTALT, false, 31);
    client.exchange(&mut server);

    assert!(client.close_requested(0));
    assert!(!client.keys().iter().any(|(key, _)| *key == KEY_F4));
}

#[test]
fn a_window_that_inhibits_shortcuts_gets_super_w_instead_of_the_overview() {
    let mut server = server();
    let mut client = client_with_window(&mut server, 32);
    client.inhibit_shortcuts(&mut server, 0);

    chord(&mut server, KEY_W);
    client.exchange(&mut server);

    assert!(!server.overview_shown());
    assert!(client.keys().contains(&(KEY_W, true)));
}

#[test]
fn super_drag_moves_a_window_from_anywhere_inside_it() {
    let mut server = server();
    let mut client = client_with_window(&mut server, 64);
    client.exchange(&mut server);
    let before = geometry(&server);
    let inside = before.loc + Point::from((32, 32));

    server.key(KEY_LEFTMETA, true, 0);
    server.move_pointer_to(inside.to_f64(), 1);
    server.press_button(BTN_LEFT, true, 2);
    server.move_pointer_to((inside + Point::from((20, 15))).to_f64(), 3);
    server.press_button(BTN_LEFT, false, 4);
    server.key(KEY_LEFTMETA, false, 5);
    client.exchange(&mut server);

    assert_eq!(geometry(&server).loc, before.loc + Point::from((20, 15)));
    assert!(!client.buttons().contains(&(BTN_LEFT, true)));
}

#[test]
fn a_plain_drag_inside_a_window_reaches_the_app_and_leaves_the_window_in_place() {
    let mut server = server();
    let mut client = client_with_window(&mut server, 64);
    client.exchange(&mut server);
    let before = geometry(&server);
    let inside = before.loc + Point::from((32, 32));

    server.move_pointer_to(inside.to_f64(), 1);
    server.press_button(BTN_LEFT, true, 2);
    server.move_pointer_to((inside + Point::from((20, 15))).to_f64(), 3);
    server.press_button(BTN_LEFT, false, 4);
    client.exchange(&mut server);

    assert_eq!(geometry(&server).loc, before.loc);
    assert!(client.buttons().contains(&(BTN_LEFT, true)));
}

#[test]
fn ctrl_alt_escape_opens_force_quit_without_typing_escape() {
    let mut server = server();
    let mut client = client_with_window(&mut server, 32);
    server.set_force_quit(std::path::Path::new("/bin/true"));

    server.key(KEY_LEFTCTRL, true, 18);
    server.key(KEY_LEFTALT, true, 19);
    tap(&mut server, KEY_ESC);
    server.key(KEY_LEFTALT, false, 22);
    server.key(KEY_LEFTCTRL, false, 23);
    client.exchange(&mut server);
    let upkeep = server.keep_session_clients_running();

    assert!(upkeep.failures.is_empty());
    assert!(server.force_quit_pid().is_some());
    assert!(!client.keys().iter().any(|(key, _)| *key == KEY_ESC));
}

#[test]
fn the_compositor_draws_an_arrow_where_the_pointer_is_once_it_moves() {
    let mut server = server();
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    renderer.render(&mut server).expect("first frame");
    let rgba = |snapshot: &bande_compositor::Snapshot, x: i32, y: i32| {
        let at = ((y * snapshot.size.w + x) * 4) as usize;
        [
            snapshot.rgba[at],
            snapshot.rgba[at + 1],
            snapshot.rgba[at + 2],
        ]
    };
    let before = renderer.snapshot(&server).expect("read back");
    assert_eq!(rgba(&before, 40, 50), BACKGROUND);

    server.move_pointer_to(Point::from((40.0, 50.0)), 1);
    renderer
        .render(&mut server)
        .expect("frame with the pointer");
    let after = renderer.snapshot(&server).expect("read back");

    let bgr = |bgra: [u8; 4]| [bgra[2], bgra[1], bgra[0]];
    assert_eq!(rgba(&after, 40, 50), bgr(ARROW_OUTLINE));
    assert_eq!(rgba(&after, 41, 55), bgr(ARROW_FILL));
}

fn game_under_pointer(server: &mut Compositor) -> (TestClient, Point<f64, Logical>) {
    let mut game = client_with_window(server, 64);
    game.use_relative_pointer(server);
    let centre = (geometry(server).loc + Point::from((32, 32))).to_f64();
    server.move_pointer_to(centre, 0);
    game.exchange(server);
    (game, centre)
}

#[test]
fn a_locked_pointer_stays_put_while_the_game_gets_relative_motion() {
    let mut server = server();
    let (mut game, centre) = game_under_pointer(&mut server);
    game.lock_pointer(&mut server, 0);
    assert!(game.pointer_constrained());

    server.move_pointer_by(Point::from((40.0, 0.0)), Point::from((30.0, 0.0)), 1_000);
    game.exchange(&mut server);

    assert_eq!(server.pointer_location(), centre);
    assert_eq!(game.relative_motions().last(), Some(&(30.0, 0.0)));
}

#[test]
fn a_confined_pointer_slides_along_the_window_edge() {
    let mut server = server();
    let (mut game, centre) = game_under_pointer(&mut server);
    game.confine_pointer(&mut server, 0);
    assert!(game.pointer_constrained());

    server.move_pointer_by(Point::from((100.0, 5.0)), Point::from((100.0, 5.0)), 1_000);
    game.exchange(&mut server);

    assert_eq!(server.pointer_location(), centre + Point::from((0.0, 5.0)));
}

#[test]
fn a_new_window_in_front_frees_a_locked_pointer() {
    let mut server = server();
    let (mut game, centre) = game_under_pointer(&mut server);
    game.lock_pointer(&mut server, 0);
    let mut other = client_with_window(&mut server, 32);
    roundtrip(&mut server, &mut [&mut game, &mut other]);

    server.move_pointer_by(Point::from((10.0, 0.0)), Point::from((10.0, 0.0)), 1_000);
    game.exchange(&mut server);

    assert_eq!(server.pointer_location(), centre + Point::from((10.0, 0.0)));
    assert!(!game.pointer_constrained());
}
