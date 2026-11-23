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
use super::feedback::{Feedback, Tranche};
use super::pool::BufferPool;
use crate::external::{Dmabuf, DmabufPlane, dmabuf_buffer};

pub const RENDERER_VARIABLE: &str = "BANDE_RENDERER";

pub(super) struct GpuSurface {
    gpu: Arc<GpuDevice>,
    renderer: GpuRenderer,
    modifiers: Vec<Vec<u64>>,
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
    pub(super) fn follow(&mut self, state: &State) -> bool {
        let offer = offer(state);
        if offer
            .main_device
            .is_some_and(|device| !self.gpu.is_node(device))
        {
            return false;
        }
        let modifiers = modifiers(&self.gpu, &offer);
        if !self.gpu.shares_images(&modifiers) {
            return false;
        }
        if modifiers != self.modifiers {
            self.buffers.clear();
            self.modifiers = modifiers;
        }
        true
    }

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
        let (buffer, stale) = self
            .buffers
            .next(size, damage, SharedImage::is_idle, |busy| {
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

fn offer(state: &State) -> Feedback {
    state
        .surface_feedback
        .clone()
        .or_else(|| state.default_feedback.clone())
        .unwrap_or_else(|| Feedback {
            main_device: None,
            tranches: vec![Tranche {
                device: 0,
                formats: state.dmabuf_formats.clone(),
                scanout: false,
            }],
        })
}

fn modifiers(gpu: &GpuDevice, offer: &Feedback) -> Vec<Vec<u64>> {
    offer.modifiers(SHARED_FOURCC, |device| {
        offer.main_device.is_none() || gpu.is_node(device)
    })
}

pub(super) fn start_gpu(state: &State, atlas: (u32, u32)) -> Gpu {
    let choice = renderer_choice();
    let offer = offer(state);
    if choice == RendererChoice::Cpu
        || choice == RendererChoice::Automatic && GpuDevice::known_software_only()
        || state.dmabuf.is_none()
        || offer.modifiers(SHARED_FOURCC, |_| true).is_empty()
    {
        return Gpu::Unavailable;
    }
    let allow_software = choice == RendererChoice::Gpu;
    let (sender, receiver) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("gpu-start".to_owned())
        .spawn(move || {
            let opened = open_gpu(&offer, atlas, allow_software);
            match &opened {
                Ok(_) => {}
                Err(error @ DrawError::Software(_)) => {
                    tracing::info!("drawing on the CPU: {error}");
                }
                Err(error) => tracing::warn!("drawing on the CPU, the GPU did not open: {error}"),
            }
            let _ = sender.send(opened);
        });
    match spawned {
        Ok(_) => Gpu::Starting(receiver),
        Err(error) => {
            tracing::warn!("drawing on the CPU, cannot start opening the GPU: {error}");
            Gpu::Unavailable
        }
    }
}

fn open_gpu(
    offer: &Feedback,
    atlas: (u32, u32),
    allow_software: bool,
) -> Result<GpuSurface, DrawError> {
    let gpu = GpuDevice::shared(allow_software, offer.main_device)?;
    let modifiers = modifiers(&gpu, offer);
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
