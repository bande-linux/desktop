#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use bande_graphics::{
    Color, Event, Key, Modifiers, Palette, Pointer, PointerButton, Rect, Scene, Scheme,
    ScrollPhase, ScrollSource, TextSystem,
};
use bande_toolkit_platform::{
    App, Edges, FrameRegions, Header, KeyboardFocus, LayerKind, LayerOptions, MenuPlacement, Role,
    SHEET_INSET, Transfer, WINDOW_HEADER_HEIGHT, Window, WindowEvent,
};
use bande_widgets::{Context, ContextMenu, Label, Menu, MenuItem, Ui};
use smithay::backend::input::{Axis, AxisSource};
use smithay::input::pointer::AxisFrame;
use smithay::utils::Point;

#[path = "support/desktop.rs"]
mod desktop;

use desktop::{Client, Desktop};

const OUTPUT: (i32, i32) = (320, 240);
const WINDOW: (u32, u32) = (120, 80);
const KEY_A: u32 = 30;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTALT: u32 = 56;
const KEY_LEFTCTRL: u32 = 29;
const KEY_CAPSLOCK: u32 = 58;
const KEY_C: u32 = 46;
const KEY_LEFTMETA: u32 = 125;
const KEY_SPACE: u32 = 57;
const KEY_ESC: u32 = 1;
const KEY_DOWN: u32 = 108;
const KEY_RIGHT: u32 = 106;
const KEY_ENTER: u32 = 28;
const KEY_D: u32 = 32;
const KEY_COMPOSE: u32 = 127;
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_SIDE: u32 = 0x113;
const BUSY: Duration = Duration::from_millis(900);

fn sample_menu() -> Menu {
    Menu::new(vec![
        MenuItem::new("Open", "open").shortcut("Ctrl+O"),
        MenuItem::new("Rename", "rename").enabled(false),
        MenuItem::separator(),
        MenuItem::submenu(
            "Sort By",
            Menu::new(vec![
                MenuItem::new("Name", "sort-name").checked(true),
                MenuItem::new("Date", "sort-date"),
            ]),
        ),
        MenuItem::new("Delete", "delete")
            .alternate(MenuItem::new("Delete Immediately", "delete-now"))
            .shift_alternate(MenuItem::new("Delete All", "delete-all")),
    ])
}

struct Forwarder {
    events: Sender<WindowEvent>,
    commands: UnixStream,
    finished: bool,
    refuse_close: bool,
    ui: Option<Ui>,
    menu: fn() -> Menu,
}

fn menu_ui(events: Sender<WindowEvent>) -> Ui {
    let area = ContextMenu::new(Label::new("Report.pdf"), sample_menu, move |action| {
        let _ = events.send(WindowEvent::MenuAction(format!("ui:{action}")));
    });
    let mut ui = Ui::new(
        Box::new(area),
        Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
    );
    ui.layout(bande_widgets::Size::new(WINDOW.0 as f32, WINDOW.1 as f32));
    ui
}

impl App for Forwarder {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        Scene {
            width,
            height,
            background: Color::hex(0x3070D0),
            items: Vec::new(),
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let event = match &mut self.ui {
            Some(ui) => match window.drive(ui, event).expect("drive the ui") {
                Some(event) => event,
                None => return,
            },
            None => event,
        };
        if let WindowEvent::Input(Event::PointerDown(pointer)) = &event
            && pointer.button == PointerButton::Secondary
        {
            window
                .show_menu((self.menu)(), MenuPlacement::AtPoint(pointer.position))
                .expect("menu");
        }
        let _ = self.events.send(event);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.commands.as_fd()]
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        let mut command = [0u8; 1];
        if self.commands.read(&mut command).unwrap_or(0) == 0 {
            self.finished = true;
            return;
        }
        match command[0] {
            b'c' => window.copy("copied from the toolkit"),
            b'p' => window.paste().expect("paste"),
            b'k' => window.copy_transfer(
                Transfer::uris(vec![
                    "file:///home/user/a.txt".to_owned(),
                    "file:///home/user/b%20c.txt".to_owned(),
                ])
                .cut(),
            ),
            b'P' => window.paste_transfer().expect("paste files"),
            b'h' => window.hide().expect("hide"),
            b's' => window.show().expect("show"),
            b'b' => thread::sleep(BUSY),
            b'u' => self.ui = Some(menu_ui(self.events.clone())),
            b'H' => {
                window.set_frame_regions(FrameRegions::default());
                window.set_header(Header {
                    title_x: 120.0,
                    items: vec![Rect::new(100.0, 10.0, 16.0, 28.0)],
                    tabs: vec![Rect::new(80.0, 10.0, 18.0, 28.0)],
                });
                window.request_redraw();
            }
            b'r' => {
                self.refuse_close = true;
                window.request_close();
            }
            b'q' => {
                self.refuse_close = false;
                window.request_close();
            }
            _ => self.finished = true,
        }
    }

    fn finished(&self) -> bool {
        self.finished
    }

    fn should_close(&mut self, _window: &mut Window) -> bool {
        let _ = self.events.send(WindowEvent::CloseRequested);
        !self.refuse_close
    }
}

impl Client {
    fn send(&mut self, command: u8) {
        std::io::Write::write_all(&mut self.commands, &[command]).expect("command");
    }
}

impl Desktop {
    fn new() -> Self {
        Self::sized(OUTPUT)
    }

    fn open(&mut self, role: Role, as_shell: bool) -> Client {
        self.open_with_menu(role, as_shell, sample_menu)
    }

    fn open_with_menu(&mut self, role: Role, as_shell: bool, menu: fn() -> Menu) -> Client {
        self.open_app(role, as_shell, WINDOW, move |events, commands| Forwarder {
            events,
            commands,
            finished: false,
            refuse_close: false,
            ui: None,
            menu,
        })
    }

