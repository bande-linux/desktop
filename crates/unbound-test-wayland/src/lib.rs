pub mod clipboard;
pub use capture::{CaptureConstraints, CaptureOutcome};
pub use cursor::CursorShape;
pub use decoration::DecorationMode;
pub use fence::SwSyncTimeline;
pub use idle::IdleEvent;
pub use shell::{Anchor, KeyboardInteractivity, Layer, LayerRequest};
pub use toplevel::{Configured, PopupRequest};
pub use wayland_protocols::xdg::shell::client::xdg_toplevel::{ResizeEdge, State as ToplevelState};
mod capture;
mod cursor;
mod decoration;
mod fence;
mod idle;
mod lock;
mod seat;
mod shell;
mod toplevel;
mod udmabuf;

use std::io::Write;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use unbound_compositor::{Headless, SessionRole};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::{
    Flags, ZwpLinuxBufferParamsV1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;
use wayland_protocols::wp::presentation_time::client::wp_presentation::WpPresentation;
use wayland_protocols::wp::presentation_time::client::wp_presentation_feedback::{
    self, WpPresentationFeedback,
};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

pub const RED: [u8; 4] = [0x00, 0x00, 0xff, 0xff];
pub const GREEN: [u8; 4] = [0x00, 0xff, 0x00, 0xff];
pub const BLUE: [u8; 4] = [0xff, 0x00, 0x00, 0xff];
pub const HALF_WHITE: [u8; 4] = [0x80, 0x80, 0x80, 0x80];

pub const LINEAR: u64 = 0;

const PAGE: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presented {
    pub time: Duration,
    pub refresh: Duration,
    pub sequence: u64,
    pub vsync: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum Memory<'a> {
    Shm,
    Dmabuf,
    Udmabuf,
    UdmabufWrittenUntil(BorrowedFd<'a>),
}

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
    dmabuf: Option<ZwpLinuxDmabufV1>,
    presentation: Option<WpPresentation>,
    presented: Vec<Presented>,
    discarded: usize,
    wm_base: Option<XdgWmBase>,
    configured: bool,
    frames_done: usize,
    registry: Option<WlRegistry>,
    advertised: Vec<(u32, String)>,
    clipboard: clipboard::Clipboard,
    input: seat::Input,
    shell: shell::Shell,
    toplevels: toplevel::Toplevels,
    lock: lock::SessionLock,
    capture: capture::Capture,
    idle: idle::Idle,
    decoration_modes: Vec<decoration::DecorationMode>,
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
            .insert_session_client(server_end, SessionRole::ClipboardDaemon)
            .expect("insert clipboard daemon");
        Self::from_socket(server, client_end)
    }

    pub(crate) fn from_socket(server: &mut Headless, client_end: UnixStream) -> Self {
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

    pub(crate) fn bind<I>(&self, interface: &str, version: u32) -> I
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
        self.show_window_in(server, image, Memory::Shm);
    }

    pub fn show_window_in(&mut self, server: &mut Headless, image: &ShmImage, memory: Memory) {
        self.map_window(server, image, memory, |_, _, _| {});
    }

    pub(crate) fn map_window(
        &mut self,
        server: &mut Headless,
        image: &ShmImage,
        memory: Memory,
        before_buffer: impl FnOnce(&Self, &WlSurface, &XdgToplevel),
    ) {
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

        before_buffer(self, &surface, &toplevel);
        self.attach(&surface, image, (0, 0, image.width, image.height), memory);
        self.exchange(server);
        self.windows.push((surface, xdg_surface, toplevel));
    }

    pub fn commit_buffer(&mut self, index: usize, image: &ShmImage, damage: (i32, i32, i32, i32)) {
        self.commit_buffer_in(index, image, damage, Memory::Shm);
    }

    pub fn commit_buffer_in(
        &mut self,
        index: usize,
        image: &ShmImage,
        damage: (i32, i32, i32, i32),
        memory: Memory,
    ) {
        let surface = self.windows[index].0.clone();
        self.attach(&surface, image, damage, memory);
        self.connection.flush().expect("client flush");
    }

    fn attach(
        &self,
        surface: &WlSurface,
        image: &ShmImage,
        damage: (i32, i32, i32, i32),
        memory: Memory,
    ) {
        let buffer = match memory {
            Memory::Shm => self.shm_buffer(image),
            Memory::Dmabuf => self.dmabuf_buffer(image, LINEAR),
            Memory::Udmabuf => self.wrap_dmabuf(
                &udmabuf::create(image.bytes).expect("udmabuf from a sealed memfd"),
                image,
                LINEAR,
            ),
            Memory::UdmabufWrittenUntil(fence) => {
                let dmabuf = udmabuf::create(image.bytes).expect("udmabuf from a sealed memfd");
                fence::add_write_fence(&dmabuf, fence).expect("a write fence on the udmabuf");
                self.wrap_dmabuf(&dmabuf, image, LINEAR)
            }
        };
        let (x, y, width, height) = damage;
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(x, y, width, height);
        surface.commit();
    }

    pub fn create_dmabuf(&mut self, server: &mut Headless, image: &ShmImage, modifier: u64) {
        self.dmabuf_buffer(image, modifier);
        let _ = self.connection.flush();
        server.dispatch().expect("server dispatch");
        if let Some(guard) = self.connection.prepare_read() {
            let _ = guard.read();
        }
        let _ = self.queue.dispatch_pending(&mut self.globals);
    }

    pub fn protocol_error(&self) -> Option<u32> {
        self.connection.protocol_error().map(|error| error.code)
    }

    fn dmabuf_buffer(&self, image: &ShmImage, modifier: u64) -> WlBuffer {
        let file = tempfile::tempfile().expect("dma-buf file");
        (&file).write_all(image.bytes).expect("dma-buf write");
        let length = u64::try_from(image.bytes.len()).expect("dma-buf size");
        file.set_len(length.next_multiple_of(PAGE))
            .expect("dma-buf length");
        self.wrap_dmabuf(&OwnedFd::from(file), image, modifier)
    }

    fn wrap_dmabuf(&self, fd: &OwnedFd, image: &ShmImage, modifier: u64) -> WlBuffer {
        let qh = self.queue.handle();
        let dmabuf = self.globals.dmabuf.as_ref().expect("zwp_linux_dmabuf_v1");
        let params = dmabuf.create_params(&qh, ());
        let stride = u32::try_from(image.stride).expect("stride");
        params.add(
            fd.as_fd(),
            0,
            0,
            stride,
            (modifier >> 32) as u32,
            modifier as u32,
        );
        let buffer = params.create_immed(
            image.width,
            image.height,
            fourcc(image.format),
            Flags::empty(),
            &qh,
            (),
        );
        params.destroy();
        buffer
    }

    pub(crate) fn shm_buffer(&self, image: &ShmImage) -> WlBuffer {
        let qh = self.queue.handle();
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
        pool.destroy();
        buffer
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

    pub fn request_presentation(&mut self, index: usize) {
        let presentation = self.globals.presentation.as_ref().expect("wp_presentation");
        presentation.feedback(&self.windows[index].0, &self.queue.handle(), ());
    }

    pub fn presented(&self) -> &[Presented] {
        &self.globals.presented
    }

    pub fn discarded(&self) -> usize {
        self.globals.discarded
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
                "zwp_linux_dmabuf_v1" => globals.dmabuf = Some(registry.bind(name, 3, qh, ())),
                "wp_presentation" => globals.presentation = Some(registry.bind(name, 1, qh, ())),
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
delegate_noop!(Globals: ignore ZwpLinuxDmabufV1);
delegate_noop!(Globals: ignore WpPresentation);

impl Dispatch<WpPresentationFeedback, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wp_presentation_feedback::Event::Presented {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
                refresh,
                seq_hi,
                seq_lo,
                flags,
            } => {
                let seconds = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
                let vsync = flags
                    .into_result()
                    .is_ok_and(|flags| flags.contains(wp_presentation_feedback::Kind::Vsync));
                globals.presented.push(Presented {
                    time: Duration::new(seconds, tv_nsec),
                    refresh: Duration::from_nanos(u64::from(refresh)),
                    sequence: (u64::from(seq_hi) << 32) | u64::from(seq_lo),
                    vsync,
                });
            }
            wp_presentation_feedback::Event::Discarded => globals.discarded += 1,
            _ => {}
        }
    }
}
delegate_noop!(Globals: ignore ZwpLinuxBufferParamsV1);

fn fourcc(format: Format) -> u32 {
    let code = match format {
        Format::Argb8888 => *b"AR24",
        Format::Xrgb8888 => *b"XR24",
        other => panic!("no dma-buf fourcc for {other:?}"),
    };
    u32::from_le_bytes(code)
}
