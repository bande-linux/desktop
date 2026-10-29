#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use smithay::utils::{Point, Size};
use unbound_compositor::{FrameClock, FrameScheduler, Headless, REFRESH, Renderer};
use unbound_graphics::{Color, Scene};
use unbound_toolkit_platform::{
    App, MenuPlacement, Role, TITLE_BAR_HEIGHT, Window, WindowEvent, run,
};
use unbound_widgets::{Menu, MenuItem};

const MIN_SSIM: f64 = 0.99;
const SCALE: f64 = 2.0;
const OUTPUT: (i32, i32) = (420, 300);

fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    let actual = image::RgbaImage::from_raw(size.0, size.1, rgba).expect("frame");
    let actual = image::DynamicImage::ImageRgba8(actual).to_rgb8();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
    if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().expect("golden directory")).expect("directory");
        actual.save(&path).expect("write golden");
    }
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    actual.save(&target).expect("save the frame");
    let expected = image::open(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .to_rgb8();
    let similarity = image_compare::rgb_hybrid_compare(&expected, &actual).expect("same size");
    assert!(
        similarity.score >= MIN_SSIM,
        "{name} differs from its golden: {:.4} (frame in {})",
        similarity.score,
        target.display()
    );
}

struct Still {
    done: mpsc::Receiver<()>,
}

impl App for Still {
    fn scene(&mut self, _window: &mut Window) -> Scene {
        Scene {
            width: 300,
            height: 160,
            background: Color::hex(0xF4F5F7),
            items: Vec::new(),
        }
    }

    fn event(&mut self, _window: &mut Window, _event: WindowEvent) {}

    fn deadline(&self) -> Option<Instant> {
        Some(Instant::now() + Duration::from_millis(20))
    }

    fn finished(&self) -> bool {
        self.done.try_recv().is_ok()
    }
}

fn shot(name: &str, prepare: fn(&mut Window), pointer: (i32, i32)) {
    let output = (
        (f64::from(OUTPUT.0) * SCALE) as i32,
        (f64::from(OUTPUT.1) * SCALE) as i32,
    );
    let mut server = Headless::new(Size::from(output)).expect("headless compositor");
    server.set_output_scale(SCALE);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let (drawn, on_screen) = mpsc::channel();
    let (checked, check_done) = mpsc::channel::<()>();
    let client = thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel("Documents", "unbound-toolkit-test"),
            (300, 160),
        )
        .expect("window");
        let mut still = Still { done: check_done };
        window.wait_for_gpu();
        let scene = still.scene(&mut window);
        window.draw(&scene).expect("first frame");
        prepare(&mut window);
        drawn.send(()).expect("report the frame");
        run(&mut window, &mut still).expect("run loop");
    });

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let start = Instant::now();
    let pump = |server: &mut Headless, scheduler: &mut FrameScheduler, renderer: &mut Renderer| {
        server.dispatch().expect("server dispatch");
        scheduler
            .poll(|| start.elapsed(), server, renderer)
            .expect("compositor frame");
        thread::sleep(Duration::from_millis(2));
    };
    while on_screen.try_recv().is_err() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the client stalled"
        );
        pump(&mut server, &mut scheduler, &mut renderer);
    }
    let at = server.windows()[0].location();
    server.move_pointer_to(
        Point::from((f64::from(at.x + pointer.0), f64::from(at.y + pointer.1))),
        0,
    );
    let settled = Instant::now() + Duration::from_millis(700);
    while Instant::now() < settled {
        pump(&mut server, &mut scheduler, &mut renderer);
    }
    let snapshot = renderer.snapshot(&server).expect("read back");
    checked.send(()).expect("release the client");
    while !client.is_finished() {
        pump(&mut server, &mut scheduler, &mut renderer);
    }
    client.join().expect("client thread");
    check_golden(
        name,
        (snapshot.size.w as u32, snapshot.size.h as u32),
        snapshot.rgba,
    );
}

#[test]
fn the_window_frame_matches_its_golden_with_the_controls_hovered() {
    shot("window-frame-hovered", |_| {}, (40, 14));
}

fn show_menu(window: &mut Window) {
    let menu = Menu::new(vec![
        MenuItem::new("Open", "open").shortcut("Super+O"),
        MenuItem::new("Rename", "rename").enabled(false),
        MenuItem::separator(),
        MenuItem::submenu(
            "Sort By",
            Menu::new(vec![
                MenuItem::new("Name", "sort-name").checked(true),
                MenuItem::new("Date Modified", "sort-date"),
            ]),
        ),
        MenuItem::new("Move to Trash", "trash").shortcut("Super+Backspace"),
    ]);
    window
        .show_menu(
            menu,
            MenuPlacement::AtPoint(unbound_graphics::Point::new(30.0, 10.0)),
        )
        .expect("menu");
}

#[test]
fn a_menu_with_an_open_submenu_matches_its_golden() {
    let sort_by = 10 + 5 + 22 + 22 + 11 + 11;
    shot(
        "menu-submenu",
        show_menu,
        (60, TITLE_BAR_HEIGHT as i32 + sort_by),
    );
}