    fn click(&mut self, button: u32) {
        self.server.press_button(button, true, 0);
        self.server.press_button(button, false, 0);
    }

    fn point_into_content(&mut self, x: i32, y: i32) {
        self.point_into_window(x + SHEET_INSET as i32, y + WINDOW_HEADER_HEIGHT as i32);
    }

    fn point_into_window(&mut self, x: i32, y: i32) {
        let origin = self.server.windows()[0].location();
        self.server.move_pointer_to(
            Point::from((f64::from(origin.x + x), f64::from(origin.y + y))),
            0,
        );
    }

    fn type_keys(&mut self, keys: &[u32]) {
        for key in keys {
            self.server.key(*key, true, 0);
        }
        for key in keys.iter().rev() {
            self.server.key(*key, false, 0);
        }
    }
}

fn typed(events: &[WindowEvent]) -> Vec<(Key, Modifiers)> {
    events
        .iter()
        .filter_map(|event| match event {
            WindowEvent::Input(Event::Key { key, modifiers }) => Some((*key, *modifiers)),
            _ => None,
        })
        .collect()
}

#[test]
fn typed_keys_arrive_as_characters_with_their_modifiers() {
    let mut desktop = Desktop::new();
    let mut client = desktop.open(Role::toplevel("keys", "test.keys"), false);
    desktop.wait_for(
        &mut [&mut client],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );

    desktop.type_keys(&[KEY_A]);
    desktop.type_keys(&[KEY_LEFTSHIFT, KEY_A]);
    desktop.type_keys(&[KEY_LEFTCTRL, KEY_C]);
    desktop.wait_for(
        &mut [&mut client],
        |events| typed(events).len() >= 3,
        |_| true,
    );

    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    let control = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    assert_eq!(
        typed(&client.seen),
        [
            (Key::Character('a'), Modifiers::default()),
            (Key::Character('A'), shift),
            (Key::Character('c'), control),
        ]
    );
}

#[test]
fn caps_lock_changes_typed_letters_but_not_the_letter_of_a_shortcut() {
    let mut desktop = Desktop::new();
    let mut client = desktop.open(Role::toplevel("caps keys", "test.caps-keys"), false);
    desktop.wait_for(
        &mut [&mut client],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );

    desktop.type_keys(&[KEY_CAPSLOCK]);
    desktop.type_keys(&[KEY_A]);
    desktop.type_keys(&[KEY_LEFTCTRL, KEY_C]);
    desktop.type_keys(&[KEY_LEFTSHIFT, KEY_LEFTCTRL, KEY_C]);
    desktop.wait_for(
        &mut [&mut client],
        |events| typed(events).len() >= 3,
        |_| true,
    );

    let control = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    assert_eq!(
        typed(&client.seen),
        [
            (Key::Character('A'), Modifiers::default()),
            (Key::Character('c'), control),
            (
                Key::Character('C'),
                Modifiers {
                    shift: true,
                    ..control
                }
            ),
        ]
    );
}

#[test]
fn a_held_key_repeats_after_the_compositors_delay() {
    let mut desktop = Desktop::new();
    let mut client = desktop.open(Role::toplevel("repeat", "test.repeat"), false);
    desktop.wait_for(
        &mut [&mut client],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );

    let pressed = Instant::now();
    desktop.server.key(KEY_A, true, 0);
    desktop.wait_for(
        &mut [&mut client],
        |events| typed(events).len() >= 3,
        |_| true,
    );
    let held = pressed.elapsed();
    desktop.server.key(KEY_A, false, 0);

    assert!(
        held >= Duration::from_millis(600),
        "repeated after {held:?}"
    );
    assert!(
        typed(&client.seen)
            .iter()
            .all(|(key, _)| *key == Key::Character('a'))
    );
}

#[test]
fn a_key_released_while_the_app_is_busy_does_not_repeat() {
    let mut desktop = Desktop::new();
    let mut client = desktop.open(Role::toplevel("busy", "test.busy"), false);
    desktop.wait_for(
        &mut [&mut client],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );

    desktop.server.key(KEY_A, true, 0);
    desktop.wait_for(
        &mut [&mut client],
        |events| !typed(events).is_empty(),
        |_| true,
    );
    client.send(b'b');
    desktop.settle(&mut [&mut client]);
    desktop.server.key(KEY_A, false, 0);
    let until = Instant::now() + BUSY + Duration::from_millis(600);
    while Instant::now() < until {
        desktop.settle(&mut [&mut client]);
    }

    assert_eq!(
        typed(&client.seen),
        [(Key::Character('a'), Modifiers::default())]
    );
}

fn presses(events: &[WindowEvent]) -> Vec<Pointer> {
    events
        .iter()
        .filter_map(|event| match event {
            WindowEvent::Input(Event::PointerDown(pointer)) => Some(*pointer),
            _ => None,
        })
        .collect()
}

fn releases(events: &[WindowEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, WindowEvent::Input(Event::PointerUp(_))))
        .count()
}

fn pointed_window(desktop: &mut Desktop, name: &str) -> Client {
    let mut client = desktop.open(Role::toplevel(name, "test.pointer"), false);
    desktop.wait_for(
        &mut [&mut client],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );
    desktop.point_into_content(30, 20);
    client
}

#[test]
fn clicks_arrive_at_surface_coordinates_with_their_button() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "buttons");
    desktop.click(BTN_LEFT);
    desktop.click(BTN_RIGHT);
    desktop.click(BTN_SIDE);
    desktop.wait_for(&mut [&mut client], |events| releases(events) == 3, |_| true);

    let pressed = presses(&client.seen);
    assert!(
        pressed
            .iter()
            .all(|pointer| (pointer.position.x, pointer.position.y) == (30.0, 20.0))
    );
    assert_eq!(
        pressed
            .iter()
            .map(|pointer| pointer.button)
            .collect::<Vec<_>>(),
        [
            PointerButton::Primary,
            PointerButton::Secondary,
            PointerButton::Back
        ]
    );
}

