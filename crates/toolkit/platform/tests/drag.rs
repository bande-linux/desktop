#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use bande_compositor::OutputConfig;
use bande_graphics::{Color, Event, PointerButton, Scene};
use bande_test_wayland::{GREEN, ShmImage, TestClient};
use bande_toolkit_platform::{App, DragAction, DragEvent, DragIcon, Transfer, Window, WindowEvent};
use smithay::utils::{Logical, Point, Size};
use wayland_client::protocol::wl_shm::Format;

#[path = "support/desktop.rs"]
mod desktop;
#[path = "support/note.rs"]
mod note;

use desktop::{Client, Desktop};
use note::{BTN_LEFT, at, note, text_of};

const OUTPUT: (i32, i32) = (640, 400);
const SIDE: &str = "HEADLESS-2";
const FOLDER_ROW: f32 = 30.0;
const COPY_ONLY_ROW: f32 = 100.0;
const REPORT: &str = "file:///home/user/Report%20one.pdf";
const KEY_ESC: u32 = 1;
const KEY_LEFTALT: u32 = 56;
const KEY_LEFTMETA: u32 = 125;

struct Shelf {
    events: Sender<WindowEvent>,
    commands: UnixStream,
    finished: bool,
    source: bool,
}

impl App for Shelf {
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
        match &event {
            WindowEvent::Input(Event::PointerDown(pointer))
                if self.source && pointer.button == PointerButton::Primary =>
            {
                let icon = DragIcon {
                    scene: Scene {
                        width: 16,
                        height: 16,
                        background: Color::hex(0xE0A030),
                        items: Vec::new(),
                    },
                    hotspot: bande_graphics::Point::new(8.0, 8.0),
                };
                window
                    .start_drag(
                        Transfer::uris(vec![REPORT.to_owned()]),
                        &[DragAction::Copy, DragAction::Move],
                        Some(icon),
                    )
                    .expect("drag");
            }
            WindowEvent::Drag(DragEvent::Entered { position, .. } | DragEvent::Moved(position)) => {
                match position.y {
                    y if y < FOLDER_ROW => window.accept_drag(Some(DragAction::Move)),
                    y if y < COPY_ONLY_ROW => window.accept_drag(Some(DragAction::Copy)),
                    _ => window.accept_drag_only(Some(DragAction::Copy)),
                }
            }
            _ => {}
        }
        let _ = self.events.send(event);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.commands.as_fd()]
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) {
        let mut command = [0u8; 1];
        if self.commands.read(&mut command).unwrap_or(0) == 0 {
            self.finished = true;
        }
    }

    fn finished(&self) -> bool {
        self.finished
    }
}

fn drags(events: &[WindowEvent]) -> Vec<DragEvent> {
    events
        .iter()
        .filter_map(|event| match event {
            WindowEvent::Drag(drag) => Some(drag.clone()),
            _ => None,
        })
        .collect()
}

fn finished(events: &[WindowEvent]) -> Option<Option<DragAction>> {
    drags(events).into_iter().find_map(|drag| match drag {
        DragEvent::Finished(action) => Some(action),
        _ => None,
    })
}

type Landed = (f32, f32, Transfer, Option<DragAction>);

fn dropped(events: &[WindowEvent]) -> Option<Landed> {
    drags(events).into_iter().find_map(|drag| match drag {
        DragEvent::Dropped {
            position,
            transfer,
            action,
        } => Some((position.x, position.y, transfer, action)),
        _ => None,
    })
}

impl Desktop {
    fn open_shelf(&mut self, name: &str, source: bool) -> Client {
        self.open_named(name, move |events, commands| Shelf {
            events,
            commands,
            finished: false,
            source,
        })
    }

    fn side_by_side(&mut self) -> (Client, Client) {
        let mut source = self.open_shelf("source", true);
        self.slide(&mut source, "source", -200);
        let mut target = self.open_shelf("target", false);
        self.slide(&mut target, "target", 200);
        (source, target)
    }

    fn drag_from(&mut self, source: &mut Client, target: &mut Client, to: Point<f64, Logical>) {
        let from = self.content_of("source", 40, 60);
        self.server.move_pointer_to(from, 0);
        self.server.press_button(BTN_LEFT, true, 0);
        self.settle(&mut [&mut *source, &mut *target]);
        self.server.move_pointer_to(to, 0);
        self.wait_for(
            &mut [target],
            |events| {
                drags(events).iter().any(
                    |drag| matches!(drag, DragEvent::Entered { uris, .. } if uris == &[REPORT]),
                )
            },
            |_| true,
        );
        self.settle(&mut [&mut *source, &mut *target]);
    }

