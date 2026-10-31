use std::cell::RefCell;
use std::os::fd::BorrowedFd;
use std::slice;
use std::sync::Arc;

use ash::vk;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Buffer, Fourcc, Modifier};
use smithay::backend::renderer::{ImportDma, ImportDmaWl, ImportMem, ImportMemWl};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::utils::{Buffer as BufferCoords, Rectangle, Size};
use smithay::wayland::compositor::SurfaceData;
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::shm::with_buffer_contents;

use crate::{
    BYTES_PER_PIXEL, PendingCopy, Rect, RenderError, Resource, TEXTURE_FOURCCS, TextureImage,
    VulkanRenderer, VulkanTexture, alpha_swizzle, disjoint, is_opaque, linear_formats,
    target_format,
};

pub(crate) struct PlaneLayout<'a> {
    pub(crate) fd: BorrowedFd<'a>,
    pub(crate) offset: u32,
    pub(crate) stride: u32,
}

impl PlaneLayout<'_> {
    pub(crate) fn end(&self, size: Size<i32, BufferCoords>) -> u64 {
        let rows = u64::from(size.h.unsigned_abs().saturating_sub(1));
        u64::from(self.offset)
            + rows * u64::from(self.stride)
            + u64::from(size.w.unsigned_abs()) * BYTES_PER_PIXEL as u64
    }
}

pub(crate) fn single_plane_layout(dmabuf: &Dmabuf) -> Result<PlaneLayout<'_>, RenderError> {
    let modifier = dmabuf.format().modifier;
    if dmabuf.num_planes() != 1 || !matches!(modifier, Modifier::Linear | Modifier::Invalid) {
        return Err(RenderError::UnsupportedFormat);
    }
    let (Some(fd), Some(offset), Some(stride)) = (
        dmabuf.handles().next(),
        dmabuf.offsets().next(),
        dmabuf.strides().next(),
    ) else {
        return Err(RenderError::UnsupportedFormat);
    };
    let pixel = BYTES_PER_PIXEL as u32;
    if !offset.is_multiple_of(pixel) || !stride.is_multiple_of(pixel) {
        return Err(RenderError::UnalignedDmabuf);
    }
    Ok(PlaneLayout { fd, offset, stride })
}

#[derive(Default)]
struct SurfaceTextures(RefCell<Vec<VulkanTexture>>);

struct MemorySource<'a> {
    pool: &'a [u8],
    offset: usize,
    stride: usize,
}

fn buffer_rect(rect: &Rectangle<i32, BufferCoords>) -> Rect {
    Rect::new(rect.loc.x, rect.loc.y, rect.size.w, rect.size.h)
}

fn upload_rects(
    fresh: bool,
    width: i32,
    height: i32,
    damage: Option<&[Rectangle<i32, BufferCoords>]>,
) -> Vec<Rect> {
    let bounds = Rect::new(0, 0, width, height);
    match damage {
        Some(damage) if !fresh => disjoint(damage.iter().map(buffer_rect), bounds),
        _ => vec![bounds],
    }
}

fn shm_fourcc(format: wl_shm::Format) -> Option<Fourcc> {
    match format {
        wl_shm::Format::Argb8888 => Some(Fourcc::Argb8888),
        wl_shm::Format::Xrgb8888 => Some(Fourcc::Xrgb8888),
        _ => None,
    }
}

impl VulkanRenderer {
    fn create_texture(
        &mut self,
        width: i32,
        height: i32,
        format: Fourcc,
    ) -> Result<VulkanTexture, RenderError> {
        let opaque = is_opaque(format).ok_or(RenderError::UnsupportedFormat)?;
        let texel = target_format(format).ok_or(RenderError::UnsupportedFormat)?;
        if width <= 0 || height <= 0 {
            return Err(RenderError::UnsupportedFormat);
        }
        let image = self.create_image(
            width,
            height,
            texel,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
            alpha_swizzle(opaque),
        )?;
        let id = self.next_id();
        self.textures.insert(
            id,
            TextureImage {
                image,
                readable: false,
            },
        );
        Ok(VulkanTexture {
            _owned: Arc::new(self.owned(Resource::Texture(id))),
            id,
            renderer: self.id,
            width,
            height,
            format,
        })
    }