#[test]
fn quick_clicks_in_one_place_count_up_and_a_moved_click_starts_over() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "counts");
    for _ in 0..3 {
        desktop.click(BTN_LEFT);
    }
    desktop.point_into_content(60, 40);
    desktop.click(BTN_LEFT);
    desktop.wait_for(&mut [&mut client], |events| releases(events) == 4, |_| true);

    let counts = presses(&client.seen)
        .iter()
        .map(|pointer| pointer.count)
        .collect::<Vec<_>>();
    assert_eq!(counts, [1, 2, 3, 1]);
}

#[test]
fn control_click_is_a_primary_click_with_control_held_and_modifier_changes_arrive_alone() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "control");
    desktop.server.key(KEY_LEFTCTRL, true, 0);
    desktop.click(BTN_LEFT);
    desktop.server.key(KEY_LEFTCTRL, false, 0);
    desktop.wait_for(
        &mut [&mut client],
        |events| {
            events
                .iter()
                .filter(|event| matches!(event, WindowEvent::Input(Event::ModifiersChanged(_))))
                .count()
                == 2
        },
        |_| true,
    );

    let pressed = presses(&client.seen);
    assert_eq!(pressed.len(), 1);
    assert_eq!(pressed[0].button, PointerButton::Primary);
    assert!(pressed[0].modifiers.control);
    assert!(
        client
            .seen
            .contains(&WindowEvent::Input(Event::ModifiersChanged(Modifiers {
                control: true,
                ..Modifiers::default()
            })))
    );
}

#[test]
fn caps_lock_is_reported_when_it_locks_and_unlocks() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "caps");
    for _ in 0..2 {
        desktop.server.key(KEY_CAPSLOCK, true, 0);
        desktop.server.key(KEY_CAPSLOCK, false, 0);
    }
    let reports = |events: &[WindowEvent]| {
        events
            .iter()
            .filter_map(|event| match event {
                WindowEvent::CapsLock(on) => Some(*on),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    desktop.wait_for(
        &mut [&mut client],
        |events| reports(events).len() == 2,
        |_| true,
    );

    assert_eq!(reports(&client.seen), [true, false]);
}

#[test]
fn finger_scrolling_begins_changes_and_ends_with_the_lift() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "scroll");
    for delta in [4.0, 6.0] {
        desktop.server.scroll(
            AxisFrame::new(0)
                .source(AxisSource::Finger)
                .value(Axis::Vertical, delta),
        );
    }
    desktop.server.scroll(
        AxisFrame::new(0)
            .source(AxisSource::Finger)
            .stop(Axis::Vertical),
    );
    desktop.server.scroll(
        AxisFrame::new(0)
            .source(AxisSource::Wheel)
            .value(Axis::Vertical, 15.0),
    );
    let scrolls = |events: &[WindowEvent]| {
        events
            .iter()
            .filter_map(|event| match event {
                WindowEvent::Input(Event::Scroll(scroll)) => {
                    Some((scroll.source, scroll.phase, scroll.delta.y))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    desktop.wait_for(
        &mut [&mut client],
        |events| scrolls(events).len() == 4,
        |_| true,
    );

    assert_eq!(
        scrolls(&client.seen),
        [
            (ScrollSource::Finger, ScrollPhase::Began, 4.0),
            (ScrollSource::Finger, ScrollPhase::Changed, 6.0),
            (ScrollSource::Finger, ScrollPhase::Ended, 0.0),
            (ScrollSource::Wheel, ScrollPhase::Changed, 15.0),
        ]
    );
}

#[test]
fn an_app_can_refuse_to_close_and_close_later() {
    let mut desktop = Desktop::new();
    let mut client = desktop.open(Role::toplevel("close", "test.close"), false);
    desktop.wait_for(&mut [&mut client], |_| true, Desktop::mapped);
    client.send(b'r');
    desktop.wait_for(
        &mut [&mut client],
        |events| events.contains(&WindowEvent::CloseRequested),
        |_| true,
    );
    desktop.settle(&mut [&mut client]);
    assert!(
        client
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    );

    client.send(b'q');
    let deadline = Instant::now() + Duration::from_secs(10);
    while client
        .thread
        .as_ref()
        .is_some_and(|thread| !thread.is_finished())
    {
        desktop.pump();
        assert!(Instant::now() < deadline, "the window stayed open");
    }
}

#[test]
fn text_copied_in_one_window_is_pasted_in_another() {
    let mut desktop = Desktop::new();
    let mut source = desktop.open(Role::toplevel("source", "test.source"), false);
    desktop.wait_for(
        &mut [&mut source],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );
    desktop.type_keys(&[KEY_A]);
    desktop.wait_for(
        &mut [&mut source],
        |events| !typed(events).is_empty(),
        |_| true,
    );
    source.send(b'c');
    desktop.settle(&mut [&mut source]);

    let mut target = desktop.open(Role::toplevel("target", "test.target"), false);
    desktop.wait_for(
        &mut [&mut target],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );
    target.send(b'p');
    desktop.wait_for(
        &mut [&mut source, &mut target],
        |events| {
            events
                .iter()
                .any(|event| matches!(event, WindowEvent::Pasted(_)))
        },
        |_| true,
    );

    assert!(
        target
            .seen
            .contains(&WindowEvent::Pasted("copied from the toolkit".to_owned()))
    );
}

#[test]
fn cut_files_reach_another_app_as_uris_marked_as_cut() {
    let mut desktop = Desktop::new();
    let mut source = desktop.open(Role::toplevel("source", "test.source"), false);
    desktop.wait_for(
        &mut [&mut source],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );
    desktop.type_keys(&[KEY_A]);
    desktop.wait_for(
        &mut [&mut source],
        |events| !typed(events).is_empty(),
        |_| true,
    );
    source.send(b'k');
    desktop.settle(&mut [&mut source]);
    let mut target = desktop.open(Role::toplevel("target", "test.target"), false);
    desktop.wait_for(
        &mut [&mut target],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );
    target.send(b'P');
    desktop.wait_for(
        &mut [&mut source, &mut target],
        |events| {
            events
                .iter()
                .any(|event| matches!(event, WindowEvent::PastedTransfer(_)))
        },
        |_| true,
    );
    let pasted = target.seen.iter().find_map(|event| match event {
        WindowEvent::PastedTransfer(transfer) => Some(transfer.clone()),
        _ => None,
    });
    let pasted = pasted.expect("files pasted");
    assert_eq!(
        pasted.uris,
        ["file:///home/user/a.txt", "file:///home/user/b%20c.txt"]
    );
    assert!(pasted.cut);
}

#[test]
fn a_launcher_overlay_takes_the_keyboard_while_shown_and_returns_it_when_hidden() {
    let mut desktop = Desktop::new();
    let mut app = desktop.open(Role::toplevel("app", "test.app"), false);
    desktop.wait_for(
        &mut [&mut app],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );
    let mut launcher = desktop.open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: 0,
            keyboard: KeyboardFocus::Exclusive,
            namespace: "launcher".to_owned(),
        }),
        true,
    );
    desktop.wait_for(
        &mut [&mut launcher],
        |events| events.contains(&WindowEvent::Focus(true)),
        |_| true,
    );
    desktop.type_keys(&[KEY_A]);
    desktop.wait_for(
        &mut [&mut launcher],
        |events| !typed(events).is_empty(),
        |_| true,
    );

    launcher.send(b'h');
    desktop.wait_for(
        &mut [&mut app],
        |events| {
            events
                .iter()
                .filter(|event| **event == WindowEvent::Focus(true))
                .count()
                == 2
        },
        |_| true,
    );
    desktop.type_keys(&[KEY_LEFTMETA, KEY_SPACE]);
    desktop.wait_for(
        &mut [&mut launcher],
        |events| events.contains(&WindowEvent::ShowLauncher),
        |_| true,
    );

    assert!(typed(&app.seen).is_empty());

    launcher.send(b's');
    let focused_before = launcher
        .seen
        .iter()
        .filter(|event| **event == WindowEvent::Focus(true))
        .count();
    desktop.wait_for(
        &mut [&mut launcher],
        |events| {
            events
                .iter()
                .filter(|event| **event == WindowEvent::Focus(true))
                .count()
                > focused_before
        },
        |_| true,
    );
}

fn finish(desktop: &mut Desktop, client: &Client) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while client
        .thread
        .as_ref()
        .is_some_and(|thread| !thread.is_finished())
    {
        desktop.pump();
        assert!(Instant::now() < deadline, "the window stayed open");
    }
}

