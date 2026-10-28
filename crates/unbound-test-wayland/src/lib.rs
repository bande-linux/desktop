pub mod clipboard;

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use unbound_compositor::Headless;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

pub const RED: [u8; 4] = [0x00, 0x00, 0xff, 0xff];
pub const GREEN: [u8; 4] = [0x00, 0xff, 0x00, 0xff];
pub const BLUE: [u8; 4] = [0xff, 0x00, 0x00, 0xff];
pub const HALF_WHITE: [u8; 4] = [0x80, 0x80, 0x80, 0x80];

pub fn quadrants(width: usize, height: usize, stride: usize) -> Vec<u8> {
    let mut pixels = vec![0u8; stride * height];
    for y in 0..height {
        for x in 0..width {
            let color = match (x < width / 2, y < height / 2) {
                (true, true) => RED,
                (false, true) => GREEN,
                (true, false) => BLUE,
                (false, false) => HALF_WHITE,
            };
            let at = y * stride + x * 4;
            pixels[at..at + 4].copy_from_slice(&color);
        }
    }
    pixels
}

pub struct ShmImage<'a> {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub format: Format,
    pub bytes: &'a [u8],
}

#[derive(Default)]
struct Globals {
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    wm_base: Option<XdgWmBase>,
    configured: bool,
    frames_done: usize,
    registry: Option<WlRegistry>,
    advertised: Vec<(u32, String)>,
    clipboard: clipboard::Clipboard,
}

pub struct TestClient {
    connection: Connection,
    queue: EventQueue<Globals>,
    globals: Globals,
    windows: Vec<(WlSurface, XdgSurface, XdgToplevel)>,
}

impl TestClient {
    pub fn connect(server: &mut Headless) -> Self {
        let (client_end, server_end) = UnixStream::pair().expect("socket pair");
        server.insert_client(server_end).expect("insert client");
        Self::from_socket(server, client_end)
    }

    pub fn connect_as_clipboard_daemon(server: &mut Headless) -> Self {
        let (client_end, server_end) = UnixStream::pair().expect("socket pair");
        server
            .insert_clipboard_daemon(server_end)
            .expect("insert clipboard daemon");
        Self::from_socket(server, client_end)
    }

    fn from_socket(server: &mut Headless, client_end: UnixStream) -> Self {
        let connection = Connection::from_socket(client_end).expect("client connection");
        let queue = connection.new_event_queue();
        connection.display().get_registry(&queue.handle(), ());
        let mut client = Self {
            connection,
            queue,
            globals: Globals::default(),
            windows: Vec::new(),
        };
        client.exchange(server);
        client
    }

    pub fn advertises(&self, interface: &str) -> bool {
        self.globals
            .advertised
            .iter()
            .any(|(_, advertised)| advertised == interface)
    }

    pub fn exchange(&mut self, server: &mut Headless) {
        roundtrip(server, &mut [self]);
    }

    fn bind<I>(&self, interface: &str, version: u32) -> I
    where
        I: wayland_client::Proxy + 'static,
        Globals: Dispatch<I, ()>,
    {
        let registry = self.globals.registry.as_ref().expect("registry");
        let (name, _) = self
            .globals
            .advertised
            .iter()
            .find(|(_, advertised)| advertised == interface)
            .unwrap_or_else(|| panic!("{interface} is not advertised"));
        registry.bind(*name, version, &self.queue.handle(), ())
    }

    pub fn show_window(&mut self, server: &mut Headless, image: &ShmImage) {
        let qh = self.queue.handle();
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let wm_base = self.globals.wm_base.as_ref().expect("xdg_wm_base");
        let surface = compositor.create_surface(&qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg_surface.get_toplevel(&qh, ());
        self.globals.configured = false;
        surface.commit();
        self.exchange(server);
        assert!(self.globals.configured, "initial configure");

        let mut file = tempfile::tempfile().expect("shm file");
        file.write_all(image.bytes).expect("shm write");
        let size = i32::try_from(image.bytes.len()).expect("shm size");
        let shm = self.globals.shm.as_ref().expect("wl_shm");
        let pool = shm.create_pool(file.as_fd(), size, &qh, ());
        let buffer = pool.create_buffer(
            0,
            image.width,
            image.height,
            image.stride,
            image.format,
            &qh,
            (),
        );
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, image.width, image.height);
        surface.commit();
        pool.destroy();
        self.exchange(server);
        self.windows.push((surface, xdg_surface, toplevel));
    }

    pub fn close_window(&mut self, server: &mut Headless, index: usize) {
        let (surface, xdg_surface, toplevel) = self.windows.remove(index);
        toplevel.destroy();
        xdg_surface.destroy();
        surface.destroy();
        self.exchange(server);
    }

    pub fn request_frame(&mut self, server: &mut Headless) {
        let (surface, _, _) = self.windows.last().expect("a shown window");
        surface.frame(&self.queue.handle(), ());
        surface.commit();
        self.exchange(server);
    }

    pub fn frames_done(&self) -> usize {
        self.globals.frames_done
    }
}

pub fn roundtrip(server: &mut Headless, clients: &mut [&mut TestClient]) {
    for _ in 0..4 {
        for client in clients.iter_mut() {
            client.connection.flush().expect("client flush");
        }
        server.dispatch().expect("server dispatch");
        for client in clients.iter_mut() {
            if let Some(guard) = client.connection.prepare_read() {
                let _ = guard.read();
            }
            client
                .queue
                .dispatch_pending(&mut client.globals)
                .expect("client dispatch");
        }
    }
}

pub fn roundtrip_until(
    server: &mut Headless,
    clients: &mut [&mut TestClient],
    condition: impl Fn(&[&mut TestClient]) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        roundtrip(server, clients);
        if condition(clients) {
            return;
        }
        assert!(Instant::now() < deadline, "condition not met within 10 s");
        std::thread::sleep(Duration::from_millis(5));
    }
}

impl Dispatch<WlRegistry, ()> for Globals {
    fn event(
        globals: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            globals.registry = Some(registry.clone());
            globals.advertised.push((name, interface.clone()));
            match interface.as_str() {
                "wl_compositor" => globals.compositor = Some(registry.bind(name, 4, qh, ())),
                "wl_shm" => globals.shm = Some(registry.bind(name, 1, qh, ())),
                "xdg_wm_base" => globals.wm_base = Some(registry.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<XdgWmBase, ()> for Globals {
    fn event(
        _: &mut Self,
        wm_base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for Globals {
    fn event(
        globals: &mut Self,
        xdg_surface: &XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            globals.configured = true;
        }
    }
}

impl Dispatch<WlCallback, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            globals.frames_done += 1;
        }
    }
}

delegate_noop!(Globals: WlCompositor);
delegate_noop!(Globals: WlShmPool);
delegate_noop!(Globals: ignore WlShm);
delegate_noop!(Globals: ignore WlSurface);
delegate_noop!(Globals: ignore WlBuffer);
delegate_noop!(Globals: ignore XdgToplevel);
