use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bande_test_wayland::{Frames, headless};
use bande_toolkit::{Color, FramePath, GpuRenderer, Item, Paint, Rect, Role, Scene, Shape, Window};

const SIZE: (u32, u32) = (320, 200);
const BACKGROUND: u32 = 0x202020;
const BLUE: u32 = 0x0000ff;
const GREEN: u32 = 0x00ff00;
const CHANGES: usize = 7;

fn scene(left: u32, right: u32) -> Scene {
    let half = SIZE.0 as f32 / 2.0;
    let square = |x: f32, color: u32| Item::Fill {
        shape: Shape::rect(Rect::new(x + 10.0, 10.0, half - 20.0, SIZE.1 as f32 - 20.0)),
        paint: Paint::Solid(Color::hex(color)),
    };
    Scene {
        width: SIZE.0,
        height: SIZE.1,
        background: Color::hex(BACKGROUND),
        items: vec![square(0.0, left), square(half, right)],
    }
}

enum Drawn {
    OnGpu,
    Finished(u32),
    NeverOnGpu,
}

#[test]
fn a_gpu_window_shows_every_partial_change_through_its_rotating_buffers() {
    let software = GpuRenderer::new()
        .map(|renderer| {
            renderer
                .adapter_info()
                .name
                .to_lowercase()
                .contains("llvmpipe")
        })
        .unwrap_or(true);
    if software {
        eprintln!("skipped: only a software Vulkan device, so windows draw on the CPU");
        return;
    }
    let mut server = headless(800, 600);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let (drawn, news) = mpsc::channel();
    let (finish, finished) = mpsc::channel::<()>();
    let app = thread::spawn(move || {
        let mut window = Window::open_on(client_end, Role::toplevel("GPU", "bande-gpu-test"), SIZE)
            .expect("window");
        window.draw(scene(BLUE, BLUE)).expect("the first frame");
        if !window.wait_for_gpu() {
            drawn.send(Drawn::NeverOnGpu).expect("send");
            return;
        }
        drawn.send(Drawn::OnGpu).expect("send");
        let colour = |change: usize| {
            if change.is_multiple_of(2) {
                GREEN
            } else {
                BLUE
            }
        };
        for change in 0..CHANGES {
            assert_eq!(
                window.draw(scene(colour(change), BLUE)).expect("draw"),
                FramePath::Gpu
            );
        }
        let left = colour(CHANGES - 1);
        window.wait_for_frame().expect("the last frame");
        drawn.send(Drawn::Finished(left)).expect("send");
        finished.recv().expect("the test's go-ahead to close");
    });
    let mut frames = Frames::start();
    let mut last = None;
    frames.run_until(&mut server, || match news.try_recv() {
        Ok(Drawn::OnGpu) | Err(mpsc::TryRecvError::Empty) => false,
        Ok(Drawn::NeverOnGpu) => panic!("the window never drew on the GPU"),
        Ok(Drawn::Finished(left)) => {
            last = Some(left);
            true
        }
        Err(mpsc::TryRecvError::Disconnected) => panic!("the window's thread stopped"),
    });
    let left = last.expect("the last colour");
    let settle = Instant::now() + Duration::from_millis(300);
    frames.run_until(&mut server, || Instant::now() > settle);

    let geometry = server
        .windows()
        .iter()
        .find_map(|window| window.geometry())
        .expect("the window is shown");
    let snapshot = frames.snapshot(&server);
    let pixel = |x: i32, y: i32| {
        let start = ((y * snapshot.size.w + x) * 4) as usize;
        let [r, g, b] = [0, 1, 2].map(|channel| u32::from(snapshot.rgba[start + channel]));
        r << 16 | g << 8 | b
    };
    let middle = geometry.loc.y + geometry.size.h / 2;
    assert_eq!(
        pixel(geometry.loc.x + geometry.size.w / 4, middle),
        left,
        "the square that changed shows its last colour"
    );
    assert_eq!(
        pixel(geometry.loc.x + geometry.size.w * 3 / 4, middle),
        BLUE,
        "the square that never changed keeps its colour"
    );
    finish.send(()).expect("close the window");
    app.join().expect("the window's thread");
}