#[test]
fn the_close_button_asks_the_app_and_closes_the_window() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "close button");
    desktop.point_into_window(18, 14);
    desktop.click(BTN_LEFT);
    finish(&mut desktop, &client);
    client.seen.extend(client.events.try_iter());
    assert!(client.seen.contains(&WindowEvent::CloseRequested));
    assert!(presses(&client.seen).is_empty(), "the app saw no click");
}

#[test]
fn a_double_click_on_the_title_bar_zooms_the_window() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "zoom");
    desktop.point_into_window(100, 14);
    desktop.click(BTN_LEFT);
    desktop.click(BTN_LEFT);
    desktop.wait_for(
        &mut [&mut client],
        |events| {
            events.iter().any(
                |event| matches!(event, WindowEvent::ToplevelChanged(state) if state.maximized),
            )
        },
        |_| true,
    );
    assert!(presses(&client.seen).is_empty());
}

#[test]
fn dragging_the_title_bar_moves_the_window_and_the_app_sees_the_pointer_leave() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "move");
    let before = desktop.server.windows()[0].location();
    desktop.point_into_window(100, 14);
    desktop.server.press_button(BTN_LEFT, true, 0);
    desktop.settle(&mut [&mut client]);
    desktop.server.move_pointer_to(
        Point::from((f64::from(before.x + 130), f64::from(before.y + 44))),
        0,
    );
    desktop.server.press_button(BTN_LEFT, false, 0);
    desktop.wait_for(
        &mut [&mut client],
        |events| events.contains(&WindowEvent::Input(Event::PointerLeave)),
        |desktop| desktop.server.windows()[0].location() != before,
    );
    let after = desktop.server.windows()[0].location();
    assert_eq!((after.x - before.x, after.y - before.y), (30, 30));
}

