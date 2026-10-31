use crate::support::*;
use smithay::utils::{Point, Size};
use unbound_compositor::{Compositor, OutputConfig, Rect};
use unbound_test_wayland::{
    Anchor, BLUE, ConstraintAdjustment, GREEN, KeyboardInteractivity, Layer, LayerRequest,
    PopupRequest, RED, ShmImage, TestClient, roundtrip, roundtrip_until,
};
use wayland_client::protocol::wl_shm::Format;

const WINDOW: (i32, i32) = (64, 48);
const CENTERED: (i32, i32) = (96, 72);
const BYSTANDER: (i32, i32) = (5, 150);
const BTN_RIGHT: u32 = 0x111;
const WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];

fn at((x, y): (i32, i32)) -> Point<f64, smithay::utils::Logical> {
    (f64::from(x), f64::from(y)).into()
}

fn app(server: &mut Compositor, color: [u8; 4], app_id: &str) -> TestClient {
    window_of(server, color, WINDOW, app_id)
}

struct Desk {
    server: Compositor,
    bystander: TestClient,
    editor: TestClient,
}

impl Desk {
    fn new() -> Self {
        let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
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
        adjustment: ConstraintAdjustment::empty(),
        grab: true,
        color: WHITE,
    }
}

fn click(server: &mut Compositor, point: (i32, i32)) {
    server.move_pointer_to(at(point), 0);
    server.press_button(BTN_LEFT, true, 0);
    server.press_button(BTN_LEFT, false, 0);
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
fn a_bar_menu_held_open_after_the_bar_gave_the_keyboard_back_leaves_the_app_active() {
    let mut desk = Desk::new();
    let mut bar = TestClient::connect_as_shell(&mut desk.server);
    bar.use_seat(&mut desk.server);
    bar.join_shell(&mut desk.server);
    bar.show_named_layer(
        &mut desk.server,
        &LayerRequest {
            layer: Layer::Top,
            anchor: Anchor::Top | Anchor::Left | Anchor::Right,
            size: (0, BAR as u32),
            exclusive_zone: BAR,
            keyboard: KeyboardInteractivity::OnDemand,
            color: GREEN,
        },
        "bar",
    );
    desk.server.move_pointer_to(at((20, 5)), 0);
    desk.server.press_button(BTN_LEFT, true, 1);
    roundtrip(
        &mut desk.server,
        &mut [&mut bar, &mut desk.bystander, &mut desk.editor],
    );
    bar.activate_app(&mut desk.server, EDITOR);
    roundtrip(
        &mut desk.server,
        &mut [&mut bar, &mut desk.bystander, &mut desk.editor],
    );
    let popup = bar.open_layer_popup(&mut desk.server, &menu(0));
    desk.server.press_button(BTN_LEFT, false, 2);
    roundtrip(
        &mut desk.server,
        &mut [&mut bar, &mut desk.bystander, &mut desk.editor],
    );
    assert!(desk.server.keyboard_grabbed());
    let heard = bar.active_apps().len();

    bar.destroy_popup(&mut desk.server, popup);
    desk.server.dispatch().expect("dispatch");
    roundtrip(
        &mut desk.server,
        &mut [&mut bar, &mut desk.bystander, &mut desk.editor],
    );

    assert!(!desk.server.keyboard_grabbed());
    assert!(
        bar.active_apps()[heard..]
            .iter()
            .all(|active| active == EDITOR),
        "the app was deactivated: {:?}",
        bar.active_apps()
    );
}

#[test]
fn a_bar_menu_over_an_open_panel_hands_the_keyboard_back_to_the_panel_when_it_closes() {
    let mut desk = Desk::new();
    let mut bar = TestClient::connect_as_shell(&mut desk.server);
    bar.use_seat(&mut desk.server);
    let strip = |keyboard, layer| LayerRequest {
        layer,
        anchor: Anchor::Top | Anchor::Left | Anchor::Right,
        size: (0, BAR as u32),
        exclusive_zone: BAR,
        keyboard,
        color: GREEN,
    };
    bar.show_named_layer(
        &mut desk.server,
        &strip(KeyboardInteractivity::OnDemand, Layer::Top),
        "bar",
    );
    bar.show_named_layer(
        &mut desk.server,
        &strip(KeyboardInteractivity::Exclusive, Layer::Overlay),
        "panel",
    );
    roundtrip(
        &mut desk.server,
        &mut [&mut bar, &mut desk.bystander, &mut desk.editor],
    );
    assert!(bar.layer_has_keyboard(1));
    desk.server.move_pointer_to(at((20, 5)), 0);
    desk.server.press_button(BTN_LEFT, true, 1);
    roundtrip(
        &mut desk.server,
        &mut [&mut bar, &mut desk.bystander, &mut desk.editor],
    );
    let popup = bar.open_layer_popup(&mut desk.server, &menu(0));
    desk.server.press_button(BTN_LEFT, false, 2);
    roundtrip(
        &mut desk.server,
        &mut [&mut bar, &mut desk.bystander, &mut desk.editor],
    );
    assert!(desk.server.keyboard_grabbed());

    bar.destroy_popup(&mut desk.server, popup);
    desk.server.dispatch().expect("dispatch");
    roundtrip(
        &mut desk.server,
        &mut [&mut bar, &mut desk.bystander, &mut desk.editor],
    );

    assert!(!desk.server.keyboard_grabbed());
    assert!(bar.layer_has_keyboard(1));
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

const MENU_SIZE: (i32, i32) = (60, 80);
const SIDE: &str = "HEADLESS-2";

fn placed_menu(anchor: (i32, i32), size: (i32, i32)) -> PopupRequest {
    PopupRequest {
        parent: 0,
        anchor_rect: (anchor.0, anchor.1, 1, 1),
        size,
        adjustment: ConstraintAdjustment::FlipX
            | ConstraintAdjustment::FlipY
            | ConstraintAdjustment::SlideX
            | ConstraintAdjustment::SlideY
            | ConstraintAdjustment::ResizeY,
        grab: false,
        color: WHITE,
    }
}

fn shell_layer(server: &mut Compositor, namespace: &str, request: &LayerRequest) -> TestClient {
    let mut shell = TestClient::connect_as_shell(server);
    shell.use_seat(server);
    shell.show_named_layer(server, request, namespace);
    shell
}

fn desktop(server: &mut Compositor) -> TestClient {
    shell_layer(
        server,
        "desktop",
        &LayerRequest {
            layer: Layer::Background,
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            size: (0, 0),
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::OnDemand,
            color: BLUE,
        },
    )
}

fn bottom_bar(server: &mut Compositor) -> TestClient {
    shell_layer(
        server,
        "bar",
        &LayerRequest {
            layer: Layer::Top,
            anchor: Anchor::Bottom | Anchor::Left | Anchor::Right,
            size: (0, BAR as u32),
            exclusive_zone: BAR,
            keyboard: KeyboardInteractivity::OnDemand,
            color: GREEN,
        },
    )
}

fn inside(outer: Rect, inner: Rect) -> bool {
    inner.location.x >= outer.location.x
        && inner.location.y >= outer.location.y
        && inner.location.x + inner.size.w <= outer.location.x + outer.size.w
        && inner.location.y + inner.size.h <= outer.location.y + outer.size.h
}

fn only_popup(server: &Compositor) -> Rect {
    let rects = server.popup_rects();
    assert_eq!(rects.len(), 1, "{rects:?}");
    rects[0]
}

#[test]
fn a_desktop_menu_opened_near_any_edge_or_corner_stays_on_the_screen() {
    let (width, height) = OUTPUT;
    let anchors = [
        (2, 2),
        (width / 2, 2),
        (width - 3, 2),
        (width - 3, height / 2),
        (width - 3, height - 3),
        (width / 2, height - 3),
        (2, height - 3),
        (2, height / 2),
    ];
    for anchor in anchors {
        let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
        let mut desk = desktop(&mut server);
        desk.open_layer_popup(&mut server, &placed_menu(anchor, MENU_SIZE));
        roundtrip(&mut server, &mut [&mut desk]);

        let placed = only_popup(&server);
        assert!(
            inside(server.usable_area(), placed),
            "a menu at {anchor:?} lands at {placed:?}"
        );
        assert_eq!(placed.size, MENU_SIZE.into());
    }
}

#[test]
fn a_desktop_menu_in_the_bottom_right_corner_opens_up_and_to_the_left_of_the_pointer() {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut desk = desktop(&mut server);
    let pointer = (OUTPUT.0 - 10, OUTPUT.1 - 10);
    desk.open_layer_popup(&mut server, &placed_menu(pointer, MENU_SIZE));
    roundtrip(&mut server, &mut [&mut desk]);

    let placed = only_popup(&server);
    assert_eq!(
        (placed.location.x, placed.location.y),
        (pointer.0 - MENU_SIZE.0, pointer.1 - MENU_SIZE.1)
    );
}

#[test]
fn a_desktop_menu_stays_clear_of_the_bar_but_a_bar_menu_may_cover_its_own_bar() {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut bar = bottom_bar(&mut server);
    let mut desk = desktop(&mut server);
    let usable = server.usable_area();
    assert_eq!(usable.size.h, OUTPUT.1 - BAR);

    desk.open_layer_popup(
        &mut server,
        &placed_menu((40, OUTPUT.1 - BAR - 4), MENU_SIZE),
    );
    roundtrip(&mut server, &mut [&mut bar, &mut desk]);
    let placed = only_popup(&server);
    assert!(inside(usable, placed), "{placed:?} covers the bar");

    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut bar = bottom_bar(&mut server);
    let request = PopupRequest {
        adjustment: ConstraintAdjustment::SlideY,
        ..placed_menu((20, 4), (30, 20))
    };
    bar.open_layer_popup(&mut server, &request);
    roundtrip(&mut server, &mut [&mut bar]);
    let placed = only_popup(&server);
    assert_eq!(placed.location.y, OUTPUT.1 - 20);
}

#[test]
fn a_menu_taller_than_the_screen_is_shortened_to_the_usable_height() {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut bar = bottom_bar(&mut server);
    let mut desk = desktop(&mut server);
    desk.open_layer_popup(&mut server, &placed_menu((100, 60), (60, 400)));
    roundtrip(&mut server, &mut [&mut bar, &mut desk]);

    assert_eq!(desk.popup_geometry(0), Some((101, 0, 60, OUTPUT.1 - BAR)));
}

#[test]
fn a_window_menu_is_kept_on_the_display_of_its_anchor() {
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    server.add_output(OutputConfig::virtual_display(SIDE, Size::from((320, 240))));
    server.move_pointer_to(at((OUTPUT.0 + 160, 120)), 0);
    let mut editor = app(&mut server, RED, EDITOR);
    let side = server.output_rect(SIDE).expect("side display");
    let window = server.windows()[0].geometry().expect("mapped");
    assert!(inside(side, window));
    let below_main = OUTPUT.1 + 5 - window.location.y;
    let right_edge = side.location.x + side.size.w - window.location.x - 10;

    let low = editor.open_popup(&mut server, &placed_menu((10, below_main), (60, 30)));
    roundtrip(&mut server, &mut [&mut editor]);
    assert_eq!(
        only_popup(&server).location.y,
        OUTPUT.1 + 6,
        "the side display is taller, so the menu need not flip"
    );
    editor.destroy_popup(&mut server, low);

    editor.open_popup(&mut server, &placed_menu((right_edge, 10), (60, 30)));
    roundtrip(&mut server, &mut [&mut editor]);
    let placed = only_popup(&server);
    assert!(inside(side, placed), "{placed:?} leaves {side:?}");
    assert_eq!(placed.location.x + 60, window.location.x + right_edge);
}
