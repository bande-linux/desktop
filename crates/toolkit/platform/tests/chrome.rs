#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use smithay::utils::{Point, Size};
use unbound_compositor::{FrameClock, FrameScheduler, Headless, REFRESH, Renderer};
use unbound_graphics::{Color, Scene};
use unbound_toolkit_platform::{Role, Window};

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

#[test]
fn the_window_frame_matches_its_golden_with_the_controls_hovered() {
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
        let scene = Scene {
            width: 300,
            height: 160,
            background: Color::hex(0xF4F5F7),
            items: Vec::new(),
        };
        window.wait_for_gpu();
        window.draw(&scene).expect("first frame");
        drawn.send(()).expect("report the frame");
        while check_done.try_recv().is_err() {
            window.dispatch().expect("dispatch");
            window.take_events();
            window.wait_for_frame().expect("frame callback");
            window.draw(&scene).expect("frame");
        }
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
    let hovered = Instant::now() + Duration::from_millis(600);
    let mut pointed = false;
    while Instant::now() < hovered {
        pump(&mut server, &mut scheduler, &mut renderer);
        if !pointed && server.windows().first().is_some_and(|w| w.size().is_some()) {
            let at = server.windows()[0].location();
            server.move_pointer_to(Point::from((f64::from(at.x + 40), f64::from(at.y + 14))), 0);
            pointed = true;
        }
    }
    let snapshot = renderer.snapshot(&server).expect("read back");
    checked.send(()).expect("release the client");
    while !client.is_finished() {
        pump(&mut server, &mut scheduler, &mut renderer);
    }
    client.join().expect("client thread");
    check_golden(
        "window-frame-hovered",
        (snapshot.size.w as u32, snapshot.size.h as u32),
        snapshot.rgba,
    );
}