#[test]
fn a_drag_from_a_header_tab_or_empty_header_space_moves_the_window_but_header_buttons_stay_the_apps()
 {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "header");
    client.send(b'H');
    desktop.settle(&mut [&mut client]);
    let origin = desktop.server.windows()[0].location();

    desktop.point_into_window(108, 24);
    desktop.server.press_button(BTN_LEFT, true, 0);
    desktop.settle(&mut [&mut client]);
    desktop.point_into_window(108, 60);
    desktop.server.press_button(BTN_LEFT, false, 0);
    desktop.wait_for(&mut [&mut client], |events| releases(events) == 1, |_| true);
    assert_eq!(desktop.server.windows()[0].location(), origin);

    for (start, by) in [((88, 24), (30_i32, 30_i32)), ((90, 4), (-20, 10))] {
        let before = desktop.server.windows()[0].location();
        desktop.point_into_window(start.0, start.1);
        desktop.server.press_button(BTN_LEFT, true, 0);
        desktop.settle(&mut [&mut client]);
        for step in [by.0.signum() * 8, by.0] {
            desktop.server.move_pointer_to(
                Point::from((
                    f64::from(before.x + start.0 + step),
                    f64::from(before.y + start.1 + by.1 * step / by.0),
                )),
                0,
            );
            desktop.settle(&mut [&mut client]);
        }
        desktop.server.press_button(BTN_LEFT, false, 0);
        desktop.wait_for(
            &mut [&mut client],
            |_| true,
            |desktop| desktop.server.windows()[0].location() != before,
        );
        let after = desktop.server.windows()[0].location();
        assert_eq!((after.x - before.x, after.y - before.y), by);
    }
    assert_eq!(presses(&client.seen).len(), 2);
    assert!(
        client
            .seen
            .contains(&WindowEvent::Input(Event::PointerLeave))
    );
}

fn drag_from(desktop: &mut Desktop, client: &mut Client, from: (i32, i32), by: (i32, i32)) {
    let at = |(x, y): (i32, i32)| Point::from((f64::from(x), f64::from(y)));
    desktop.server.move_pointer_to(at(from), 0);
    desktop.server.press_button(BTN_LEFT, true, 0);
    desktop.settle(&mut [client]);
    desktop
        .server
        .move_pointer_to(at((from.0 + by.0, from.1 + by.1)), 0);
    desktop.settle(&mut [client]);
    desktop.server.press_button(BTN_LEFT, false, 0);
}

fn window_rect(desktop: &Desktop) -> (i32, i32, i32, i32) {
    let geometry = desktop.server.windows()[0].geometry().expect("mapped");
    (
        geometry.loc.x,
        geometry.loc.y,
        geometry.size.w,
        geometry.size.h,
    )
}

#[test]
fn dragging_just_outside_the_right_edge_resizes_the_window() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "resize");
    let (x, y, width, height) = window_rect(&desktop);

    drag_from(&mut desktop, &mut client, (x + width + 3, y + 60), (40, 0));

    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| window_rect(desktop) == (x, y, width + 40, height),
    );
}

#[test]
fn dragging_just_inside_the_right_edge_resizes_the_window_without_a_press_in_the_content() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "resize-inside");
    let (x, y, width, height) = window_rect(&desktop);

    drag_from(&mut desktop, &mut client, (x + width - 2, y + 60), (40, 0));

    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| window_rect(desktop) == (x, y, width + 40, height),
    );
    assert!(presses(&client.seen).is_empty());
}

#[test]
fn dragging_the_wedge_of_the_bottom_right_corner_resizes_both_ways() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "resize-corner");
    let (x, y, width, height) = window_rect(&desktop);

    drag_from(
        &mut desktop,
        &mut client,
        (x + width - 1, y + height - 1),
        (40, 20),
    );

    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| window_rect(desktop) == (x, y, width + 40, height + 20),
    );
}

#[test]
fn dragging_the_bottom_rim_further_in_still_moves_the_window() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "move-rim");
    let (x, y, width, height) = window_rect(&desktop);

    drag_from(
        &mut desktop,
        &mut client,
        (x + width / 2, y + height - 5),
        (-20, -10),
    );

    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| window_rect(desktop) == (x - 20, y - 10, width, height),
    );
}

fn menu_events(events: &[WindowEvent]) -> Vec<WindowEvent> {
    events
        .iter()
        .filter(|event| matches!(event, WindowEvent::MenuAction(_) | WindowEvent::MenuClosed))
        .cloned()
        .collect()
}

fn open_menu(desktop: &mut Desktop, client: &mut Client) {
    desktop.server.press_button(BTN_RIGHT, true, 0);
    desktop.wait_for(
        &mut [client],
        |_| true,
        |desktop| desktop.server.popup_count() == 1,
    );
}

#[test]
fn a_menu_opens_at_the_pointer_and_a_click_on_an_item_chooses_it() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "menu click");
    open_menu(&mut desktop, &mut client);
    desktop.server.press_button(BTN_RIGHT, false, 0);
    desktop.settle(&mut [&mut client]);
    assert_eq!(
        desktop.server.popup_count(),
        1,
        "a click without a drag keeps it open"
    );
    desktop.point_into_content(30 + 40, 20 + 16);
    desktop.click(BTN_LEFT);
    desktop.wait_for(
        &mut [&mut client],
        |events| !menu_events(events).is_empty(),
        |_| true,
    );
    assert_eq!(
        menu_events(&client.seen),
        [WindowEvent::MenuAction("open".to_owned())]
    );
    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| desktop.server.popup_count() == 0,
    );
}

#[test]
fn releasing_a_held_secondary_button_over_an_item_chooses_it() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "menu drag");
    open_menu(&mut desktop, &mut client);
    desktop.settle(&mut [&mut client]);
    desktop.point_into_content(30 + 40, 20 + 5 + 22 + 22 + 11 + 22 + 11);
    desktop.server.press_button(BTN_RIGHT, false, 0);
    desktop.wait_for(
        &mut [&mut client],
        |events| !menu_events(events).is_empty(),
        |_| true,
    );
    assert_eq!(
        menu_events(&client.seen),
        [WindowEvent::MenuAction("delete".to_owned())]
    );
}

