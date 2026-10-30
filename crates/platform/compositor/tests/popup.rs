#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use smithay::utils::{Point, Size};
use unbound_compositor::Headless;
use unbound_test_wayland::{
    Anchor, BLUE, GREEN, KeyboardInteractivity, Layer, LayerRequest, PopupRequest, RED, ShmImage,
    TestClient, roundtrip, roundtrip_until,
};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (256, 192);
const WINDOW: (i32, i32) = (64, 48);
const CENTERED: (i32, i32) = (96, 72);
const BYSTANDER: (i32, i32) = (5, 150);
const BAR: i32 = 24;
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const KEY_ESC: u32 = 1;
const KEY_TAB: u32 = 15;
const KEY_A: u32 = 30;
const KEY_LEFTALT: u32 = 56;
const KEY_UP: u32 = 103;
const KEY_LEFTMETA: u32 = 125;
const EDITOR: &str = "org.example.Editor";
const VIEWER: &str = "org.example.Viewer";
const WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];

fn at((x, y): (i32, i32)) -> Point<f64, smithay::utils::Logical> {
    (f64::from(x), f64::from(y)).into()
}

fn app(server: &mut Headless, color: [u8; 4], app_id: &str) -> TestClient {
    let mut client = TestClient::connect(server);
    client.use_seat(server);
    let pixels = color.repeat((WINDOW.0 * WINDOW.1) as usize);
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
    client.set_app_id(server, 0, app_id);
    client
}

struct Desk {
    server: Headless,
    bystander: TestClient,
    editor: TestClient,
}

impl Desk {
    fn new() -> Self {
        let mut server = Headless::new(Size::from(OUTPUT)).expect("headless compositor");
        let mut bystander = app(&mut server, BLUE, VIEWER);
        bystander.set_maximized(&mut server, 0, true);
        bystander.fill_configured_size(&mut server, 0, BLUE);
        let mut editor = app(&mut server, RED, EDITOR);
        click(&mut server, (CENTERED.0 + 5, CENTERED.1 + 5));
        roundtrip(&mut server, &mut [&mut bystander, &mut editor]);
        assert!(editor.keyboard_focus_on(0));
        Self {
            server,
            bystander,
            editor,
        }
    }

    fn exchange(&mut self) {
        roundtrip(
            &mut self.server,
            &mut [&mut self.bystander, &mut self.editor],
        );
    }

    fn open_menu(&mut self) -> usize {
        self.server
            .move_pointer_to(at((CENTERED.0 + 5, CENTERED.1 + 5)), 0);
        self.server.press_button(BTN_RIGHT, true, 1);
        self.exchange();
        let popup = self.editor.open_popup(&mut self.server, &menu(0));
        self.server.press_button(BTN_RIGHT, false, 2);
        self.exchange();
        assert!(self.server.pointer_grabbed());
        assert!(self.server.keyboard_grabbed());
        popup
    }

    fn type_a(&mut self) -> (usize, usize) {
        let before = (self.editor.keys().len(), self.bystander.keys().len());
        self.server.key(KEY_A, true, 20);
        self.server.key(KEY_A, false, 21);
        self.exchange();
        (
            self.editor.keys().len() - before.0,
            self.bystander.keys().len() - before.1,
        )
    }

    fn assert_free(&mut self) {
        assert!(!self.server.pointer_grabbed(), "a pointer grab remains");
        assert!(!self.server.keyboard_grabbed(), "a keyboard grab remains");
        self.server.move_pointer_to(at(BYSTANDER), 40);
        self.exchange();
        assert!(
            self.bystander.pointer_focus().is_some(),
            "the pointer does not reach another app"
        );
        let before = self.bystander.buttons().len();
        click(&mut self.server, BYSTANDER);
        self.exchange();
        assert_eq!(
            self.bystander.buttons().len(),
            before + 2,
            "a click does not reach another app"
        );
    }
}

fn menu(parent: usize) -> PopupRequest {
    PopupRequest {
        parent,
        anchor_rect: (10, 10, 1, 1),
        size: (30, 20),
        flip_x: false,
        grab: true,
        color: WHITE,
    }
}

fn click(server: &mut Headless, point: (i32, i32)) {
    server.move_pointer_to(at(point), 0);
    server.press_button(BTN_LEFT, true, 0);
    server.press_button(BTN_LEFT, false, 0);
}

