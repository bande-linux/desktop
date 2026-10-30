#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(dead_code)]

use std::os::unix::net::UnixStream;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use smithay::utils::Size;
use unbound_compositor::{Headless, Renderer, SessionRole};
use unbound_inbox::Inbox;
use unbound_screenshot::engine::{Engine, EngineEvent, Shot};
use unbound_test_wayland::{RED, ShmImage, TestClient, roundtrip};
use wayland_client::Connection;
use wayland_client::protocol::wl_shm::Format;

pub const OUTPUT: (i32, i32) = (256, 192);
pub const WINDOW: (i32, i32) = (64, 48);
pub const WAIT: Duration = Duration::from_secs(10);

pub struct Desktop {
    pub server: Headless,
    pub renderer: Renderer,
    pub app: TestClient,
    pub engine: Engine,
    pub events: Inbox<EngineEvent>,
}

impl Desktop {
    pub fn start() -> Self {
        let mut server = Headless::new(Size::from(OUTPUT)).expect("headless compositor");
        let mut renderer = Renderer::new().expect("renderer");
        let mut app = TestClient::connect(&mut server);
        app.show_window(
            &mut server,
            &ShmImage {
                width: WINDOW.0,
                height: WINDOW.1,
                stride: WINDOW.0 * 4,
                format: Format::Xrgb8888,
                bytes: &RED.repeat((WINDOW.0 * WINDOW.1) as usize),
            },
        );
        app.set_app_id(&mut server, 0, "org.example.Red");
        roundtrip(&mut server, &mut [&mut app]);
        let (client_end, server_end) = UnixStream::pair().expect("socket pair");
        server
            .insert_session_client(server_end, SessionRole::Shell)
            .expect("shell client");
        let events = Inbox::new().expect("inbox");
        let outbox = events.outbox();
        let starting = thread::spawn(move || {
            let connection = Connection::from_socket(client_end).expect("connection");
            Engine::start(connection, outbox).expect("capture engine")
        });
        let deadline = Instant::now() + WAIT;
        while !starting.is_finished() {
            assert!(
                Instant::now() < deadline,
                "the capture engine never started"
            );
            server.dispatch().expect("dispatch");
            renderer.render(&mut server).expect("frame");
            thread::sleep(Duration::from_millis(2));
        }
        let mut desktop = Self {
            server,
            renderer,
            app,
            engine: starting.join().expect("engine thread"),
            events,
        };
        desktop.pump(Duration::from_millis(100));
        desktop
    }

    pub fn step(&mut self) {
        self.server.dispatch().expect("dispatch");
        self.renderer.render(&mut self.server).expect("frame");
        self.server.dispatch().expect("dispatch");
        self.app.exchange(&mut self.server);
        thread::sleep(Duration::from_millis(2));
    }

    pub fn pump(&mut self, period: Duration) {
        let until = Instant::now() + period;
        while Instant::now() < until {
            self.step();
        }
    }

    pub fn wait<T: Send + 'static>(
        &mut self,
        work: impl FnOnce(Engine) -> T + Send + 'static,
    ) -> T {
        let engine = self.engine.clone();
        let handle: JoinHandle<T> = thread::spawn(move || work(engine));
        let deadline = Instant::now() + WAIT;
        while !handle.is_finished() {
            assert!(
                Instant::now() < deadline,
                "the capture engine never answered"
            );
            self.step();
        }
        handle.join().expect("engine call")
    }
}

pub fn rgba_at(shot: &Shot, (x, y): (i32, i32)) -> [u8; 4] {
    let rgba = shot.rgba();
    let at = ((y * shot.width as i32 + x) * 4) as usize;
    [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
}
