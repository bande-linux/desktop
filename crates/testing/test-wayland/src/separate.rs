use std::env;
use std::io::{BufRead, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use unbound_compositor::Compositor;
use wayland_client::Connection;
use wayland_client::protocol::wl_shm::Format;

use crate::{Globals, ShmImage, TestClient, quadrants};

const SOCKET: &str = "UNBOUND_TEST_SEPARATE_APP_SOCKET";

pub struct SeparateApp {
    orders: ChildStdin,
    _socket_dir: TempDir,
}

impl SeparateApp {
    pub fn launch(server: &mut Compositor, test: &str) -> Self {
        let socket_dir = tempfile::tempdir().expect("socket dir");
        let socket = socket_dir.path().join("wayland");
        let listener = UnixListener::bind(&socket).expect("bind socket");
        let mut launcher = Command::new("setsid")
            .arg("--fork")
            .arg(env::current_exe().expect("test binary"))
            .args(["--exact", test])
            .env(SOCKET, &socket)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .expect("launch separate app");
        let orders = launcher.stdin.take().expect("separate app stdin");
        assert!(launcher.wait().expect("setsid").success());
        let (stream, _) = listener.accept().expect("separate app connects");
        server.insert_client(stream).expect("insert separate app");
        Self {
            orders,
            _socket_dir: socket_dir,
        }
    }

    pub fn show_window(&mut self, server: &mut Compositor, side: i32) {
        writeln!(self.orders, "{side}").expect("order a window");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !server
            .windows()
            .iter()
            .any(|window| window.size().is_some_and(|size| size.w == side))
        {
            assert!(Instant::now() < deadline, "separate app window not mapped");
            server.dispatch().expect("server dispatch");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

pub fn act_as_separate_app() -> bool {
    let Some(socket) = env::var_os(SOCKET) else {
        return false;
    };
    let stream = UnixStream::connect(socket).expect("connect to the compositor");
    let connection = Connection::from_socket(stream).expect("client connection");
    let mut queue = connection.new_event_queue();
    connection.display().get_registry(&queue.handle(), ());
    let mut globals = Globals::default();
    queue.roundtrip(&mut globals).expect("registry roundtrip");
    let mut client = TestClient {
        connection,
        queue,
        globals,
        windows: Vec::new(),
    };
    for order in std::io::stdin().lock().lines() {
        let side = order.expect("order").parse().expect("window side");
        client.show_window_blocking(side);
    }
    true
}

impl TestClient {
    fn show_window_blocking(&mut self, side: i32) {
        let qh = self.queue.handle();
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let wm_base = self.globals.wm_base.as_ref().expect("xdg_wm_base");
        let surface = compositor.create_surface(&qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg_surface.get_toplevel(&qh, ());
        self.globals.configured = false;
        surface.commit();
        while !self.globals.configured {
            self.queue
                .blocking_dispatch(&mut self.globals)
                .expect("initial configure");
        }
        let pixel_side = usize::try_from(side).expect("side");
        let pixels = quadrants(pixel_side, pixel_side, pixel_side * 4);
        let buffer = self.shm_buffer(&ShmImage {
            width: side,
            height: side,
            stride: side * 4,
            format: Format::Argb8888,
            bytes: &pixels,
        });
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, side, side);
        surface.commit();
        self.queue
            .roundtrip(&mut self.globals)
            .expect("window roundtrip");
        self.windows.push((surface, xdg_surface, toplevel));
    }
}
