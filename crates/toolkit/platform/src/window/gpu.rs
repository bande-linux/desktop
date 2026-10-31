use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use bande_graphics::{DrawError, GpuDevice, GpuRenderer, Region, Scene, pixels};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Proxy};

pub const RENDERER_VARIABLE: &str = "BANDE_RENDERER";

pub(super) struct GpuSurface {
    gpu: Arc<GpuDevice>,
    renderer: GpuRenderer,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    configured: Option<(u32, u32)>,
    pub(super) transparent: bool,
}

pub(super) enum Gpu {
    Starting(Receiver<Result<GpuSurface, DrawError>>),
    Ready(Box<GpuSurface>),
    Released,
    Unavailable,
}

impl GpuSurface {
    pub(super) fn present(
        &mut self,
        scene: &Scene,
        scale: f64,
        damage: Option<&[Region]>,
    ) -> Result<(), DrawError> {
        let size = (pixels(scene.width, scale), pixels(scene.height, scale));
        if self.configured != Some(size) {
            self.config.width = size.0;
            self.config.height = size.1;
            self.gpu.configure(&self.surface, &self.config)?;
            self.configured = Some(size);
        }
        self.renderer.present(scene, scale, &self.surface, damage)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RendererChoice {
    Automatic,
    Gpu,
    Cpu,
}

fn renderer_choice() -> RendererChoice {
    match std::env::var(RENDERER_VARIABLE).as_deref() {
        Ok("gpu") => RendererChoice::Gpu,
        Ok("cpu") => RendererChoice::Cpu,
        _ => RendererChoice::Automatic,
    }
}

pub(super) fn start_gpu(connection: &Connection, surface: &WlSurface, atlas: (u32, u32)) -> Gpu {
    let choice = renderer_choice();
    if choice == RendererChoice::Cpu
        || choice == RendererChoice::Automatic && GpuDevice::known_software_only()
    {
        return Gpu::Unavailable;
    }
    let allow_software = choice == RendererChoice::Gpu;
    let display = connection.backend().display_ptr() as usize;
    let wl_surface = surface.id().as_ptr() as usize;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let opened = open_gpu(display, wl_surface, atlas, allow_software);
        if let Err(error @ DrawError::Software(_)) = &opened {
            tracing::info!("drawing on the CPU: {error}");
        }
        let _ = sender.send(opened);
    });
    Gpu::Starting(receiver)
}

fn open_gpu(
    display: usize,
    surface: usize,
    atlas: (u32, u32),
    allow_software: bool,
) -> Result<GpuSurface, DrawError> {
    let missing = || DrawError::Gpu("the Wayland surface has no libwayland pointer".to_owned());
    let display = NonNull::new(display as *mut c_void).ok_or_else(missing)?;
    let window = NonNull::new(surface as *mut c_void).ok_or_else(missing)?;
    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            display,
        ))),
        raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(window)),
    };
    let (gpu, surface) = unsafe { GpuDevice::shared_for(target, allow_software) }?;
    let capabilities = surface.get_capabilities(gpu.adapter());
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
        .ok_or_else(|| DrawError::Gpu("the surface offers no 8-bit UNORM format".to_owned()))?;
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
    if !capabilities.usages.contains(wgpu::TextureUsages::COPY_DST) {
        return Err(DrawError::Gpu(
            "the surface cannot receive copied frames".to_owned(),
        ));
    }
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::COPY_DST,
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
        transparent: alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied,
        renderer: GpuRenderer::with_device(Arc::clone(&gpu), format, atlas),
        gpu,
        surface,
        config,
        configured: None,
    })
}