#[test]
fn the_keyboard_walks_into_a_submenu_and_escape_closes_level_by_level() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "menu keys");
    open_menu(&mut desktop, &mut client);
    desktop.server.press_button(BTN_RIGHT, false, 0);
    desktop.settle(&mut [&mut client]);
    desktop.type_keys(&[KEY_DOWN]);
    desktop.type_keys(&[KEY_DOWN]);
    desktop.type_keys(&[KEY_RIGHT]);
    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| desktop.server.popup_count() == 2,
    );
    desktop.type_keys(&[KEY_ESC]);
    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| desktop.server.popup_count() == 1,
    );
    desktop.type_keys(&[KEY_D]);
    desktop.type_keys(&[KEY_ENTER]);
    desktop.wait_for(
        &mut [&mut client],
        |events| !menu_events(events).is_empty(),
        |_| true,
    );
    assert_eq!(
        menu_events(&client.seen),
        [WindowEvent::MenuAction("delete".to_owned())]
    );
}

#[test]
fn escape_or_a_click_outside_closes_a_menu_without_a_choice() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "menu escape");
    open_menu(&mut desktop, &mut client);
    desktop.server.press_button(BTN_RIGHT, false, 0);
    desktop.settle(&mut [&mut client]);
    desktop.type_keys(&[KEY_ESC]);
    desktop.wait_for(
        &mut [&mut client],
        |events| menu_events(events) == [WindowEvent::MenuClosed],
        |desktop| desktop.server.popup_count() == 0,
    );

    open_menu(&mut desktop, &mut client);
    desktop.server.press_button(BTN_RIGHT, false, 0);
    desktop.settle(&mut [&mut client]);
    desktop.server.move_pointer_to(Point::from((2.0, 2.0)), 0);
    desktop.click(BTN_LEFT);
    desktop.wait_for(
        &mut [&mut client],
        |events| menu_events(events).len() == 2,
        |desktop| desktop.server.popup_count() == 0,
    );
    assert_eq!(
        menu_events(&client.seen),
        [WindowEvent::MenuClosed, WindowEvent::MenuClosed]
    );
}

#[test]
fn a_key_typed_right_after_choosing_an_item_reaches_the_window() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "menu chosen then typed");
    open_menu(&mut desktop, &mut client);
    desktop.server.press_button(BTN_RIGHT, false, 0);
    desktop.settle(&mut [&mut client]);
    desktop.type_keys(&[KEY_DOWN]);
    desktop.type_keys(&[KEY_ENTER]);
    desktop.type_keys(&[KEY_D]);
    desktop.wait_for(
        &mut [&mut client],
        |events| {
            typed(events)
                .iter()
                .any(|(key, _)| *key == Key::Character('d'))
        },
        |_| true,
    );
    assert_eq!(
        menu_events(&client.seen),
        [WindowEvent::MenuAction("open".to_owned())]
    );
}

#[test]
fn a_closed_menu_lets_go_of_the_keyboard_while_it_fades_out() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "menu fade");
    open_menu(&mut desktop, &mut client);
    desktop.server.press_button(BTN_RIGHT, false, 0);
    desktop.settle(&mut [&mut client]);
    desktop.type_keys(&[KEY_ESC]);
    desktop.wait_for(
        &mut [&mut client],
        |events| menu_events(events) == [WindowEvent::MenuClosed],
        |desktop| !desktop.server.keyboard_grabbed() && desktop.server.popup_count() == 1,
    );
    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| desktop.server.popup_count() == 0,
    );
}

#[test]
fn a_click_just_beside_a_menu_closes_it_and_only_the_next_click_reaches_the_window() {
    let mut desktop = Desktop::sized((800, 600));
    let mut client = pointed_window(&mut desktop, "menu own window");
    desktop.point_into_content(100, 60);
    open_menu(&mut desktop, &mut client);
    desktop.server.press_button(BTN_RIGHT, false, 0);
    desktop.settle(&mut [&mut client]);
    let before = presses(&client.seen).len();

    desktop.point_into_content(100 - 8, 60 + 10);
    desktop.click(BTN_LEFT);
    desktop.wait_for(
        &mut [&mut client],
        |events| menu_events(events) == [WindowEvent::MenuClosed],
        |desktop| desktop.server.popup_count() == 0,
    );
    desktop.settle(&mut [&mut client]);
    assert_eq!(
        presses(&client.seen).len(),
        before,
        "the closing click is consumed"
    );

    desktop.click(BTN_LEFT);
    desktop.wait_for(
        &mut [&mut client],
        |events| presses(events).len() == before + 1,
        |_| true,
    );
    assert_eq!(
        presses(&client.seen)
            .last()
            .map(|pointer| (pointer.position.x, pointer.position.y)),
        Some((92.0, 70.0))
    );
}

#[test]
fn clicking_a_submenu_item_opens_its_submenu_once() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "menu submenu click");
    open_menu(&mut desktop, &mut client);
    desktop.server.press_button(BTN_RIGHT, false, 0);
    desktop.settle(&mut [&mut client]);
    desktop.point_into_content(30 + 40, 20 + 5 + 22 + 22 + 11 + 11);
    desktop.click(BTN_LEFT);
    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| desktop.server.popup_count() == 2,
    );
    desktop.settle(&mut [&mut client]);
    assert_eq!(desktop.server.popup_count(), 2);
}

#[test]
fn resting_on_a_submenu_item_opens_its_submenu() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "menu hover");
    open_menu(&mut desktop, &mut client);
    desktop.server.press_button(BTN_RIGHT, false, 0);
    desktop.settle(&mut [&mut client]);
    desktop.point_into_content(30 + 40, 20 + 5 + 22 + 22 + 11 + 11);
    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| desktop.server.popup_count() == 2,
    );
}

