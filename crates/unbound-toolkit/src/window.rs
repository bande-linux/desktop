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
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{
    ConnectError, Connection, Dispatch, DispatchError, EventQueue, Proxy, QueueHandle,
    delegate_noop,
};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

use crate::cpu::CpuRenderer;
use crate::gpu::GpuRenderer;
use crate::render::RenderError;
use crate::scene::Scene;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePath {
    Cpu,
    Gpu,
}

#[derive(Debug)]
pub enum WindowError {
    Connect(ConnectError),
    Dispatch(DispatchError),
    Wayland(WaylandError),
    Missing(&'static str),
    Io(io::Error),
    Render(RenderError),
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(error) => write!(f, "cannot connect to the compositor: {error}"),
            Self::Dispatch(error) => write!(f, "Wayland dispatch failed: {error}"),
            Self::Wayland(error) => write!(f, "Wayland connection failed: {error}"),
            Self::Missing(global) => write!(f, "the compositor does not offer {global}"),
            Self::Io(error) => write!(f, "shared memory failed: {error}"),
            Self::Render(error) => write!(f, "{error}"),
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
struct State {
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    wm_base: Option<XdgWmBase>,
    configured: bool,
    suggested_size: Option<(u32, u32)>,
    closed: bool,
    frame_pending: bool,
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

pub struct Window {
    gpu: Gpu,
    cpu: CpuRenderer,
    buffers: Vec<ShmBuffer>,
    rgba: Vec<u8>,
    toplevel: XdgToplevel,
    xdg_surface: XdgSurface,
    surface: WlSurface,
    queue: EventQueue<State>,
    state: State,
    connection: Connection,
    size: (u32, u32),
}

impl Window {
    pub fn open(title: &str, size: (u32, u32)) -> Result<Self, WindowError> {
        Self::on(
            Connection::connect_to_env().map_err(WindowError::Connect)?,
            title,
            size,
        )
    }

    pub fn open_on(stream: UnixStream, title: &str, size: (u32, u32)) -> Result<Self, WindowError> {
        Self::on(
            Connection::from_socket(stream).map_err(WindowError::Connect)?,
            title,
            size,
        )
    }

    fn on(connection: Connection, title: &str, size: (u32, u32)) -> Result<Self, WindowError> {
        let mut queue = connection.new_event_queue();
        let handle = queue.handle();
        connection.display().get_registry(&handle, ());
        let mut state = State::default();
        queue.roundtrip(&mut state)?;
        let compositor = state
            .compositor
            .clone()
            .ok_or(WindowError::Missing("wl_compositor"))?;
        let wm_base = state
            .wm_base
            .clone()
            .ok_or(WindowError::Missing("xdg_wm_base"))?;
        state.shm.as_ref().ok_or(WindowError::Missing("wl_shm"))?;

        let surface = compositor.create_surface(&handle, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &handle, ());
        let toplevel = xdg_surface.get_toplevel(&handle, ());
        toplevel.set_title(title.to_owned());
        surface.commit();
        while !state.configured {
            queue.blocking_dispatch(&mut state)?;
        }

        let display = connection.backend().display_ptr() as usize;
        let wl_surface = surface.id().as_ptr() as usize;
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(open_gpu(display, wl_surface));
        });

        let size = state.suggested_size.unwrap_or(size);
        Ok(Self {
            gpu: Gpu::Starting(receiver),
            cpu: CpuRenderer::new(),
            buffers: Vec::new(),
            rgba: Vec::new(),
            toplevel,
            xdg_surface,
            surface,
            queue,
            state,
            connection,
            size,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn is_closed(&self) -> bool {
        self.state.closed
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

    pub fn draw(&mut self, scene: &Scene) -> Result<FramePath, WindowError> {
        self.wait_for_frame()?;
        self.queue.dispatch_pending(&mut self.state)?;
        if let Some(size) = self.state.suggested_size.take() {
            self.size = size;
        }
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
        self.connection.flush()?;
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
        self.gpu = Gpu::Unavailable;
        for buffer in &self.buffers {
            buffer.buffer.destroy();
        }
        self.toplevel.destroy();
        self.xdg_surface.destroy();
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
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(descriptor.with_env());
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
            xdg_toplevel::Event::Close => state.closed = true,
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
