use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use smithay::utils::Size;
use unbound_compositor::{FrameClock, FrameScheduler, Headless, REFRESH, Renderer};
use unbound_toolkit::{Color, FramePath, Role, Scene, Window};

const OUTPUT: (i32, i32) = (320, 240);
const WINDOW: (u32, u32) = (200, 120);
const RED: Color = Color::hex(0xD03030);
const BLUE: Color = Color::hex(0x2F6FEB);

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
            let window = &server.windows()[0];
            let size = window.size().expect("mapped window");
            assert_eq!((size.w, size.h), (WINDOW.0 as i32, WINDOW.1 as i32));
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
