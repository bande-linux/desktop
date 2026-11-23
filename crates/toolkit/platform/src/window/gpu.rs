use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use bande_graphics::{
    DrawError, GpuDevice, GpuRenderer, Region, SHARED_FOURCC, Scene, SharedImage, pixels,
};
use wayland_client::QueueHandle;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;

use super::State;
use super::pool::BufferPool;
use crate::external::{Dmabuf, DmabufPlane, dmabuf_buffer};

pub const RENDERER_VARIABLE: &str = "BANDE_RENDERER";

pub(super) struct GpuSurface {
    gpu: Arc<GpuDevice>,
    renderer: GpuRenderer,
    modifiers: Vec<u64>,
    buffers: BufferPool<SharedImage>,
}

pub(super) enum Gpu {
    Starting(Receiver<Result<GpuSurface, DrawError>>),
    Ready(Box<GpuSurface>),
    Released,
    Unavailable,
}

#[derive(Clone, Copy)]
pub(super) struct GpuTarget<'a> {
    pub(super) dmabuf: &'a ZwpLinuxDmabufV1,
    pub(super) handle: &'a QueueHandle<State>,
    pub(super) surface: &'a WlSurface,
}

impl GpuSurface {
    pub(super) fn release_uploads(&mut self) {
        self.renderer.release_uploads();
    }

    pub(super) fn present(
        &mut self,
        target: GpuTarget<'_>,
        scene: &Scene,
        scale: f64,
        damage: Option<&[Region]>,
    ) -> Result<(), DrawError> {
        let size = (pixels(scene.width, scale), pixels(scene.height, scale));
        let (gpu, modifiers) = (&self.gpu, &self.modifiers);
        let (buffer, stale) = self.buffers.next(size, damage, |busy| {
            let image = gpu.shared_image(size, modifiers)?;
            let planes = [DmabufPlane {
                fd: image.fd.as_fd(),
                offset: image.offset,
                stride: image.stride,
            }];
            let dmabuf = Dmabuf {
                planes: &planes,
                width: size.0,
                height: size.1,
                fourcc: SHARED_FOURCC,
                modifier: image.modifier,
                flipped: false,
            };
            let buffer = dmabuf_buffer(target.dmabuf, target.handle, &dmabuf, busy);
            Ok((image, buffer))
        })?;
        self.renderer
            .present(scene, scale, &buffer.contents, stale.as_deref())?;
        buffer.show(target.surface, damage);
        Ok(())
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

pub(super) fn start_gpu(state: &State, atlas: (u32, u32)) -> Gpu {
    let choice = renderer_choice();
    let modifiers: Vec<u64> = state
        .dmabuf_formats
        .iter()
        .filter(|(fourcc, _)| *fourcc == SHARED_FOURCC)
        .map(|(_, modifier)| *modifier)
        .collect();
    if choice == RendererChoice::Cpu
        || choice == RendererChoice::Automatic && GpuDevice::known_software_only()
        || state.dmabuf.is_none()
        || modifiers.is_empty()
    {
        return Gpu::Unavailable;
    }
    let allow_software = choice == RendererChoice::Gpu;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let opened = open_gpu(modifiers, atlas, allow_software);
        match &opened {
            Ok(_) => {}
            Err(error @ DrawError::Software(_)) => tracing::info!("drawing on the CPU: {error}"),
            Err(error) => tracing::warn!("drawing on the CPU, the GPU did not open: {error}"),
        }
        let _ = sender.send(opened);
    });
    Gpu::Starting(receiver)
}

fn open_gpu(
    modifiers: Vec<u64>,
    atlas: (u32, u32),
    allow_software: bool,
) -> Result<GpuSurface, DrawError> {
    let gpu = GpuDevice::shared(allow_software)?;
    if !gpu.shares_images(&modifiers) {
        return Err(DrawError::Gpu(
            "the compositor takes none of the GPU's image layouts".to_owned(),
        ));
    }
    Ok(GpuSurface {
        renderer: GpuRenderer::with_device(Arc::clone(&gpu), atlas),
        gpu,
        modifiers,
        buffers: BufferPool::default(),
    })
}