    fn surface_texture(
        &mut self,
        surface: Option<&SurfaceData>,
        width: i32,
        height: i32,
        format: Fourcc,
    ) -> Result<(VulkanTexture, bool), RenderError> {
        let Some(states) = surface else {
            return Ok((self.create_texture(width, height, format)?, true));
        };
        states.data_map.insert_if_missing(SurfaceTextures::default);
        let Some(cache) = states.data_map.get::<SurfaceTextures>() else {
            return Ok((self.create_texture(width, height, format)?, true));
        };
        let reusable = cache
            .0
            .borrow()
            .iter()
            .find(|texture| {
                texture.renderer == self.id
                    && texture.width == width
                    && texture.height == height
                    && texture.format == format
            })
            .cloned();
        if let Some(texture) = reusable {
            return Ok((texture, false));
        }
        let texture = self.create_texture(width, height, format)?;
        let mut textures = cache.0.borrow_mut();
        textures.retain(|cached| cached.renderer != self.id);
        textures.push(texture.clone());
        Ok((texture, true))
    }

    fn stage(
        &mut self,
        texture: u64,
        source: &MemorySource,
        rect: Rect,
    ) -> Result<(), RenderError> {
        let row_bytes = rect.width.unsigned_abs() as usize * BYTES_PER_PIXEL;
        let rows = rect.height.unsigned_abs() as usize;
        let (index, offset) = self.reserve(row_bytes * rows)?;
        let left = rect.x.unsigned_abs() as usize * BYTES_PER_PIXEL;
        let top = rect.y.unsigned_abs() as usize;
        let staging = &self.staging[index];
        for row in 0..rows {
            let start = source.offset + (top + row) * source.stride + left;
            let bytes = source
                .pool
                .get(start..start + row_bytes)
                .ok_or(RenderError::BufferOutOfBounds)?;
            let destination = offset + row * row_bytes;
            debug_assert!(destination + row_bytes <= staging.capacity);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    staging.mapped.add(destination),
                    row_bytes,
                );
            }
        }
        self.copies.push(PendingCopy {
            texture,
            source: self.staging[index].buffer,
            offset: offset as u64,
            row_texels: 0,
            rect,
        });
        self.counters.uploaded_bytes += (row_bytes * rows) as u64;
        Ok(())
    }

    fn import_dmabuf_into(
        &mut self,
        dmabuf: &Dmabuf,
        surface: Option<&SurfaceData>,
        damage: Option<&[Rectangle<i32, BufferCoords>]>,
    ) -> Result<VulkanTexture, RenderError> {
        let layout = single_plane_layout(dmabuf)?;
        let format = dmabuf.format().code;
        let size = dmabuf.size();
        let weak = dmabuf.weak();
        let source = match self.imports.get(&weak) {
            Some(imported) => imported.buffer,
            None => {
                let imported =
                    self.import_dmabuf_memory(layout.fd, vk::BufferUsageFlags::TRANSFER_SRC)?;
                if layout.end(size) > imported.size {
                    self.destroy_imported(&imported);
                    return Err(RenderError::BufferOutOfBounds);
                }
                let buffer = imported.buffer;
                self.imports.insert(weak.clone(), imported);
                buffer
            }
        };
        let cached = self.dmabuf_textures.get(&weak).cloned();
        let (texture, fresh) = match (surface, cached) {
            (Some(_), _) => self.surface_texture(surface, size.w, size.h, format)?,
            (None, Some(texture)) => (texture, false),
            (None, None) => {
                let texture = self.create_texture(size.w, size.h, format)?;
                self.dmabuf_textures.insert(weak, texture.clone());
                (texture, true)
            }
        };
        let pixel = BYTES_PER_PIXEL as u32;
        for rect in upload_rects(fresh, size.w, size.h, damage) {
            let offset = u64::from(layout.offset)
                + u64::from(rect.y.unsigned_abs()) * u64::from(layout.stride)
                + u64::from(rect.x.unsigned_abs()) * u64::from(pixel);
            self.copies.push(PendingCopy {
                texture: texture.id,
                source,
                offset,
                row_texels: layout.stride / pixel,
                rect,
            });
            self.counters.uploaded_bytes += rect.area() * u64::from(pixel);
        }
        Ok(texture)
    }
}

