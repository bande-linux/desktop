use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{Compositor, FrameClock, FrameScheduler, REFRESH, Renderer, Snapshot};
use smithay::utils::Size;
use wayland_client::protocol::wl_shm::Format;

use crate::{RED, ShmImage, TestClient, roundtrip};

const PATIENCE: Duration = Duration::from_secs(180);

pub fn headless(width: i32, height: i32) -> Compositor {
    let mut server = Compositor::new(Size::from((width, height))).expect("headless compositor");
    server.set_reduce_motion(true);
    server
}

pub fn chord(server: &mut Compositor, keys: &[u32]) {
    for key in keys {
        server.key(*key, true, 0);
    }
    for key in keys.iter().rev() {
        server.key(*key, false, 0);
    }
}

impl TestClient {
    pub fn window(server: &mut Compositor, app_id: &str) -> Self {
        let bytes = RED.repeat(64 * 64);
        let mut client = Self::connect(server);
        client.use_seat(server);
        client.show_window(
            server,
            &ShmImage {
                width: 64,
                height: 64,
                stride: 256,
                format: Format::Xrgb8888,
                bytes: &bytes,
            },
        );
        client.set_app_id(server, 0, app_id);
        roundtrip(server, &mut [&mut client]);
        client
    }
}

pub struct Frames {
    renderer: Renderer,
    schedulers: Vec<FrameScheduler>,
    start: Instant,
}

impl Frames {
    pub fn start() -> Self {
        Self {
            renderer: Renderer::new().expect("Vulkan renderer"),
            schedulers: vec![FrameScheduler::new(FrameClock::new(
                Duration::ZERO,
                REFRESH,
            ))],
            start: Instant::now(),
        }
    }

    pub fn add_output(&mut self, output: &str) {
        let clock = FrameClock::new(Duration::ZERO, REFRESH);
        self.schedulers
            .push(FrameScheduler::for_output(clock, output));
    }

    pub fn rendered(&self) -> u64 {
        self.renderer.frames()
    }

    pub fn snapshot(&mut self, server: &Compositor) -> Snapshot {
        self.renderer
            .snapshot(server)
            .expect("snapshot of the output")
    }

    pub fn snapshot_output(&mut self, output: &str) -> Snapshot {
        self.renderer
            .snapshot_output(output)
            .expect("snapshot of the output")
    }

    pub fn now(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn draw_at(&mut self, server: &mut Compositor, time: Duration) {
        server.dispatch().expect("server dispatch");
        for scheduler in &mut self.schedulers {
            scheduler
                .poll(|| time, server, &mut self.renderer)
                .expect("compositor frame");
        }
    }

    pub fn run_until(&mut self, server: &mut Compositor, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        let start = self.start;
        while !done() {
            assert!(Instant::now() < deadline, "waited in vain");
            self.draw_at(server, start.elapsed());
            thread::sleep(Duration::from_millis(1));
        }
    }
}
