use smithay::reexports::wayland_server::protocol::wl_shm::Format;
use smithay::utils::{Physical, Size};
use smithay::wayland::shm::with_buffer_contents;
use unbound_render::{DeviceInfo, FrameStats, Placement, RenderError, ShmView};

use crate::state::Headless;

pub const BACKGROUND: [u8; 3] = [0x20, 0x28, 0x30];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub size: Size<i32, Physical>,
    pub rgba: Vec<u8>,
}

pub struct Renderer {
    vulkan: unbound_render::Renderer,
}

impl Renderer {
    pub fn new() -> Result<Self, RenderError> {
        Ok(Self {
            vulkan: unbound_render::Renderer::new()?,
        })
    }

    pub fn device_info(&self) -> &DeviceInfo {
        self.vulkan.device_info()
    }

    pub fn render(&mut self, compositor: &mut Headless) -> Result<FrameStats, RenderError> {
        let state = &mut compositor.state;
        let windows = &mut state.windows;
        self.vulkan
            .retain_textures(|texture| windows.iter().any(|window| window.id == texture));
        for window in windows.iter_mut() {
            let Some(pending) = window.pending.take() else {
                continue;
            };
            let uploaded = with_buffer_contents(&pending.buffer, |pointer, length, data| {
                let pool = unsafe { std::slice::from_raw_parts(pointer, length) };
                let view = ShmView {
                    pool,
                    offset: usize::try_from(data.offset).unwrap_or(usize::MAX),
                    stride: usize::try_from(data.stride).unwrap_or(usize::MAX),
                    width: data.width,
                    height: data.height,
                    opaque: data.format == Format::Xrgb8888,
                };
                self.vulkan.upload(window.id, &view, &pending.damage)
            });
            pending.buffer.release();
            if let Ok(result) = uploaded {
                result?;
            }
        }

        let placements: Vec<Placement> = windows
            .iter()
            .filter(|window| window.size().is_some())
            .map(|window| Placement {
                texture: window.id,
                x: window.location().x,
                y: window.location().y,
            })
            .collect();
        let damage = std::mem::take(&mut state.damage);
        let output = compositor.output_size();
        self.vulkan
            .render(output.w, output.h, BACKGROUND, &placements, &damage)
    }

    pub fn snapshot(&mut self, compositor: &Headless) -> Result<Snapshot, RenderError> {
        Ok(Snapshot {
            size: compositor.output_size(),
            rgba: self.vulkan.read_back()?,
        })
    }
}
