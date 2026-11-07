#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::Compositor;
use bande_test_wayland::{Frames, headless};
use bande_toolkit::{App, Color, Rect, Role, Scene, Window, WindowEvent};
use smithay::utils::Point;

const BTN_LEFT: u32 = 0x110;
const ZOOM: (f64, f64) = (66.0, 24.0);
const FREE_TITLE_BAR: (f64, f64) = (300.0, 24.0);
const CONTENT: (f64, f64) = (480.0, 200.0);
const MOVE_AREA: (f64, f64) = (350.0, 70.0);

struct Blank;

impl App for Blank {
    fn scene(&mut self, window: &mut Window) -> Scene {
        window.set_move_areas(vec![Rect::new(300.0, 0.0, 100.0, 40.0)]);
        let (width, height) = window.size();
        Scene {
            width,
            height,
            background: Color::WHITE,
            items: Vec::new(),
        }
    }

    fn event(&mut self, _window: &mut Window, _event: WindowEvent) {}
}

struct Stage {
    server: Compositor,
    frames: Frames,
    time: u32,
}

impl Stage {
    fn wait(&mut self, milliseconds: u64) {
        let until = Instant::now() + Duration::from_millis(milliseconds);
        self.frames
            .run_until(&mut self.server, || Instant::now() >= until);
        self.server.window_menu_tick(Instant::now());
    }

    fn point_at(&mut self, (x, y): (f64, f64)) {
        let origin = self.server.windows()[0].geometry().expect("mapped").loc;
        self.time += 10;
        self.server.move_pointer_to(
            Point::from((f64::from(origin.x) + x, f64::from(origin.y) + y)),
            self.time,
        );
        self.wait(50);
    }

    fn click(&mut self) {
        self.time += 10;
        self.server.press_button(BTN_LEFT, true, self.time);
        self.wait(50);
        self.time += 10;
        self.server.press_button(BTN_LEFT, false, self.time);
        self.wait(200);
    }

    fn double_click(&mut self) {
        self.click();
        for pressed in [true, false] {
            self.time += 10;
            self.server.press_button(BTN_LEFT, pressed, self.time);
        }
        self.wait(500);
    }

    fn width(&self) -> i32 {
        self.server.windows()[0].geometry().expect("mapped").size.w
    }

    fn offered(&self) -> bool {
        !self.server.window_menu().is_empty()
    }
}

#[test]
fn the_zoom_control_offers_placements_only_while_the_pointer_rests_and_still_zooms() {
    let mut server = headless(1200, 800);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel("Title bar", "bande-toolkit-test"),
            (500, 300),
        )
        .expect("window");
        let _ = bande_toolkit::run(&mut window, &mut Blank);
    });
    let mut stage = Stage {
        server,
        frames: Frames::start(),
        time: 0,
    };
    stage.wait(500);
    let floating = stage.width();

    stage.point_at(ZOOM);
    stage.wait(300);
    assert!(!stage.offered(), "a pointer passing by gets no menu");
    stage.wait(800);
    assert!(
        stage.offered(),
        "a resting pointer is offered the placements"
    );

    stage.point_at(CONTENT);
    assert!(stage.offered(), "the menu forgives a short excursion");
    stage.wait(500);
    assert!(!stage.offered(), "the menu leaves with the pointer");

    stage.point_at(ZOOM);
    stage.wait(1000);
    assert!(stage.offered());
    stage.click();
    assert!(!stage.offered(), "a click closes the offer");
    assert!(stage.width() > floating, "and zooms as if it was not there");

    stage.point_at(FREE_TITLE_BAR);
    stage.double_click();
    assert_eq!(
        stage.width(),
        floating,
        "free title bar double-click unzooms"
    );

    stage.point_at(MOVE_AREA);
    stage.double_click();
    assert!(
        stage.width() > floating,
        "an app's move area acts as title bar"
    );
}