fn chord(server: &mut Headless, keys: &[u32]) {
    for &key in keys {
        server.key(key, true, 0);
    }
    for &key in keys.iter().rev() {
        server.key(key, false, 0);
    }
}

#[test]
fn super_tab_while_a_menu_is_open_closes_it_and_the_next_app_gets_keys_and_pointer() {
    let mut desk = Desk::new();
    let popup = desk.open_menu();

    chord(&mut desk.server, &[KEY_LEFTMETA, KEY_TAB]);
    desk.exchange();

    assert!(desk.editor.popup_dismissed(popup));
    assert_eq!(desk.server.popup_count(), 0);
    assert!(desk.bystander.keyboard_focus_on(0));
    assert_eq!(desk.type_a(), (0, 2));
    desk.assert_free();
}

#[test]
fn activating_another_app_from_the_shell_closes_the_menu_and_moves_the_keyboard() {
    let mut desk = Desk::new();
    let mut shell = TestClient::connect_as_shell(&mut desk.server);
    shell.join_shell(&mut desk.server);
    let popup = desk.open_menu();

    shell.activate_app(&mut desk.server, VIEWER);
    desk.exchange();

    assert!(desk.editor.popup_dismissed(popup));
    assert!(desk.bystander.keyboard_focus_on(0));
    assert_eq!(desk.type_a(), (0, 2));
    desk.assert_free();
}

#[test]
fn a_launcher_appearing_over_a_menu_closes_it_and_takes_the_keyboard() {
    let mut desk = Desk::new();
    let mut shell = TestClient::connect_as_shell(&mut desk.server);
    shell.use_seat(&mut desk.server);
    let popup = desk.open_menu();

    shell.show_layer(
        &mut desk.server,
        &LayerRequest {
            layer: Layer::Overlay,
            anchor: Anchor::Top,
            size: (100, 40),
            exclusive_zone: 0,
            keyboard: KeyboardInteractivity::Exclusive,
            color: GREEN,
        },
    );
    roundtrip(
        &mut desk.server,
        &mut [&mut shell, &mut desk.bystander, &mut desk.editor],
    );

    assert!(desk.editor.popup_dismissed(popup));
    assert!(shell.layer_has_keyboard(0));
    desk.server.key(KEY_A, true, 20);
    desk.server.key(KEY_A, false, 21);
    roundtrip(
        &mut desk.server,
        &mut [&mut shell, &mut desk.bystander, &mut desk.editor],
    );
    assert_eq!(shell.keys().len(), 2);
    shell.hide_layer(&mut desk.server, 0);
    desk.exchange();
    desk.assert_free();
}

#[test]
fn opening_the_overview_closes_the_menu() {
    let mut desk = Desk::new();
    let popup = desk.open_menu();

    chord(&mut desk.server, &[KEY_LEFTMETA, KEY_LEFTALT, KEY_UP]);
    desk.exchange();

    assert!(desk.editor.popup_dismissed(popup));
    assert!(!desk.server.pointer_grabbed());
    assert!(!desk.server.keyboard_grabbed());
    chord(&mut desk.server, &[KEY_ESC]);
    desk.exchange();
    desk.assert_free();
}

#[test]
fn a_client_that_destroys_its_menu_frees_the_pointer_and_keeps_its_keyboard() {
    let mut desk = Desk::new();
    let popup = desk.open_menu();

    desk.editor.destroy_popup(&mut desk.server, popup);
    desk.server.dispatch().expect("dispatch");
    desk.exchange();

    assert!(desk.editor.keyboard_focus_on(0));
    assert_eq!(desk.type_a(), (2, 0));
    desk.assert_free();
}

#[test]
fn a_client_that_dies_with_a_menu_open_frees_pointer_and_keyboard() {
    let mut desk = Desk::new();
    desk.open_menu();

    let Desk {
        mut server,
        mut bystander,
        editor,
    } = desk;
    drop(editor);
    roundtrip_until(&mut server, &mut [&mut bystander], |apps| {
        apps[0].keyboard_focus_on(0)
    });

    assert!(!server.pointer_grabbed());
    assert!(!server.keyboard_grabbed());
    click(&mut server, BYSTANDER);
    roundtrip(&mut server, &mut [&mut bystander]);
    assert_eq!(bystander.buttons().len(), 2);
}

