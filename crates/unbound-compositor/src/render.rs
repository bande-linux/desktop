use std::collections::HashMap;

use smithay::backend::allocator::dmabuf::{Dmabuf, WeakDmabuf};
use smithay::backend::allocator::{Buffer, Fourcc};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm::Format;
use smithay::utils::{Physical, Size};
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::shm::with_buffer_contents;
use unbound_render::{DeviceInfo, DmabufView, FrameStats, Placement, Rect, RenderError, ShmView};

use crate::state::Headless;

pub const BACKGROUND: [u8; 3] = [0x20, 0x28, 0x30];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub size: Size<i32, Physical>,
    pub rgba: Vec<u8>,
}

pub struct Renderer {
    vulkan: unbound_render::Renderer,
    imports: HashMap<WeakDmabuf, u64>,
    next_import: u64,
}

impl Renderer {
    pub fn new() -> Result<Self, RenderError> {
        Ok(Self {
            vulkan: unbound_render::Renderer::new()?,
            imports: HashMap::new(),
            next_import: 0,
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
        let vulkan = &mut self.vulkan;
        self.imports.retain(|weak, import| {
            let alive = !weak.is_gone();
            if !alive {
                vulkan.forget_dmabuf(*import);
            }
            alive
        });
        let mut uploaded = Vec::new();
        for window in windows.iter_mut() {
            let Some(pending) = window.pending.take() else {
                continue;
            };
            let result = match get_dmabuf(&pending.buffer) {
                Ok(dmabuf) => self.upload_dmabuf(window.id, dmabuf, &pending.damage),
                Err(_) => self.upload_shm(window.id, &pending.buffer, &pending.damage),
            };
            uploaded.push(pending.buffer);
            if let Err(error) = result {
                uploaded.iter().for_each(WlBuffer::release);
                return Err(error);
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
        let stats = self
            .vulkan
            .render(output.w, output.h, BACKGROUND, &placements, &damage);
        uploaded.iter().for_each(WlBuffer::release);
        stats
    }

    fn upload_shm(
        &mut self,
        texture: u64,
        buffer: &WlBuffer,
        damage: &[Rect],
    ) -> Result<(), RenderError> {
        with_buffer_contents(buffer, |pointer, length, data| {
            let pool = unsafe { std::slice::from_raw_parts(pointer, length) };
            let view = ShmView {
                pool,
                offset: usize::try_from(data.offset).unwrap_or(usize::MAX),
                stride: usize::try_from(data.stride).unwrap_or(usize::MAX),
                width: data.width,
                height: data.height,
                opaque: data.format == Format::Xrgb8888,
            };
            self.vulkan.upload(texture, &view, damage)
        })
        .unwrap_or(Ok(()))
    }

    fn upload_dmabuf(
        &mut self,
        texture: u64,
        dmabuf: &Dmabuf,
        damage: &[Rect],
    ) -> Result<(), RenderError> {
        let (Some(fd), Some(offset), Some(stride)) = (
            dmabuf.handles().next(),
            dmabuf.offsets().next(),
            dmabuf.strides().next(),
        ) else {
            return Ok(());
        };
        let import = *self.imports.entry(dmabuf.weak()).or_insert_with(|| {
            self.next_import += 1;
            self.next_import
        });
        let view = DmabufView {
            fd,
            offset,
            stride,
            width: dmabuf.size().w,
            height: dmabuf.size().h,
            opaque: dmabuf.format().code == Fourcc::Xrgb8888,
        };
        self.vulkan.upload_dmabuf(texture, import, &view, damage)
    }

    pub fn snapshot(&mut self, compositor: &Headless) -> Result<Snapshot, RenderError> {
        Ok(Snapshot {
            size: compositor.output_size(),
            rgba: self.vulkan.read_back()?,
        })
    }
}