    fn finish_drop(&mut self, source: &mut Client, target: &mut Client) {
        self.server.press_button(BTN_LEFT, false, 0);
        self.wait_for(
            &mut [&mut *source],
            |events| finished(events).is_some(),
            |_| true,
        );
        self.settle(&mut [&mut *source, &mut *target]);
    }
}

fn drop_between_windows(
    modifier: Option<u32>,
    y: i32,
) -> (Option<Option<DragAction>>, Option<Landed>) {
    let mut desktop = Desktop::sized(OUTPUT);
    let (mut source, mut target) = desktop.side_by_side();
    let to = desktop.content_of("target", 50, y);
    desktop.drag_from(&mut source, &mut target, to);
    if let Some(key) = modifier {
        desktop.server.key(key, true, 0);
        desktop.settle(&mut [&mut source, &mut target]);
    }
    desktop.finish_drop(&mut source, &mut target);
    if let Some(key) = modifier {
        desktop.server.key(key, false, 0);
    }
    desktop.next_click_reaches(&mut target);
    (finished(&source.seen), dropped(&target.seen))
}

#[test]
fn a_file_dropped_on_a_folder_row_of_another_app_moves_there() {
    let (finished, dropped) = drop_between_windows(None, 10);

    let (x, y, transfer, action) = dropped.expect("a drop");
    assert_eq!((x, y), (50.0, 10.0));
    assert_eq!(transfer.uris, [REPORT]);
    assert_eq!(action, Some(DragAction::Move));
    assert_eq!(finished, Some(Some(DragAction::Move)));
}

#[test]
fn holding_option_while_still_over_the_folder_makes_the_drop_a_copy() {
    let (finished, dropped) = drop_between_windows(Some(KEY_LEFTALT), 10);

    assert_eq!(dropped.expect("a drop").3, Some(DragAction::Copy));
    assert_eq!(finished, Some(Some(DragAction::Copy)));
}

#[test]
fn holding_super_moves_where_the_target_would_copy() {
    let plain = drop_between_windows(None, 80);
    let with_super = drop_between_windows(Some(KEY_LEFTMETA), 80);

    assert_eq!(plain.0, Some(Some(DragAction::Copy)));
    assert_eq!(with_super.1.expect("a drop").3, Some(DragAction::Move));
    assert_eq!(with_super.0, Some(Some(DragAction::Move)));
}

#[test]
fn a_target_that_only_copies_keeps_the_drop_a_copy_with_super_held() {
    let (finished, dropped) = drop_between_windows(Some(KEY_LEFTMETA), 110);

    assert_eq!(dropped.expect("a drop").3, Some(DragAction::Copy));
    assert_eq!(finished, Some(Some(DragAction::Copy)));
}

#[test]
fn escape_cancels_a_drag_between_apps_and_the_source_hears_it_ended() {
    let mut desktop = Desktop::sized(OUTPUT);
    let (mut source, mut target) = desktop.side_by_side();
    let to = desktop.content_of("target", 50, 10);
    desktop.drag_from(&mut source, &mut target, to);

    desktop.server.key(KEY_ESC, true, 0);
    desktop.server.key(KEY_ESC, false, 0);
    desktop.wait_for(
        &mut [&mut source],
        |events| finished(events).is_some(),
        |_| true,
    );
    desktop.server.press_button(BTN_LEFT, false, 0);
    desktop.settle(&mut [&mut source, &mut target]);

    assert_eq!(finished(&source.seen), Some(None));
    assert!(drags(&target.seen).contains(&DragEvent::Left));
    assert_eq!(dropped(&target.seen), None);
    desktop.next_click_reaches(&mut target);
}

#[test]
fn a_file_dragged_onto_an_app_on_the_other_display_arrives() {
    let mut desktop = Desktop::sized(OUTPUT);
    desktop
        .server
        .add_output(OutputConfig::virtual_display(SIDE, Size::from(OUTPUT)));
    let mut source = desktop.open_shelf("source", true);
    desktop
        .server
        .move_pointer_to(at(OUTPUT.0 + OUTPUT.0 / 2, OUTPUT.1 / 2), 0);
    let mut target = desktop.open_shelf("target", false);
    let on_side = desktop
        .server
        .windows()
        .iter()
        .find(|window| window.app_id() == "test.target")
        .map(|window| desktop.server.output_of(window));
    assert_eq!(on_side.as_deref(), Some(SIDE));

    let to = desktop.content_of("target", 50, 10);
    desktop.drag_from(&mut source, &mut target, to);
    desktop.finish_drop(&mut source, &mut target);

    assert_eq!(dropped(&target.seen).expect("a drop").2.uris, [REPORT]);
    assert_eq!(finished(&source.seen), Some(Some(DragAction::Move)));
    desktop.next_click_reaches(&mut target);
}

