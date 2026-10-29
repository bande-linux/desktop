use std::ffi::c_void;
use std::fmt;
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use rustix::fs::{MemfdFlags, memfd_create};
use unbound_protocols::client::unbound_shell_v1::VolumeKey;
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_data_device_manager::WlDataDeviceManager;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{
    ConnectError, Connection, Dispatch, DispatchError, EventQueue, Proxy, QueueHandle,
    delegate_noop,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1;
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::{self, ZwlrLayerShellV1};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, ZwlrLayerSurfaceV1,
};

use crate::capture::Capture;
use crate::clipboard::Clipboard;
use crate::cpu::CpuRenderer;
use crate::gpu::{GpuRenderer, vulkan_instance};
use crate::lock::{Locked, SessionLock};
use crate::render::RenderError;
use crate::scene::Scene;
use crate::seat::Input;
use crate::shell::Shell;
use crate::text_input::TextInput;
use crate::ui;

pub const RENDERER_VARIABLE: &str = "UNBOUND_RENDERER";
pub const ACTIVATION_TOKEN_VARIABLE: &str = "XDG_ACTIVATION_TOKEN";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePath {
    Cpu,
    Gpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    Background,
    Bottom,
    Top,
    Overlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Edges {
    pub const ALL: Self = Self {
        top: true,
        bottom: true,
        left: true,
        right: true,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardFocus {
    None,
    Exclusive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerOptions {
    pub layer: LayerKind,
    pub anchor: Edges,
    pub exclusive_zone: i32,
    pub keyboard: KeyboardFocus,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    Toplevel { title: String, app_id: String },
    Layer(LayerOptions),
    Lock,
}

impl Role {
    pub fn toplevel(title: &str, app_id: &str) -> Self {
        Self::Toplevel {
            title: title.to_owned(),
            app_id: app_id.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WindowEvent {
    Input(ui::Event),
    PointerLeft,
    Scroll { dx: f32, dy: f32 },
    Focus(bool),
    CloseRequested,
    Pasted(String),
    ShowLauncher,
    CaptureScreen,
    CaptureArea,
    ActiveApp(String),
    AppsChanged,
    VolumeKey { key: VolumeKey, fine: bool },
    ToggleControlCenter,
}

#[derive(Debug)]
pub enum WindowError {
    Connect(ConnectError),
    Dispatch(DispatchError),
    Wayland(WaylandError),
    Missing(&'static str),
    Io(io::Error),
    Render(RenderError),
    LockRefused,
    CaptureFailed(u32),
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(error) => write!(f, "cannot connect to the compositor: {error}"),
            Self::Dispatch(error) => write!(f, "Wayland dispatch failed: {error}"),
            Self::Wayland(error) => write!(f, "Wayland connection failed: {error}"),
            Self::Missing(global) => write!(f, "the compositor does not offer {global}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Render(error) => write!(f, "{error}"),
            Self::LockRefused => write!(f, "the compositor refused to lock the session"),
            Self::CaptureFailed(reason) => {
                write!(
                    f,
                    "the compositor could not capture the screen (reason {reason})"
                )
            }
        }
    }
}

impl std::error::Error for WindowError {}

impl From<DispatchError> for WindowError {
    fn from(error: DispatchError) -> Self {
        Self::Dispatch(error)
    }
}

impl From<WaylandError> for WindowError {
    fn from(error: WaylandError) -> Self {
        Self::Wayland(error)
    }
}

impl From<io::Error> for WindowError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<RenderError> for WindowError {
    fn from(error: RenderError) -> Self {
        Self::Render(error)
    }
}

#[derive(Default)]
pub(crate) struct State {
    compositor: Option<WlCompositor>,
    pub(crate) shm: Option<WlShm>,
    wm_base: Option<XdgWmBase>,
    layer_shell: Option<ZwlrLayerShellV1>,
    pub(crate) activation: Option<XdgActivationV1>,
    pub(crate) seat: Option<WlSeat>,
    pub(crate) data_device_manager: Option<WlDataDeviceManager>,
    configured: bool,
    suggested_size: Option<(u32, u32)>,
    closed: bool,
    frame_pending: bool,
    pub(crate) events: Vec<WindowEvent>,
    pub(crate) input: Input,
    pub(crate) clipboard: Clipboard,
    pub(crate) shell: Shell,
    pub(crate) lock: SessionLock,
    pub(crate) capture: Capture,
    pub(crate) text_input: TextInput,
}

impl State {
    pub(crate) fn configure(&mut self, width: u32, height: u32) {
        self.configured = true;
        if width > 0 && height > 0 {
            self.suggested_size = Some((width, height));
        }
    }
}

struct ShmBuffer {
    buffer: WlBuffer,
    file: File,
    width: u32,
    height: u32,
    busy: Arc<AtomicBool>,
}

struct GpuSurface {
    renderer: GpuRenderer,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    configured: Option<(u32, u32)>,
}

enum Gpu {
    Starting(Receiver<Result<GpuSurface, RenderError>>),
    Ready(Box<GpuSurface>),
    Unavailable,
}

enum ShellSurface {
    Toplevel(XdgSurface, XdgToplevel),
    Layer(ZwlrLayerSurfaceV1),
    Lock(Locked),
}

pub struct Window {
    gpu: Gpu,
    cpu: CpuRenderer,
    buffers: Vec<ShmBuffer>,
    rgba: Vec<u8>,
    role: ShellSurface,
    pub(crate) surface: WlSurface,
    pub(crate) queue: EventQueue<State>,
    pub(crate) state: State,
    pub(crate) connection: Connection,
    size: (u32, u32),
    visible: bool,
    needs_redraw: bool,
}

fn anchor_of(edges: Edges) -> zwlr_layer_surface_v1::Anchor {
    let mut anchor = zwlr_layer_surface_v1::Anchor::empty();
    for (on, edge) in [
        (edges.top, zwlr_layer_surface_v1::Anchor::Top),
        (edges.bottom, zwlr_layer_surface_v1::Anchor::Bottom),
        (edges.left, zwlr_layer_surface_v1::Anchor::Left),
        (edges.right, zwlr_layer_surface_v1::Anchor::Right),
    ] {
        if on {
            anchor |= edge;
        }
    }
    anchor
}

impl Window {
    pub fn open(role: Role, size: (u32, u32)) -> Result<Self, WindowError> {
        Self::on(
            Connection::connect_to_env().map_err(WindowError::Connect)?,
            role,
            size,
        )
    }

    pub fn open_on(stream: UnixStream, role: Role, size: (u32, u32)) -> Result<Self, WindowError> {
        Self::on(
            Connection::from_socket(stream).map_err(WindowError::Connect)?,
            role,
            size,
        )
    }

    fn on(connection: Connection, role: Role, size: (u32, u32)) -> Result<Self, WindowError> {
        let mut queue = connection.new_event_queue();
        let handle = queue.handle();
        connection.display().get_registry(&handle, ());
        let mut state = State::default();
        queue.roundtrip(&mut state)?;
        let compositor = state
            .compositor
            .clone()
            .ok_or(WindowError::Missing("wl_compositor"))?;
        state.shm.as_ref().ok_or(WindowError::Missing("wl_shm"))?;
        state.join_data_device(&handle);

        let surface = compositor.create_surface(&handle, ());
        state.join_text_input(&surface, &handle);
        let role = match role {
            Role::Toplevel { title, app_id } => {
                let wm_base = state
                    .wm_base
                    .clone()
                    .ok_or(WindowError::Missing("xdg_wm_base"))?;
                let xdg_surface = wm_base.get_xdg_surface(&surface, &handle, ());
                let toplevel = xdg_surface.get_toplevel(&handle, ());
                toplevel.set_title(title);
                toplevel.set_app_id(app_id);
                ShellSurface::Toplevel(xdg_surface, toplevel)
            }
            Role::Layer(options) => {
                let layer_shell = state
                    .layer_shell
                    .clone()
                    .ok_or(WindowError::Missing("zwlr_layer_shell_v1"))?;
                let layer = match options.layer {
                    LayerKind::Background => zwlr_layer_shell_v1::Layer::Background,
                    LayerKind::Bottom => zwlr_layer_shell_v1::Layer::Bottom,
                    LayerKind::Top => zwlr_layer_shell_v1::Layer::Top,
                    LayerKind::Overlay => zwlr_layer_shell_v1::Layer::Overlay,
                };
                let layer_surface = layer_shell.get_layer_surface(
                    &surface,
                    None,
                    layer,
                    options.namespace,
                    &handle,
                    (),
                );
                layer_surface.set_anchor(anchor_of(options.anchor));
                let stretched = |a: bool, b: bool, length: u32| if a && b { 0 } else { length };
                layer_surface.set_size(
                    stretched(options.anchor.left, options.anchor.right, size.0),
                    stretched(options.anchor.top, options.anchor.bottom, size.1),
                );
                layer_surface.set_exclusive_zone(options.exclusive_zone);
                layer_surface.set_keyboard_interactivity(match options.keyboard {
                    KeyboardFocus::None => zwlr_layer_surface_v1::KeyboardInteractivity::None,
                    KeyboardFocus::Exclusive => {
                        zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive
                    }
                });
                ShellSurface::Layer(layer_surface)
            }
            Role::Lock => ShellSurface::Lock(Locked::lock(&mut state, &mut queue, &surface)?),
        };
        if let (Some(activation), Ok(token)) = (
            state.activation.as_ref(),
            std::env::var(ACTIVATION_TOKEN_VARIABLE),
        ) {
            activation.activate(token, &surface);
        }
        if !matches!(role, ShellSurface::Lock(_)) {
            surface.commit();
        }
        while !state.configured {
            queue.blocking_dispatch(&mut state)?;
        }

        let gpu = if std::env::var_os(RENDERER_VARIABLE).is_some_and(|value| value == "cpu") {
            Gpu::Unavailable
        } else {
            let display = connection.backend().display_ptr() as usize;
            let wl_surface = surface.id().as_ptr() as usize;
            let (sender, receiver) = mpsc::channel();
            thread::spawn(move || {
                let _ = sender.send(open_gpu(display, wl_surface));
            });
            Gpu::Starting(receiver)
        };

        let size = state.suggested_size.unwrap_or(size);
        Ok(Self {
            gpu,
            cpu: CpuRenderer::new(),
            buffers: Vec::new(),
            rgba: Vec::new(),
            role,
            surface,
            queue,
            state,
            connection,
            size,
            visible: true,
            needs_redraw: true,
        })
    }

    pub fn unlock(&mut self) -> Result<(), WindowError> {
        if let ShellSurface::Lock(locked) = &self.role {
            locked.unlock();
        }
        self.flush()
    }

    pub fn size(&self) -> (u32, u32) {
        self.state.suggested_size.unwrap_or(self.size)
    }

    pub fn set_title(&mut self, title: &str) {
        if let ShellSurface::Toplevel(_, toplevel) = &self.role {
            toplevel.set_title(title.to_owned());
        }
    }

    pub fn is_closed(&self) -> bool {
        self.state.closed
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn request_redraw(&mut self) {
        self.needs_redraw = true;
    }

    pub(crate) fn wants_frame(&self) -> bool {
        self.visible && !self.state.frame_pending && (self.needs_redraw || self.resized())
    }

    fn resized(&self) -> bool {
        self.state
            .suggested_size
            .is_some_and(|size| size != self.size)
    }

    pub fn gpu_ready(&mut self) -> bool {
        self.poll_gpu();
        matches!(self.gpu, Gpu::Ready(_))
    }

    pub fn wait_for_gpu(&mut self) -> bool {
        if let Gpu::Starting(receiver) = &self.gpu {
            self.gpu = match receiver.recv() {
                Ok(Ok(gpu)) => Gpu::Ready(Box::new(gpu)),
                Ok(Err(_)) | Err(_) => Gpu::Unavailable,
            };
        }
        matches!(self.gpu, Gpu::Ready(_))
    }

    pub fn wait_for_frame(&mut self) -> Result<(), WindowError> {
        while self.state.frame_pending && !self.state.closed {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        Ok(())
    }

    pub fn dispatch(&mut self) -> Result<(), WindowError> {
        self.queue.blocking_dispatch(&mut self.state)?;
        Ok(())
    }

    pub(crate) fn dispatch_pending(&mut self) -> Result<(), WindowError> {
        self.queue.dispatch_pending(&mut self.state)?;
        Ok(())
    }

    pub(crate) fn flush(&mut self) -> Result<(), WindowError> {
        match self.connection.flush() {
            Ok(()) => Ok(()),
            Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn take_events(&mut self) -> Vec<WindowEvent> {
        std::mem::take(&mut self.state.events)
    }

    pub fn hide(&mut self) -> Result<(), WindowError> {
        if !self.visible {
            return Ok(());
        }
        self.visible = false;
        self.surface.attach(None, 0, 0);
        self.surface.commit();
        self.state.frame_pending = false;
        self.flush()
    }

    pub fn show(&mut self) -> Result<(), WindowError> {
        if self.visible {
            return Ok(());
        }
        self.state.configured = false;
        self.surface.commit();
        while !self.state.configured {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        self.visible = true;
        self.needs_redraw = true;
        Ok(())
    }

    pub fn set_layer_size(&mut self, size: (u32, u32)) -> Result<(), WindowError> {
        let ShellSurface::Layer(layer) = &self.role else {
            return Err(WindowError::Missing("a layer surface"));
        };
        layer.set_size(size.0, size.1);
        if !self.visible {
            return Ok(());
        }
        self.state.configured = false;
        self.surface.commit();
        while !self.state.configured {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        self.needs_redraw = true;
        Ok(())
    }

    pub fn draw(&mut self, scene: &Scene) -> Result<FramePath, WindowError> {
        self.wait_for_frame()?;
        self.queue.dispatch_pending(&mut self.state)?;
        self.state.suggested_size = None;
        self.size = (scene.width, scene.height);
        self.needs_redraw = false;
        self.poll_gpu();
        self.surface.frame(&self.queue.handle(), ());
        self.state.frame_pending = true;
        let path = match &mut self.gpu {
            Gpu::Ready(gpu) => {
                gpu.present(scene)?;
                FramePath::Gpu
            }
            Gpu::Starting(_) | Gpu::Unavailable => {
                self.draw_on_cpu(scene)?;
                FramePath::Cpu
            }
        };
        self.flush()?;
        Ok(path)
    }

    fn poll_gpu(&mut self) {
        if let Gpu::Starting(receiver) = &self.gpu {
            match receiver.try_recv() {
                Ok(Ok(gpu)) => self.gpu = Gpu::Ready(Box::new(gpu)),
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => self.gpu = Gpu::Unavailable,
                Err(TryRecvError::Empty) => {}
            }
        }
    }

    fn draw_on_cpu(&mut self, scene: &Scene) -> Result<(), WindowError> {
        let (width, height) = (scene.width, scene.height);
        let bytes = width as usize * height as usize * 4;
        self.rgba.resize(bytes, 0);
        self.cpu.render_into(scene, &mut self.rgba)?;
        for pixel in self.rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        self.buffers.retain(|buffer| {
            buffer.width == width && buffer.height == height || buffer.busy.load(Ordering::Acquire)
        });
        let index = match self.buffers.iter().position(|buffer| {
            buffer.width == width && buffer.height == height && !buffer.busy.load(Ordering::Acquire)
        }) {
            Some(index) => index,
            None => {
                let buffer = self.create_buffer(width, height)?;
                self.buffers.push(buffer);
                self.buffers.len() - 1
            }
        };
        let buffer = &self.buffers[index];
        buffer.file.write_all_at(&self.rgba, 0)?;
        buffer.busy.store(true, Ordering::Release);
        self.surface.attach(Some(&buffer.buffer), 0, 0);
        self.surface
            .damage_buffer(0, 0, clamp_i32(width), clamp_i32(height));
        self.surface.commit();
        Ok(())
    }

    fn create_buffer(&self, width: u32, height: u32) -> Result<ShmBuffer, WindowError> {
        let shm = self
            .state
            .shm
            .as_ref()
            .ok_or(WindowError::Missing("wl_shm"))?;
        let stride = clamp_i32(width.saturating_mul(4));
        let length = u64::from(width) * u64::from(height) * 4;
        let file = File::from(
            memfd_create("unbound-toolkit", MemfdFlags::CLOEXEC).map_err(io::Error::from)?,
        );
        file.set_len(length)?;
        let handle = self.queue.handle();
        let pool = shm.create_pool(
            file.as_fd(),
            clamp_i32(u32::try_from(length).unwrap_or(u32::MAX)),
            &handle,
            (),
        );
        let busy = Arc::new(AtomicBool::new(false));
        let buffer = pool.create_buffer(
            0,
            clamp_i32(width),
            clamp_i32(height),
            stride,
            wl_shm::Format::Argb8888,
            &handle,
            busy.clone(),
        );
        pool.destroy();
        Ok(ShmBuffer {
            buffer,
            file,
            width,
            height,
            busy,
        })
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        self.wait_for_gpu();
        self.gpu = Gpu::Unavailable;
        for buffer in &self.buffers {
            buffer.buffer.destroy();
        }
        match &self.role {
            ShellSurface::Toplevel(xdg_surface, toplevel) => {
                toplevel.destroy();
                xdg_surface.destroy();
            }
            ShellSurface::Layer(layer_surface) => layer_surface.destroy(),
            ShellSurface::Lock(_) => {}
        }
        self.surface.destroy();
        let _ = self.connection.flush();
    }
}

impl GpuSurface {
    fn present(&mut self, scene: &Scene) -> Result<(), RenderError> {
        let size = (scene.width, scene.height);
        if self.configured != Some(size) {
            self.config.width = scene.width.max(1);
            self.config.height = scene.height.max(1);
            self.surface.configure(self.renderer.device(), &self.config);
            self.configured = Some(size);
        }
        self.renderer.present(scene, &self.surface)
    }
}

fn clamp_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn open_gpu(display: usize, surface: usize) -> Result<GpuSurface, RenderError> {
    let missing = || RenderError::Gpu("the Wayland surface has no libwayland pointer".to_owned());
    let display = NonNull::new(display as *mut c_void).ok_or_else(missing)?;
    let window = NonNull::new(surface as *mut c_void).ok_or_else(missing)?;
    let instance = vulkan_instance();
    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            display,
        ))),
        raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(window)),
    };
    let surface = unsafe { instance.create_surface_unsafe(target) }
        .map_err(|error| RenderError::Gpu(error.to_string()))?;
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&surface),
        ..wgpu::RequestAdapterOptions::default()
    }))
    .map_err(RenderError::Adapter)?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(RenderError::Device)?;
    let capabilities = surface.get_capabilities(&adapter);
    let format = capabilities
        .formats
        .iter()
        .copied()
        .find(|format| {
            matches!(
                format,
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
            )
        })
        .ok_or_else(|| RenderError::Gpu("the surface offers no 8-bit UNORM format".to_owned()))?;
    let alpha_mode = if capabilities
        .alpha_modes
        .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
    {
        wgpu::CompositeAlphaMode::PreMultiplied
    } else {
        wgpu::CompositeAlphaMode::Auto
    };
    let present_mode = if capabilities
        .present_modes
        .contains(&wgpu::PresentMode::Mailbox)
    {
        wgpu::PresentMode::Mailbox
    } else {
        wgpu::PresentMode::Fifo
    };
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: 1,
        height: 1,
        present_mode,
        desired_maximum_frame_latency: 2,
        alpha_mode,
        view_formats: Vec::new(),
    };
    Ok(GpuSurface {
        renderer: GpuRenderer::with_device(adapter, device, queue, format),
        surface,
        config,
        configured: None,
    })
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(5), handle, ()));
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, handle, ())),
                "xdg_wm_base" => {
                    state.wm_base = Some(registry.bind(name, version.min(5), handle, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(registry.bind(name, version.min(4), handle, ()));
                }
                "xdg_activation_v1" => {
                    state.activation = Some(registry.bind(name, 1, handle, ()));
                }
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(7), handle, ()));
                }
                "wl_data_device_manager" => {
                    state.data_device_manager =
                        Some(registry.bind(name, version.min(3), handle, ()));
                }
                "ext_session_lock_manager_v1" => {
                    state.lock.manager = Some(registry.bind(name, 1, handle, ()));
                }
                "wl_output" if state.lock.output.is_none() => {
                    state.lock.output = Some(registry.bind(name, 1, handle, ()));
                }
                "unbound_shell_v1" => {
                    state.shell.protocol = Some(registry.bind(name, version.min(4), handle, ()));
                }
                "ext_output_image_capture_source_manager_v1" => {
                    state.capture.sources = Some(registry.bind(name, 1, handle, ()));
                }
                "ext_image_copy_capture_manager_v1" => {
                    state.capture.manager = Some(registry.bind(name, 1, handle, ()));
                }
                "zwp_text_input_manager_v3" => {
                    state.text_input.manager = Some(registry.bind(name, 1, handle, ()));
                }
                "ext_foreign_toplevel_list_v1" => {
                    let _: ExtForeignToplevelListV1 = registry.bind(name, 1, handle, ());
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<XdgWmBase, ()> for State {
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

impl Dispatch<XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        xdg_surface: &XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
        }
    }
}

impl Dispatch<XdgToplevel, ()> for State {
    fn event(
        state: &mut Self,
        _: &XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } if width > 0 && height > 0 => {
                state.suggested_size = Some((width.unsigned_abs(), height.unsigned_abs()));
            }
            xdg_toplevel::Event::Close => {
                state.closed = true;
                state.events.push(WindowEvent::CloseRequested);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer_surface.ack_configure(serial);
                state.configure(width, height);
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.closed = true;
                state.events.push(WindowEvent::CloseRequested);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.frame_pending = false;
        }
    }
}

impl Dispatch<WlBuffer, Arc<AtomicBool>> for State {
    fn event(
        _: &mut Self,
        _: &WlBuffer,
        event: wl_buffer::Event,
        busy: &Arc<AtomicBool>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            busy.store(false, Ordering::Release);
        }
    }
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlShmPool);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ignore ZwlrLayerShellV1);
delegate_noop!(State: ignore XdgActivationV1);
delegate_noop!(State: ignore WlDataDeviceManager);
