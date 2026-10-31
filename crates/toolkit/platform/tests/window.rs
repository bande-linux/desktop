use std::os::unix::net::UnixStream;
use std::sync::{Once, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{Compositor, FrameClock, FrameScheduler, REFRESH, Renderer, SessionRole};
use bande_graphics::{Color, Item, Point, Scene, TextAlign, TextStyle, TextSystem};
use bande_test_wayland::{ShmImage, TestClient, quadrants};
use bande_toolkit_platform::{
    FramePath, RENDERER_VARIABLE, Role, SHEET_INSET, WINDOW_HEADER_HEIGHT, Window,
};
use smithay::utils::Size;
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (320, 240);
const WINDOW: (u32, u32) = (200, 120);
const RED: Color = Color::hex(0xD03030);
const BLUE: Color = Color::hex(0x2F6FEB);
const FRAMED: (i32, i32) = (
    (WINDOW.0 + 2 * SHEET_INSET) as i32,
    (WINDOW.1 + WINDOW_HEADER_HEIGHT + SHEET_INSET) as i32,
);

fn draw_on_the_gpu() {
    static FORCED: Once = Once::new();
    FORCED.call_once(|| unsafe { std::env::set_var(RENDERER_VARIABLE, "gpu") });
}

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
    draw_on_the_gpu();
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");

    let (presented, on_screen) = mpsc::channel();
    let (checked, check_done) = mpsc::channel::<()>();
    let client = thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel("toolkit test", "bande-toolkit-test"),
            WINDOW,
        )
        .expect("window");
        let mut paths = Vec::new();
        for color in [RED, BLUE] {
            paths.push(window.draw(filled(color)).expect("frame"));
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
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
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
        window.draw(scene).expect("frame");
        window.wait_for_frame().expect("frame callback");
        shown.send((width, height)).expect("report frame");
        check_done.recv().expect("compositor checked the frame");
        window.unlock().expect("unlock");
    });

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let start = Instant::now();
    let centre = |renderer: &mut Renderer, server: &Compositor| {
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
    let mut server = Compositor::new(Size::from((640, 480))).expect("headless compositor");
    server.set_output_scale(2.0);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");

    let (drawn, on_screen) = mpsc::channel();
    let (checked, check_done) = mpsc::channel::<()>();
    let client = thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel("toolkit test", "bande-toolkit-test"),
            WINDOW,
        )
        .expect("window");
        let scale = window.scale();
        window.draw(filled(RED)).expect("frame");
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
            let middle = (location.y + WINDOW_HEADER_HEIGHT as i32 + WINDOW.1 as i32 / 2) * 2;
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

#[test]
fn windows_sharing_the_gpu_resize_while_others_draw_and_stay_on_the_gpu() {
    const WINDOWS: usize = 4;
    const FRAMES: u32 = 30;
    draw_on_the_gpu();
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let clients: Vec<_> = (0..WINDOWS)
        .map(|index| {
            let (client_end, server_end) = UnixStream::pair().expect("socket pair");
            server.insert_client(server_end).expect("insert client");
            thread::spawn(move || {
                let mut window = Window::open_on(
                    client_end,
                    Role::toplevel("toolkit test", "bande-toolkit-test"),
                    WINDOW,
                )
                .expect("window");
                window.wait_for_gpu();
                let mut text = TextSystem::new();
                (0..FRAMES)
                    .map(|frame| {
                        let grown = (frame + index as u32) % 2 * 24;
                        let label = text.paragraph(
                            &format!("Window {index}, frame {frame}"),
                            TextStyle::new(10.0 + frame as f32, BLUE),
                            None,
                            TextAlign::Start,
                        );
                        let scene = Scene {
                            width: WINDOW.0 + grown,
                            height: WINDOW.1 + grown,
                            background: RED,
                            items: vec![Item::Text {
                                origin: Point::new(8.0, 8.0),
                                paragraph: label,
                            }],
                        };
                        let path = window.draw(scene).expect("frame");
                        window.wait_for_frame().expect("frame callback");
                        path
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let start = Instant::now();
    while !clients.iter().all(thread::JoinHandle::is_finished) {
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "a client stalled"
        );
        server.dispatch().expect("server dispatch");
        scheduler
            .poll(|| start.elapsed(), &mut server, &mut renderer)
            .expect("compositor frame");
        thread::sleep(Duration::from_micros(200));
    }
    for client in clients {
        let paths = client.join().expect("client thread");
        assert!(
            paths.iter().all(|path| *path == FramePath::Gpu),
            "{paths:?}"
        );
    }
}

#[test]
fn a_change_under_an_opaque_window_shades_only_the_window_on_top() {
    const BELOW: (usize, usize) = (300, 220);
    const TOOLKIT: &str = "bande-toolkit-test";
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut below = TestClient::connect(&mut server);
    let stride = BELOW.0 * 4;
    let pixels = quadrants(BELOW.0, BELOW.1, stride);
    let changed: Vec<u8> = pixels.iter().map(|channel| !channel).collect();
    let image = |bytes| ShmImage {
        width: BELOW.0 as i32,
        height: BELOW.1 as i32,
        stride: stride as i32,
        format: Format::Xrgb8888,
        bytes,
    };
    below.show_window(&mut server, &image(&pixels));

    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let (drawn, on_screen) = mpsc::channel();
    let (checked, check_done) = mpsc::channel::<()>();
    let client = thread::spawn(move || {
        let mut window =
            Window::open_on(client_end, Role::toplevel("toolkit test", TOOLKIT), WINDOW)
                .expect("window");
        window.draw(filled(RED)).expect("frame");
        window.wait_for_frame().expect("frame callback");
        drawn.send(()).expect("report frame");
        check_done.recv().expect("compositor checked the frame");
    });

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let start = Instant::now();
    let mut settle = |server: &mut Compositor, renderer: &mut Renderer| {
        server.dispatch().expect("server dispatch");
        scheduler
            .poll(|| start.elapsed(), server, renderer)
            .expect("compositor frame");
        thread::sleep(Duration::from_micros(200));
    };
    while on_screen.try_recv().is_err() {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "the client stalled"
        );
        settle(&mut server, &mut renderer);
    }
    while server.is_animating() {
        settle(&mut server, &mut renderer);
    }
    renderer.render(&mut server).expect("settled frame");

    let above = server
        .windows()
        .iter()
        .find(|window| window.app_id() == TOOLKIT)
        .and_then(|window| window.geometry())
        .expect("the toolkit window");
    let under = server
        .windows()
        .iter()
        .find(|window| window.app_id() != TOOLKIT)
        .map(|window| window.location())
        .expect("the window below");
    let inset = 16;
    let damage = (
        above.loc.x + inset - under.x,
        above.loc.y + WINDOW_HEADER_HEIGHT as i32 - under.y,
        above.size.w - 2 * inset,
        above.size.h - WINDOW_HEADER_HEIGHT as i32 - inset,
    );
    below.commit_buffer(0, &image(&changed), damage);
    server.dispatch().expect("server dispatch");
    let stats = renderer.render(&mut server).expect("frame");
    checked.send(()).expect("release the client");
    client.join().expect("client thread");

    assert!(stats.repainted_pixels > 0, "{stats:?}");
    assert_eq!(stats.shaded_pixels, stats.repainted_pixels, "{stats:?}");
}