#[test]
fn a_hung_client_with_a_menu_open_does_not_hold_the_next_app() {
    let mut desk = Desk::new();
    let popup = desk.open_menu();
    desk.editor.hang();

    chord(&mut desk.server, &[KEY_LEFTMETA, KEY_TAB]);
    desk.exchange();

    assert!(desk.editor.popup_dismissed(popup));
    assert_eq!(desk.type_a(), (0, 2));
    desk.assert_free();
}

#[test]
fn a_second_apps_menu_while_one_is_open_is_refused_cleanly() {
    let mut desk = Desk::new();
    let first = desk.open_menu();

    let second = desk.bystander.open_popup(&mut desk.server, &menu(0));
    desk.exchange();

    assert!(desk.bystander.popup_dismissed(second));
    assert!(!desk.editor.popup_dismissed(first));
    chord(&mut desk.server, &[KEY_ESC]);
    desk.exchange();
    desk.editor.destroy_popup(&mut desk.server, first);
    desk.server.dispatch().expect("dispatch");
    desk.exchange();
    desk.assert_free();
}

#[test]
fn a_bar_menu_closes_on_a_click_on_a_window_and_on_super_tab() {
    let mut desk = Desk::new();
    let mut bar = TestClient::connect_as_shell(&mut desk.server);
    bar.use_seat(&mut desk.server);
    bar.show_layer(
        &mut desk.server,
        &LayerRequest {
            layer: Layer::Top,
            anchor: Anchor::Top | Anchor::Left | Anchor::Right,
            size: (0, BAR as u32),
            exclusive_zone: BAR,
            keyboard: KeyboardInteractivity::OnDemand,
            color: GREEN,
        },
    );
    let open_bar_menu = |desk: &mut Desk, bar: &mut TestClient| {
        desk.server.move_pointer_to(at((20, 5)), 0);
        desk.server.press_button(BTN_LEFT, true, 1);
        roundtrip(
            &mut desk.server,
            &mut [&mut *bar, &mut desk.bystander, &mut desk.editor],
        );
        let popup = bar.open_layer_popup(&mut desk.server, &menu(0));
        desk.server.press_button(BTN_LEFT, false, 2);
        roundtrip(
            &mut desk.server,
            &mut [&mut *bar, &mut desk.bystander, &mut desk.editor],
        );
        assert!(desk.server.pointer_grabbed());
        let delivered = desk.bystander.buttons().len();
        (popup, delivered)
    };

    let (popup, delivered) = open_bar_menu(&mut desk, &mut bar);
    click(&mut desk.server, BYSTANDER);
    roundtrip(
        &mut desk.server,
        &mut [&mut bar, &mut desk.bystander, &mut desk.editor],
    );
    assert!(bar.popup_dismissed(popup));
    assert_eq!(desk.bystander.buttons().len(), delivered);
    desk.assert_free();

    let (popup, _) = open_bar_menu(&mut desk, &mut bar);
    chord(&mut desk.server, &[KEY_LEFTMETA, KEY_TAB]);
    roundtrip(
        &mut desk.server,
        &mut [&mut bar, &mut desk.bystander, &mut desk.editor],
    );
    assert!(bar.popup_dismissed(popup));
    desk.assert_free();
}

#[test]
fn a_drag_out_of_a_menu_ends_the_menu_grab_when_it_drops() {
    let mut desk = Desk::new();
    desk.editor.join_seat(&mut desk.server);
    desk.server
        .move_pointer_to(at((CENTERED.0 + 5, CENTERED.1 + 5)), 0);
    desk.server.press_button(BTN_LEFT, true, 1);
    desk.exchange();
    let popup = desk.editor.open_popup(&mut desk.server, &menu(0));
    let icon = WHITE.repeat(16 * 16);
    desk.editor.start_drag(
        &mut desk.server,
        &ShmImage {
            width: 16,
            height: 16,
            stride: 16 * 4,
            format: Format::Argb8888,
            bytes: &icon,
        },
        (-8, -8),
    );
    desk.exchange();
    desk.server.move_pointer_to(at((40, 40)), 3);
    desk.server.press_button(BTN_LEFT, false, 4);
    desk.exchange();

    assert!(desk.editor.popup_dismissed(popup));
    desk.assert_free();
}
