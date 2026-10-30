use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use smithay::utils::Size;
use unbound_compositor::{FrameClock, FrameScheduler, Headless, REFRESH, Renderer, SessionRole};
use unbound_graphics::{Color, Scene};
use unbound_toolkit_platform::{FramePath, HEADER_HEIGHT, Role, SHEET_INSET, Window};

const OUTPUT: (i32, i32) = (320, 240);
const WINDOW: (u32, u32) = (200, 120);
const RED: Color = Color::hex(0xD03030);
const BLUE: Color = Color::hex(0x2F6FEB);
const FRAMED: (i32, i32) = (
    (WINDOW.0 + 2 * SHEET_INSET) as i32,
    (WINDOW.1 + HEADER_HEIGHT + SHEET_INSET) as i32,
);

fn filled(color: Color) -> Scene {
    Scene {
        width: WINDOW.0,
        height: WINDOW.1,
        background: color,
        items: Vec::new(),
    }
}

#[test]
fn the_first_frame_is_drawn_on_the_cpu_and_later_frames_on_the_gpu() {
    let mut server = Headless::new(Size::from(OUTPUT)).expect("headless compositor");
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");

    let (presented, on_screen) = mpsc::channel();
    let (checked, check_done) = mpsc::channel::<()>();
    let client = thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel("toolkit test", "unbound-toolkit-test"),
            WINDOW,
        )
        .expect("window");
        let mut paths = Vec::new();
        for color in [RED, BLUE] {
            paths.push(window.draw(&filled(color)).expect("frame"));
            window.wait_for_frame().expect("frame callback");
            presented.send(color).expect("report frame");
            check_done.recv().expect("compositor checked the frame");
            window.wait_for_gpu();
        }
        paths
    });

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let start = Instant::now();
    let mut seen = Vec::new();
    while !client.is_finished() {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "the client stalled"
        );
        server.dispatch().expect("server dispatch");
        scheduler
            .poll(|| start.elapsed(), &mut server, &mut renderer)
            .expect("compositor frame");
        if let Ok(color) = on_screen.try_recv() {
            while server.is_animating() {
                server.dispatch().expect("server dispatch");
                scheduler
                    .poll(|| start.elapsed(), &mut server, &mut renderer)
                    .expect("compositor frame");
            }
            let window = &server.windows()[0];
            let size = window.geometry().expect("mapped window").size;
            assert_eq!((size.w, size.h), FRAMED);
            let snapshot = renderer.snapshot(&server).expect("read back");
            let location = window.location();
            let (x, y) = (location.x + size.w / 2, location.y + size.h / 2);
            let offset = ((y * snapshot.size.w + x) * 4) as usize;
            seen.push(snapshot.rgba[offset..offset + 3].to_vec());
            assert_eq!(
                &snapshot.rgba[offset..offset + 3],
                &[color.r, color.g, color.b],
                "the compositor shows frame {}",
                seen.len()
            );
            checked.send(()).expect("release the client");
        }
        thread::sleep(Duration::from_micros(200));
    }
    let paths = client.join().expect("client thread");
    assert_eq!(paths, [FramePath::Cpu, FramePath::Gpu]);
    assert_eq!(seen.len(), 2);
}

