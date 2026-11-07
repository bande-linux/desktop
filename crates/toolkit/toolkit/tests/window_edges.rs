#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::thread;
use std::time::{Duration, Instant};

use bande_test_wayland::{Compositor, Frames, headless};
use bande_toolkit::{App, Color, FrameRegions, Header, Rect, Role, Scene, Window, WindowEvent};

const BTN_LEFT: u32 = 0x110;
const TAB: Rect = Rect::new(90.0, 8.0, 120.0, 32.0);
const DRAG: i32 = 16;

struct TabbedWindow;

impl App for TabbedWindow {
    fn scene(&mut self, window: &mut Window) -> Scene {
        window.set_frame_regions(FrameRegions::default());
        window.set_header(Header {
            title_x: 600.0,
            items: Vec::new(),
            tabs: vec![TAB],
        });
        let (width, height) = window.size();
        Scene {
            width,
            height,
            background: Color::TRANSPARENT,
            items: Vec::new(),
        }
    }

    fn event(&mut self, _: &mut Window, _: WindowEvent) {}
}

#[derive(Debug, PartialEq)]
enum Drag {
    Moved,
    Resized { moved: (i32, i32), grew: (i32, i32) },
}

struct Desk {
    server: Compositor,
    frames: Frames,
    time: u32,
}

impl Desk {
    fn settle(&mut self, millis: u64) {
        let until = Instant::now() + Duration::from_millis(millis);
        self.frames
            .run_until(&mut self.server, || Instant::now() >= until);
    }

    fn window(&self) -> (i32, i32, i32, i32) {
        let rect = self.server.windows()[0].geometry().expect("mapped window");
        (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h)
    }

    fn press_at(&mut self, x: f64, y: f64) -> (f64, f64) {
        let (left, top, ..) = self.window();
        let at = (f64::from(left) + x, f64::from(top) + y);
        self.time += 5_000;
        self.server.move_pointer_to(at.into(), self.time);
        self.settle(150);
        self.server.press_button(BTN_LEFT, true, self.time + 1);
        self.settle(350);
        at
    }

    fn pull(&mut self, at: (f64, f64), by: i32) {
        let from = self.window();
        let to = (at.0 + f64::from(by), at.1 + f64::from(by));
        self.server.move_pointer_to(to.into(), self.time + 2);
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.window() == from && Instant::now() < deadline {
            self.settle(20);
        }
        self.settle(100);
    }

    fn drag_from(&mut self, x: f64, y: f64) -> Drag {
        let before = self.window();
        let at = self.press_at(x, y);
        self.pull(at, DRAG);
        let during = self.window();
        self.pull(at, 0);
        self.server.press_button(BTN_LEFT, false, self.time + 3);
        self.settle(150);
        let moved = (during.0 - before.0, during.1 - before.1);
        let grew = (during.2 - before.2, during.3 - before.3);
        if grew == (0, 0) && moved == (DRAG, DRAG) {
            Drag::Moved
        } else {
            Drag::Resized { moved, grew }
        }
    }
}

#[test]
fn the_free_space_inside_a_window_edge_moves_it_and_only_outside_the_outline_resizes() {
    let mut server = headless(1400, 900);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    thread::spawn(move || {
        let mut window = Window::open_on(client_end, Role::toplevel("Tabs", "edges"), (700, 450))
            .expect("window");
        let _ = bande_toolkit::run(&mut window, &mut TabbedWindow);
    });
    let mut desk = Desk {
        server,
        frames: Frames::start(),
        time: 0,
    };
    while desk
        .server
        .windows()
        .first()
        .and_then(|w| w.geometry())
        .is_none()
    {
        desk.settle(20);
    }
    desk.settle(500);
    let (_, _, width, height) = desk.window();
    let (width, height) = (f64::from(width), f64::from(height));
    let top = Drag::Resized {
        moved: (0, DRAG),
        grew: (0, -DRAG),
    };
    let top_left = Drag::Resized {
        moved: (DRAG, DRAG),
        grew: (-DRAG, -DRAG),
    };
    let bottom_right = Drag::Resized {
        moved: (0, 0),
        grew: (DRAG, DRAG),
    };

    assert_eq!(desk.drag_from(width * 0.8, 0.5), Drag::Moved);
    assert_eq!(desk.drag_from(150.0, 0.5), Drag::Moved, "above a tab");
    assert_eq!(desk.drag_from(width * 0.8, -0.5), top);
    assert_eq!(desk.drag_from(0.5, 0.5), top_left, "in the rounded cut-out");
    assert_eq!(desk.drag_from(-5.0, -5.0), top_left);
    assert_eq!(desk.drag_from(-14.0, -14.0), top_left, "out diagonally");
    assert_eq!(desk.drag_from(30.0, -1.0), top_left, "along the top");
    assert_eq!(desk.drag_from(-1.0, 30.0), top_left, "along the side");
    assert_eq!(desk.drag_from(34.0, -1.0), top, "past the corner");
    assert_eq!(desk.drag_from(width + 2.0, height + 2.0), bottom_right);

    let room = 900 - desk.window().1 - desk.window().3;
    let at = desk.press_at(width * 0.8, 20.0);
    desk.server
        .move_pointer_to((at.0, at.1 + f64::from(room)).into(), desk.time + 2);
    desk.settle(400);
    desk.server.press_button(BTN_LEFT, false, desk.time + 3);
    desk.settle(400);
    assert_eq!(
        desk.window().1 + desk.window().3,
        900,
        "resting on the screen edge"
    );
    let at = desk.press_at(width * 0.5, height - 1.0);
    desk.pull(at, -DRAG);
    assert_eq!(
        desk.window().3,
        height as i32 - DRAG,
        "no room outside: 3 pt inside"
    );
    desk.server.press_button(BTN_LEFT, false, desk.time + 3);
}