#[test]
fn the_menu_key_opens_the_focused_widgets_menu_with_its_first_item_ready() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "menu key");
    client.send(b'u');
    desktop.settle(&mut [&mut client]);
    desktop.type_keys(&[KEY_COMPOSE]);
    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| desktop.server.popup_count() == 1,
    );
    desktop.type_keys(&[KEY_ENTER]);
    desktop.wait_for(
        &mut [&mut client],
        |events| !menu_events(events).is_empty(),
        |_| true,
    );
    assert_eq!(
        menu_events(&client.seen),
        [WindowEvent::MenuAction("ui:open".to_owned())]
    );
}

#[test]
fn clicking_an_on_demand_background_layer_gives_it_the_keyboard_until_a_window_is_clicked() {
    let mut desktop = Desktop::new();
    let mut surface = desktop.open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Background,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::OnDemand,
            namespace: "desktop".to_owned(),
        }),
        true,
    );
    let mut app = desktop.open(Role::toplevel("app", "test.app"), false);
    desktop.wait_for(
        &mut [&mut app],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );
    desktop.server.move_pointer_to(Point::from((4.0, 4.0)), 0);
    desktop.click(BTN_LEFT);
    desktop.wait_for(
        &mut [&mut surface],
        |events| events.contains(&WindowEvent::Focus(true)),
        |_| true,
    );
    desktop.type_keys(&[KEY_A]);
    desktop.wait_for(
        &mut [&mut surface],
        |events| !typed(events).is_empty(),
        |_| true,
    );
    desktop.wait_for(
        &mut [&mut app],
        |events| events.contains(&WindowEvent::Focus(false)),
        |_| true,
    );
    desktop.point_into_content(10, 10);
    desktop.click(BTN_LEFT);
    desktop.wait_for(
        &mut [&mut surface],
        |events| events.contains(&WindowEvent::Focus(false)),
        |_| true,
    );
    assert!(typed(&app.seen).is_empty());
}

#[test]
fn holding_alt_swaps_in_an_items_alternate() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "menu alternate");
    open_menu(&mut desktop, &mut client);
    desktop.server.press_button(BTN_RIGHT, false, 0);
    desktop.settle(&mut [&mut client]);
    desktop.server.key(KEY_LEFTALT, true, 0);
    desktop.settle(&mut [&mut client]);
    desktop.point_into_content(30 + 40, 20 + 5 + 22 + 22 + 11 + 22 + 11);
    desktop.click(BTN_LEFT);
    desktop.wait_for(
        &mut [&mut client],
        |events| !menu_events(events).is_empty(),
        |_| true,
    );
    desktop.server.key(KEY_LEFTALT, false, 0);
    assert_eq!(
        menu_events(&client.seen),
        [WindowEvent::MenuAction("delete-now".to_owned())]
    );
}

#[test]
fn holding_shift_swaps_in_an_items_shift_alternate() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "menu shift alternate");
    open_menu(&mut desktop, &mut client);
    desktop.server.press_button(BTN_RIGHT, false, 0);
    desktop.settle(&mut [&mut client]);
    desktop.server.key(KEY_LEFTSHIFT, true, 0);
    desktop.settle(&mut [&mut client]);
    desktop.point_into_content(30 + 40, 20 + 5 + 22 + 22 + 11 + 22 + 11);
    desktop.click(BTN_LEFT);
    desktop.wait_for(
        &mut [&mut client],
        |events| !menu_events(events).is_empty(),
        |_| true,
    );
    desktop.server.key(KEY_LEFTSHIFT, false, 0);
    assert_eq!(
        menu_events(&client.seen),
        [WindowEvent::MenuAction("delete-all".to_owned())]
    );
}

#[test]
fn the_title_bar_menu_puts_the_window_in_full_screen() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "window menu");
    desktop.point_into_window(90, 14);
    desktop.server.press_button(BTN_RIGHT, true, 0);
    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| desktop.server.popup_count() == 1,
    );
    desktop.server.press_button(BTN_RIGHT, false, 0);
    desktop.settle(&mut [&mut client]);
    desktop.type_keys(&[KEY_DOWN]);
    desktop.type_keys(&[KEY_DOWN]);
    desktop.type_keys(&[KEY_DOWN]);
    desktop.type_keys(&[KEY_ENTER]);
    desktop.wait_for(
        &mut [&mut client],
        |events| {
            events.iter().any(
                |event| matches!(event, WindowEvent::ToplevelChanged(state) if state.fullscreen),
            )
        },
        |desktop| desktop.server.popup_count() == 0,
    );
    assert!(
        menu_events(&client.seen).is_empty(),
        "the app is not involved"
    );
}

const KEY_END: u32 = 107;

fn long_menu() -> Menu {
    Menu::new(
        (0..40)
            .map(|index| MenuItem::new(&format!("Network {index}"), &format!("join-{index}")))
            .collect(),
    )
}