#[test]
fn a_lock_window_covers_the_output_until_it_unlocks() {
    let mut server = Headless::new(Size::from(OUTPUT)).expect("headless compositor");
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server
        .insert_session_client(server_end, SessionRole::Shell)
        .expect("insert shell client");

    let (shown, on_screen) = mpsc::channel();
    let (checked, check_done) = mpsc::channel::<()>();
    let client = thread::spawn(move || {
        let mut window = Window::open_on(client_end, Role::Lock, (0, 0)).expect("lock window");
        let (width, height) = window.size();
        shown.send((width, height)).expect("report size");
        let scene = Scene {
            width,
            height,
            background: BLUE,
            items: Vec::new(),
        };
        window.draw(&scene).expect("frame");
        window.wait_for_frame().expect("frame callback");
        shown.send((width, height)).expect("report frame");
        check_done.recv().expect("compositor checked the frame");
        window.unlock().expect("unlock");
    });

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let start = Instant::now();
    let centre = |renderer: &mut Renderer, server: &Headless| {
        let snapshot = renderer.snapshot(server).expect("read back");
        let offset = (((OUTPUT.1 / 3) * snapshot.size.w + OUTPUT.0 / 3) * 4) as usize;
        snapshot.rgba[offset..offset + 3].to_vec()
    };
    let mut reports = 0;
    let mut unlocked = false;
    while !unlocked {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "the lock client stalled"
        );
        server.dispatch().expect("server dispatch");
        scheduler
            .poll(|| start.elapsed(), &mut server, &mut renderer)
            .expect("compositor frame");
        if let Ok(size) = on_screen.try_recv() {
            assert_eq!(size, (OUTPUT.0 as u32, OUTPUT.1 as u32));
            reports += 1;
            if reports == 2 {
                assert_eq!(centre(&mut renderer, &server), [BLUE.r, BLUE.g, BLUE.b]);
                checked.send(()).expect("release the client");
            }
        }
        unlocked = client.is_finished()
            && reports == 2
            && centre(&mut renderer, &server) != [BLUE.r, BLUE.g, BLUE.b];
        thread::sleep(Duration::from_micros(200));
    }
    client.join().expect("client thread");
}

#[test]
fn on_a_2x_output_a_window_keeps_its_size_in_points_and_fills_twice_the_pixels() {
    let mut server = Headless::new(Size::from((640, 480))).expect("headless compositor");
    server.set_output_scale(2.0);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");

    let (drawn, on_screen) = mpsc::channel();
    let (checked, check_done) = mpsc::channel::<()>();
    let client = thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel("toolkit test", "unbound-toolkit-test"),
            WINDOW,
        )
        .expect("window");
        let scale = window.scale();
        window.draw(&filled(RED)).expect("frame");
        window.wait_for_frame().expect("frame callback");
        drawn.send(scale).expect("report frame");
        check_done.recv().expect("compositor checked the frame");
    });

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let start = Instant::now();
    let mut verified = false;
    while !client.is_finished() {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "the client stalled"
        );
        server.dispatch().expect("server dispatch");
        scheduler
            .poll(|| start.elapsed(), &mut server, &mut renderer)
            .expect("compositor frame");
        if let Ok(scale) = on_screen.try_recv() {
            assert_eq!(scale, 2.0, "the window was told the output scale");
            while server.is_animating() {
                server.dispatch().expect("server dispatch");
                scheduler
                    .poll(|| start.elapsed(), &mut server, &mut renderer)
                    .expect("compositor frame");
            }
            let window = &server.windows()[0];
            let size = window.geometry().expect("mapped window").size;
            assert_eq!((size.w, size.h), FRAMED);
            let snapshot = renderer.snapshot(&server).expect("read back");
            let location = window.location();
            let at = |x: i32, y: i32| {
                let offset = ((y * snapshot.size.w + x) * 4) as usize;
                snapshot.rgba[offset..offset + 3].to_vec()
            };
            let inset = SHEET_INSET as i32;
            let left = (location.x + inset) * 2;
            let middle = (location.y + HEADER_HEIGHT as i32 + WINDOW.1 as i32 / 2) * 2;
            let right = left + WINDOW.0 as i32 * 2 - 1;
            assert_eq!(at(left, middle), [RED.r, RED.g, RED.b]);
            assert_eq!(at(right, middle), [RED.r, RED.g, RED.b]);
            assert_ne!(at(right + 2, middle), [RED.r, RED.g, RED.b]);
            assert_ne!(at(left + 40, (location.y + 2) * 2), [RED.r, RED.g, RED.b]);
            checked.send(()).expect("release the client");
            verified = true;
        }
        thread::sleep(Duration::from_micros(200));
    }
    assert!(verified, "the frame was drawn and verified");
    client.join().expect("client thread");
}