#[test]
fn a_target_that_never_finishes_does_not_keep_the_source_dragging() {
    let mut desktop = Desktop::sized(OUTPUT);
    let mut source = desktop.open_shelf("source", true);
    desktop.slide(&mut source, "source", -200);
    let mut target = TestClient::connect(&mut desktop.server);
    target.use_seat(&mut desktop.server);
    target.join_seat(&mut desktop.server);
    target.accept_drops(true);
    target.hold_drops();
    let pixels = GREEN.repeat(160 * 120);
    target.show_window(
        &mut desktop.server,
        &ShmImage {
            width: 160,
            height: 120,
            stride: 160 * 4,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
    );
    let window = desktop
        .server
        .windows()
        .iter()
        .find(|window| window.app_id() != "test.source")
        .map(|window| window.location())
        .expect("the target window");

    let from = desktop.content_of("source", 40, 60);
    desktop.server.move_pointer_to(from, 0);
    desktop.server.press_button(BTN_LEFT, true, 0);
    desktop.settle(&mut [&mut source]);
    desktop
        .server
        .move_pointer_to(at(window.x + 80, window.y + 60), 0);
    target.exchange(&mut desktop.server);
    desktop.settle(&mut [&mut source]);
    target.exchange(&mut desktop.server);
    desktop.server.press_button(BTN_LEFT, false, 0);
    target.exchange(&mut desktop.server);
    let dropped_at = Instant::now();
    desktop.wait_for(
        &mut [&mut source],
        |events| finished(events).is_some(),
        |_| true,
    );

    assert_eq!(target.drops(), 1);
    assert!(dropped_at.elapsed() >= Duration::from_millis(1500));
    assert_eq!(finished(&source.seen), Some(None));
    assert!(!desktop.server.pointer_grabbed());
}

impl Desktop {
    fn notes_side_by_side(&mut self, source: &str, target: &str) -> (Client, Client) {
        let mut left = self.open_named("source", note(source, 0..source.len()));
        self.slide(&mut left, "source", -200);
        let mut right = self.open_named("target", note(target, 0..0));
        self.slide(&mut right, "target", 200);
        (left, right)
    }

    fn drag_text(
        &mut self,
        clients: &mut [&mut Client],
        from: Point<f64, Logical>,
        to: Point<f64, Logical>,
    ) {
        self.server.move_pointer_to(from, 0);
        self.server.press_button(BTN_LEFT, true, 0);
        self.settle(clients);
        self.server
            .move_pointer_to(at(from.x as i32 + 12, from.y as i32), 0);
        self.wait_for(clients, |_| true, |desktop| desktop.server.is_dragging());
        let hovered = |events: &[WindowEvent]| {
            drags(events)
                .iter()
                .filter(|drag| matches!(drag, DragEvent::Entered { .. } | DragEvent::Moved(_)))
                .count()
        };
        let before = clients.last().map_or(0, |client| hovered(&client.seen));
        self.server.move_pointer_to(to, 0);
        let deadline = Instant::now() + Duration::from_secs(20);
        while clients.last().map_or(0, |client| hovered(&client.seen)) <= before {
            assert!(
                Instant::now() < deadline,
                "the drag never reached its target"
            );
            self.settle(clients);
        }
        self.settle(clients);
    }
}

#[test]
fn text_dragged_into_another_apps_field_is_copied_at_the_drop_point() {
    let mut desktop = Desktop::sized(OUTPUT);
    let (mut source, mut target) = desktop.notes_side_by_side("brave", "Hello world");
    let from = desktop.content_of("source", 10, 11);
    let to = desktop.content_of("target", 180, 11);

    desktop.drag_text(&mut [&mut source, &mut target], from, to);
    desktop.server.press_button(BTN_LEFT, false, 0);
    desktop.wait_for(
        &mut [&mut target],
        |events| text_of(events).is_some(),
        |_| true,
    );
    desktop.settle(&mut [&mut source, &mut target]);

    assert_eq!(text_of(&target.seen), Some("Hello worldbrave"));
    assert_eq!(text_of(&source.seen), None);
    desktop.next_click_reaches(&mut target);
}

#[test]
fn holding_super_moves_dragged_text_out_of_the_other_field() {
    let mut desktop = Desktop::sized(OUTPUT);
    let (mut source, mut target) = desktop.notes_side_by_side("brave", "Hello world");
    let from = desktop.content_of("source", 10, 11);
    let to = desktop.content_of("target", 180, 11);

    desktop.drag_text(&mut [&mut source, &mut target], from, to);
    desktop.server.key(KEY_LEFTMETA, true, 0);
    desktop.settle(&mut [&mut source, &mut target]);
    desktop.server.press_button(BTN_LEFT, false, 0);
    desktop.wait_for(
        &mut [&mut source],
        |events| text_of(events) == Some(""),
        |_| true,
    );
    desktop.server.key(KEY_LEFTMETA, false, 0);
    desktop.settle(&mut [&mut source, &mut target]);

    assert_eq!(text_of(&target.seen), Some("Hello worldbrave"));
    desktop.next_click_reaches(&mut target);
}

#[test]
fn a_selection_dragged_within_its_field_moves_to_the_drop_point() {
    let mut desktop = Desktop::sized(OUTPUT);
    let mut field = desktop.open_named("source", note("ab", 0..1));
    let from = desktop.content_of("source", 9, 11);
    let to = desktop.content_of("source", 150, 11);

    desktop.drag_text(&mut [&mut field], from, to);
    desktop.server.press_button(BTN_LEFT, false, 0);
    desktop.wait_for(
        &mut [&mut field],
        |events| text_of(events) == Some("ba"),
        |_| true,
    );
    desktop.settle(&mut [&mut field]);

    assert_eq!(text_of(&field.seen), Some("ba"));
    assert!(!desktop.server.pointer_grabbed());
}

#[test]
fn escape_while_dragging_text_keeps_both_fields_unchanged() {
    let mut desktop = Desktop::sized(OUTPUT);
    let (mut source, mut target) = desktop.notes_side_by_side("brave", "Hello world");
    let from = desktop.content_of("source", 10, 11);
    let to = desktop.content_of("target", 180, 11);

    desktop.drag_text(&mut [&mut source, &mut target], from, to);
    desktop.server.key(KEY_ESC, true, 0);
    desktop.server.key(KEY_ESC, false, 0);
    desktop.settle(&mut [&mut source, &mut target]);
    desktop.server.press_button(BTN_LEFT, false, 0);
    desktop.settle(&mut [&mut source, &mut target]);

    assert_eq!(text_of(&source.seen), None);
    assert_eq!(text_of(&target.seen), None);
    desktop.next_click_reaches(&mut target);
}

#[test]
fn text_dragged_from_another_toolkit_lands_in_a_field() {
    let mut desktop = Desktop::sized(OUTPUT);
    let mut field = desktop.open_named("target", note("Hello ", 0..0));
    desktop.slide(&mut field, "target", 200);
    let mut source = TestClient::connect(&mut desktop.server);
    source.use_seat(&mut desktop.server);
    source.join_seat(&mut desktop.server);
    let pixels = GREEN.repeat(160 * 120);
    source.show_window(
        &mut desktop.server,
        &ShmImage {
            width: 160,
            height: 120,
            stride: 160 * 4,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
    );
    let window = desktop
        .server
        .windows()
        .iter()
        .find(|window| window.app_id() != "test.target")
        .map(|window| window.location())
        .expect("the source window");
    desktop
        .server
        .move_pointer_to(at(window.x + 40, window.y + 40), 0);
    desktop.server.press_button(BTN_LEFT, true, 0);
    source.exchange(&mut desktop.server);
    let icon = GREEN.repeat(4 * 4);
    source.start_drag(
        &mut desktop.server,
        &ShmImage {
            width: 4,
            height: 4,
            stride: 16,
            format: Format::Argb8888,
            bytes: &icon,
        },
        (0, 0),
    );

    let to = desktop.content_of("target", 180, 11);
    desktop.server.move_pointer_to(to, 0);
    desktop.settle(&mut [&mut field]);
    source.exchange(&mut desktop.server);
    desktop.settle(&mut [&mut field]);
    desktop.server.press_button(BTN_LEFT, false, 0);
    let deadline = Instant::now() + Duration::from_secs(20);
    while text_of(&field.seen).is_none() && Instant::now() < deadline {
        source.exchange(&mut desktop.server);
        desktop.settle(&mut [&mut field]);
    }

    assert_eq!(text_of(&field.seen), Some("Hello dragged"));
    assert!(!desktop.server.pointer_grabbed());
}