impl Desktop {
    fn desktop_layer(&mut self, menu: fn() -> Menu) -> Client {
        let mut client = self.open_with_menu(
            Role::Layer(LayerOptions {
                layer: LayerKind::Background,
                anchor: Edges::ALL,
                exclusive_zone: -1,
                keyboard: KeyboardFocus::OnDemand,
                namespace: "desktop".to_owned(),
            }),
            true,
            menu,
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut nudge = 0.0;
        while !client.seen.iter().any(|event| {
            matches!(
                event,
                WindowEvent::Input(Event::PointerEnter(_) | Event::PointerMove(_))
            )
        }) {
            assert!(
                Instant::now() < deadline,
                "the layer never took the pointer"
            );
            nudge = 1.0 - nudge;
            self.server
                .move_pointer_to(Point::from((40.0 + nudge, 40.0)), 0);
            self.settle(&mut [&mut client]);
        }
        client
    }

    fn open_menu_at(&mut self, client: &mut Client, at: (f64, f64), popups: usize) {
        self.server.move_pointer_to(Point::from(at), 0);
        self.server.press_button(BTN_RIGHT, true, 0);
        self.server.press_button(BTN_RIGHT, false, 0);
        self.wait_for(
            &mut [client],
            |_| true,
            |desktop| desktop.server.popup_rects().len() == popups,
        );
        self.settle(&mut [client]);
    }

    fn close_menus(&mut self, client: &mut Client) {
        self.type_keys(&[KEY_ESC]);
        self.type_keys(&[KEY_ESC]);
        self.wait_for(
            &mut [client],
            |_| true,
            |desktop| desktop.server.popup_count() == 0,
        );
    }
}

#[test]
fn a_desktop_menu_opened_in_any_corner_or_edge_stays_on_the_screen_at_1x_and_1_5x() {
    for scale in [1.0, 1.5] {
        let mut desktop = Desktop::sized((480, 360));
        desktop.server.set_output_scale(scale);
        let mut client = desktop.desktop_layer(sample_menu);
        let screen = desktop.server.usable_area();
        let (width, height) = (f64::from(screen.size.w), f64::from(screen.size.h));
        for at in [
            (3.0, 3.0),
            (width / 2.0, 3.0),
            (width - 3.0, 3.0),
            (width - 3.0, height / 2.0),
            (width - 3.0, height - 3.0),
            (width / 2.0, height - 3.0),
            (3.0, height - 3.0),
            (3.0, height / 2.0),
        ] {
            desktop.open_menu_at(&mut client, at, 1);
            let placed = desktop.server.popup_rects()[0];
            assert!(
                screen.contains_rect(placed),
                "at {scale}x a menu at {at:?} lands at {placed:?}"
            );
            desktop.close_menus(&mut client);
        }
    }
}

#[test]
fn a_submenu_without_room_on_the_right_opens_on_the_left_overlapping_its_menu() {
    let mut desktop = Desktop::sized((640, 360));
    let mut client = desktop.desktop_layer(sample_menu);
    let screen = desktop.server.usable_area();
    desktop.open_menu_at(&mut client, (10.0, 10.0), 1);
    let width = desktop.server.popup_rects()[0].size.w;
    desktop.close_menus(&mut client);

    let at = f64::from(screen.size.w - width - 4);
    desktop.open_menu_at(&mut client, (at, 10.0), 1);
    desktop.type_keys(&[KEY_DOWN]);
    desktop.type_keys(&[KEY_DOWN]);
    desktop.type_keys(&[KEY_RIGHT]);
    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| desktop.server.popup_rects().len() == 2,
    );
    let mut rects = desktop.server.popup_rects();
    rects.sort_by_key(|rect| std::cmp::Reverse(rect.size.h));
    let [menu, submenu] = rects[..] else {
        panic!("{rects:?}");
    };
    assert_eq!(menu.loc.x, at as i32);
    assert!(screen.contains_rect(submenu), "{submenu:?}");
    assert_eq!(submenu.loc.x + submenu.size.w, menu.loc.x + 2);
}

#[test]
fn a_menu_taller_than_the_screen_is_shortened_and_reaches_every_item_by_keyboard_and_wheel() {
    let mut desktop = Desktop::new();
    let mut client = desktop.desktop_layer(long_menu);
    let screen = desktop.server.usable_area();
    desktop.open_menu_at(&mut client, (20.0, 20.0), 1);
    let placed = desktop.server.popup_rects()[0];
    assert_eq!(placed.loc.y, screen.loc.y);
    assert_eq!(placed.size.h, screen.size.h);
    assert!(screen.contains_rect(placed));

    desktop.type_keys(&[KEY_END]);
    desktop.type_keys(&[KEY_ENTER]);
    desktop.wait_for(
        &mut [&mut client],
        |events| !menu_events(events).is_empty(),
        |desktop| desktop.server.popup_count() == 0,
    );
    assert_eq!(
        menu_events(&client.seen),
        [WindowEvent::MenuAction("join-39".to_owned())]
    );
    client.seen.clear();

    desktop.open_menu_at(&mut client, (20.0, 20.0), 1);
    desktop.server.scroll(
        AxisFrame::new(0)
            .source(AxisSource::Wheel)
            .value(Axis::Vertical, 3000.0),
    );
    desktop.settle(&mut [&mut client]);
    let middle = f64::from(placed.loc.y + placed.size.h / 2);
    desktop
        .server
        .move_pointer_to(Point::from((40.0, middle)), 0);
    desktop.settle(&mut [&mut client]);
    desktop.click(BTN_LEFT);
    desktop.wait_for(
        &mut [&mut client],
        |events| !menu_events(events).is_empty(),
        |_| true,
    );
    let [WindowEvent::MenuAction(action)] = &menu_events(&client.seen)[..] else {
        panic!("{:?}", client.seen);
    };
    let index: usize = action
        .trim_start_matches("join-")
        .parse()
        .expect("a network");
    assert!(index >= 30, "the wheel left {action} in the middle");
}

#[test]
fn holding_the_zoom_control_opens_the_tiling_menu_instead_of_zooming() {
    let mut desktop = Desktop::new();
    let mut client = pointed_window(&mut desktop, "zoom hold");
    desktop.point_into_window(66, 24);
    desktop.server.press_button(BTN_LEFT, true, 0);
    desktop.wait_for(
        &mut [&mut client],
        |_| true,
        |desktop| desktop.server.popup_count() == 1,
    );
    desktop.server.press_button(BTN_LEFT, false, 0);
    desktop.settle(&mut [&mut client]);

    assert_eq!(desktop.server.popup_count(), 1);
    assert!(
        !client
            .seen
            .iter()
            .any(|event| matches!(event, WindowEvent::ToplevelChanged(state) if state.maximized))
    );
}
