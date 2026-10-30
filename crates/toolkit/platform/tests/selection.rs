#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read;
use std::time::{Duration, Instant};

use smithay::utils::{Logical, Point};
use unbound_test_wayland::clipboard::TEXT;
use unbound_test_wayland::{GREEN, ShmImage, TestClient};
use wayland_client::protocol::wl_shm::Format;

#[path = "support/desktop.rs"]
mod desktop;
#[path = "support/note.rs"]
mod note;

use desktop::{Client, Desktop};
use note::{BTN_LEFT, at, note, text_of};

const OUTPUT: (i32, i32) = (640, 400);
const BTN_MIDDLE: u32 = 0x112;

impl Desktop {
    fn select_across(
        &mut self,
        client: &mut Client,
        from: Point<f64, Logical>,
        to: Point<f64, Logical>,
    ) {
        self.server.move_pointer_to(from, 0);
        self.server.press_button(BTN_LEFT, true, 0);
        self.settle(&mut [&mut *client]);
        self.server.move_pointer_to(to, 0);
        self.settle(&mut [&mut *client]);
        self.server.press_button(BTN_LEFT, false, 0);
        self.settle(&mut [client]);
    }

    fn middle_click(&mut self, at: Point<f64, Logical>) {
        self.server.move_pointer_to(at, 0);
        self.server.press_button(BTN_MIDDLE, true, 0);
        self.server.press_button(BTN_MIDDLE, false, 0);
    }

    fn foreign_window(&mut self) -> (TestClient, Point<f64, Logical>) {
        let mut foreign = TestClient::connect(&mut self.server);
        foreign.join_seat(&mut self.server);
        let pixels = GREEN.repeat(160 * 120);
        foreign.show_window(
            &mut self.server,
            &ShmImage {
                width: 160,
                height: 120,
                stride: 160 * 4,
                format: Format::Xrgb8888,
                bytes: &pixels,
            },
        );
        let window = self
            .server
            .windows()
            .iter()
            .find(|window| !window.app_id().starts_with("test."))
            .map(|window| window.location())
            .expect("the foreign window");
        (foreign, at(window.x + 40, window.y + 40))
    }
}

#[test]
fn text_selected_in_one_app_is_pasted_by_a_middle_click_in_another() {
    let mut desktop = Desktop::sized(OUTPUT);
    let mut source = desktop.open_named("source", note("brave", 0..0));
    desktop.slide(&mut source, "source", -200);
    let mut target = desktop.open_named("target", note("Hello world", 0..0));
    desktop.slide(&mut target, "target", 200);

    let (from, to) = (
        desktop.content_of("source", 2, 11),
        desktop.content_of("source", 180, 11),
    );
    desktop.select_across(&mut source, from, to);
    desktop.middle_click(desktop.content_of("target", 180, 11));
    desktop.wait_for(
        &mut [&mut source, &mut target],
        |events| text_of(events).is_some(),
        |_| true,
    );
    desktop.settle(&mut [&mut source, &mut target]);

    assert_eq!(text_of(&target.seen), Some("Hello worldbrave"));
    assert_eq!(text_of(&source.seen), None);
    desktop.next_click_reaches(&mut target);
}

#[test]
fn another_toolkits_primary_selection_is_pasted_by_a_middle_click() {
    let mut desktop = Desktop::sized(OUTPUT);
    let mut field = desktop.open_named("target", note("Hello ", 0..0));
    desktop.slide(&mut field, "target", 200);
    let (mut foreign, inside) = desktop.foreign_window();
    desktop.server.move_pointer_to(inside, 0);
    desktop.server.press_button(BTN_LEFT, true, 0);
    desktop.server.press_button(BTN_LEFT, false, 0);
    foreign.exchange(&mut desktop.server);
    foreign.copy_primary(TEXT, b"from afar");
    foreign.exchange(&mut desktop.server);

    desktop.middle_click(desktop.content_of("target", 180, 11));
    let deadline = Instant::now() + Duration::from_secs(20);
    while text_of(&field.seen).is_none() && Instant::now() < deadline {
        foreign.exchange(&mut desktop.server);
        desktop.settle(&mut [&mut field]);
    }

    assert_eq!(text_of(&field.seen), Some("Hello from afar"));
    desktop.next_click_reaches(&mut field);
}

#[test]
fn a_selection_in_a_field_is_the_primary_selection_of_other_toolkits() {
    let mut desktop = Desktop::sized(OUTPUT);
    let mut field = desktop.open_named("source", note("brave", 0..0));
    desktop.slide(&mut field, "source", -200);
    let (from, to) = (
        desktop.content_of("source", 2, 11),
        desktop.content_of("source", 180, 11),
    );
    desktop.select_across(&mut field, from, to);
    let (mut foreign, inside) = desktop.foreign_window();
    desktop.server.move_pointer_to(inside, 0);
    desktop.server.press_button(BTN_LEFT, true, 0);
    desktop.server.press_button(BTN_LEFT, false, 0);
    foreign.exchange(&mut desktop.server);
    assert!(foreign.has_keyboard_focus());

    let mut pipe = foreign.request_primary_paste(TEXT);
    foreign.exchange(&mut desktop.server);
    desktop.settle(&mut [&mut field]);
    let mut pasted = String::new();
    pipe.read_to_string(&mut pasted)
        .expect("read the selection");

    assert_eq!(pasted, "brave");
    assert!(!desktop.server.pointer_grabbed());
}