impl ImportMem for VulkanRenderer {
    fn import_memory(
        &mut self,
        data: &[u8],
        format: Fourcc,
        size: Size<i32, BufferCoords>,
        flipped: bool,
    ) -> Result<VulkanTexture, RenderError> {
        if flipped {
            return Err(RenderError::UnsupportedTransform);
        }
        let texture = self.create_texture(size.w, size.h, format)?;
        let source = MemorySource {
            pool: data,
            offset: 0,
            stride: size.w.unsigned_abs() as usize * BYTES_PER_PIXEL,
        };
        self.stage(texture.id, &source, Rect::new(0, 0, size.w, size.h))?;
        Ok(texture)
    }

    fn update_memory(
        &mut self,
        texture: &VulkanTexture,
        data: &[u8],
        region: Rectangle<i32, BufferCoords>,
    ) -> Result<(), RenderError> {
        if texture.renderer != self.id {
            return Err(RenderError::ForeignResource);
        }
        let source = MemorySource {
            pool: data,
            offset: 0,
            stride: texture.width.unsigned_abs() as usize * BYTES_PER_PIXEL,
        };
        let bounds = Rect::new(0, 0, texture.width, texture.height);
        match buffer_rect(&region).intersection(&bounds) {
            Some(rect) => self.stage(texture.id, &source, rect),
            None => Ok(()),
        }
    }

    fn mem_formats(&self) -> Box<dyn Iterator<Item = Fourcc>> {
        Box::new(TEXTURE_FOURCCS.into_iter())
    }
}

impl ImportMemWl for VulkanRenderer {
    fn import_shm_buffer(
        &mut self,
        buffer: &WlBuffer,
        surface: Option<&SurfaceData>,
        damage: &[Rectangle<i32, BufferCoords>],
    ) -> Result<VulkanTexture, RenderError> {
        with_buffer_contents(buffer, |pointer, length, data| {
            let format = shm_fourcc(data.format).ok_or(RenderError::UnsupportedFormat)?;
            let pool = unsafe { slice::from_raw_parts(pointer, length) };
            let (texture, fresh) =
                self.surface_texture(surface, data.width, data.height, format)?;
            let source = MemorySource {
                pool,
                offset: usize::try_from(data.offset).map_err(|_| RenderError::BufferOutOfBounds)?,
                stride: usize::try_from(data.stride).map_err(|_| RenderError::BufferOutOfBounds)?,
            };
            for rect in upload_rects(fresh, data.width, data.height, Some(damage)) {
                self.stage(texture.id, &source, rect)?;
            }
            Ok(texture)
        })
        .map_err(|_| RenderError::UnknownBuffer)?
    }
}

impl ImportDma for VulkanRenderer {
    fn dmabuf_formats(&self) -> FormatSet {
        linear_formats()
    }

    fn import_dmabuf(
        &mut self,
        dmabuf: &Dmabuf,
        damage: Option<&[Rectangle<i32, BufferCoords>]>,
    ) -> Result<VulkanTexture, RenderError> {
        self.import_dmabuf_into(dmabuf, None, damage)
    }
}

impl ImportDmaWl for VulkanRenderer {
    fn import_dma_buffer(
        &mut self,
        buffer: &WlBuffer,
        surface: Option<&SurfaceData>,
        damage: &[Rectangle<i32, BufferCoords>],
    ) -> Result<VulkanTexture, RenderError> {
        let dmabuf = get_dmabuf(buffer).map_err(|_| RenderError::UnknownBuffer)?;
        self.import_dmabuf_into(dmabuf, surface, Some(damage))
    }
}
