use std::cell::RefCell;
use std::fs::File;
use std::os::fd::BorrowedFd;
use std::os::unix::fs::MetadataExt;
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
    BYTES_PER_PIXEL, Image, PendingCopy, Rect, RenderError, Resource, TEXTURE_FOURCCS,
    TextureImage, VulkanRenderer, VulkanTexture, alpha_swizzle, disjoint, is_opaque,
    linear_formats, target_format,
};

const GPU_LINEAR_ALIGNMENT: u32 = 16;

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

fn chroma_size(width: i32, height: i32) -> (i32, i32) {
    ((width + 1) / 2, (height + 1) / 2)
}

fn same_object(first: BorrowedFd<'_>, second: BorrowedFd<'_>) -> Result<bool, RenderError> {
    let identity = |fd: BorrowedFd<'_>| {
        File::from(fd.try_clone_to_owned()?)
            .metadata()
            .map(|metadata| (metadata.dev(), metadata.ino()))
    };
    Ok(identity(first)? == identity(second)?)
}

fn video_layout(dmabuf: &Dmabuf) -> Result<(PlaneLayout<'_>, PlaneLayout<'_>), RenderError> {
    let modifier = dmabuf.format().modifier;
    if dmabuf.num_planes() != 2 || !matches!(modifier, Modifier::Linear | Modifier::Invalid) {
        return Err(RenderError::UnsupportedFormat);
    }
    let mut planes = dmabuf
        .handles()
        .zip(dmabuf.offsets())
        .zip(dmabuf.strides())
        .map(|((fd, offset), stride)| PlaneLayout { fd, offset, stride });
    let (Some(luma), Some(chroma)) = (planes.next(), planes.next()) else {
        return Err(RenderError::UnsupportedFormat);
    };
    if !same_object(luma.fd, chroma.fd)?
        || !chroma.offset.is_multiple_of(2)
        || !chroma.stride.is_multiple_of(2)
    {
        return Err(RenderError::UnalignedDmabuf);
    }
    Ok((luma, chroma))
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
        if width <= 0 || height <= 0 {
            return Err(RenderError::UnsupportedFormat);
        }
        if format == Fourcc::Nv12 {
            let (chroma_width, chroma_height) = chroma_size(width, height);
            let chroma = self.create_plane(chroma_width, chroma_height, vk::Format::R8G8_UNORM)?;
            let mut luma = self.create_plane(width, height, vk::Format::R8_UNORM)?;
            luma.format = format;
            luma.chroma = Some(Arc::new(chroma));
            return Ok(luma);
        }
        let opaque = is_opaque(format).ok_or(RenderError::UnsupportedFormat)?;
        let texel = target_format(format).ok_or(RenderError::UnsupportedFormat)?;
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
                imported: false,
            },
        );
        Ok(VulkanTexture {
            _owned: Arc::new(self.owned(Resource::Texture(id))),
            id,
            chroma: None,
            renderer: self.id,
            width,
            height,
            format,
        })
    }

    fn create_plane(
        &mut self,
        width: i32,
        height: i32,
        texel: vk::Format,
    ) -> Result<VulkanTexture, RenderError> {
        let image = self.create_image(
            width,
            height,
            texel,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
            vk::ComponentSwizzle::IDENTITY,
        )?;
        let id = self.next_id();
        self.textures.insert(
            id,
            TextureImage {
                image,
                readable: false,
                imported: false,
            },
        );
        Ok(VulkanTexture {
            _owned: Arc::new(self.owned(Resource::Texture(id))),
            id,
            chroma: None,
            renderer: self.id,
            width,
            height,
            format: Fourcc::R8,
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

    fn sampled_video(
        &mut self,
        dmabuf: &Dmabuf,
        luma: &PlaneLayout<'_>,
        chroma: &PlaneLayout<'_>,
    ) -> Result<VulkanTexture, RenderError> {
        let size = dmabuf.size();
        let file = File::from(luma.fd.try_clone_to_owned()?);
        let layouts = [luma, chroma].map(|plane| vk::SubresourceLayout {
            offset: u64::from(plane.offset),
            row_pitch: u64::from(plane.stride),
            ..Default::default()
        });
        let image = self.import_video_image(size.w, size.h, &layouts, file)?;
        let (chroma_width, chroma_height) = chroma_size(size.w, size.h);
        let views = self
            .plane_view(
                image.image,
                vk::Format::R8_UNORM,
                vk::ImageAspectFlags::PLANE_0,
            )
            .and_then(|luma_view| {
                self.plane_view(
                    image.image,
                    vk::Format::R8G8_UNORM,
                    vk::ImageAspectFlags::PLANE_1,
                )
                .map(|chroma_view| (luma_view, chroma_view))
                .inspect_err(|_| unsafe { self.device.destroy_image_view(luma_view, None) })
            });
        let (luma_view, chroma_view) = match views {
            Ok(views) => views,
            Err(error) => {
                self.destroy_image(&image);
                return Err(error);
            }
        };
        let chroma_id = self.next_id();
        self.textures.insert(
            chroma_id,
            TextureImage {
                image: Image {
                    image: vk::Image::null(),
                    memory: vk::DeviceMemory::null(),
                    view: chroma_view,
                    width: chroma_width,
                    height: chroma_height,
                },
                readable: true,
                imported: true,
            },
        );
        let id = self.next_id();
        self.textures.insert(
            id,
            TextureImage {
                image: Image {
                    view: luma_view,
                    ..image
                },
                readable: false,
                imported: true,
            },
        );
        Ok(VulkanTexture {
            _owned: Arc::new(self.owned(Resource::Texture(id))),
            id,
            chroma: Some(Arc::new(VulkanTexture {
                _owned: Arc::new(self.owned(Resource::Texture(chroma_id))),
                id: chroma_id,
                chroma: None,
                renderer: self.id,
                width: chroma_width,
                height: chroma_height,
                format: Fourcc::R8,
            })),
            renderer: self.id,
            width: size.w,
            height: size.h,
            format: Fourcc::Nv12,
        })
    }

    fn sample_video_directly(
        &mut self,
        dmabuf: &Dmabuf,
        luma: &PlaneLayout<'_>,
        chroma: &PlaneLayout<'_>,
    ) -> Option<VulkanTexture> {
        if !self.samples_video {
            return None;
        }
        if ![luma, chroma].iter().all(|plane| {
            plane.offset.is_multiple_of(GPU_LINEAR_ALIGNMENT)
                && plane.stride.is_multiple_of(GPU_LINEAR_ALIGNMENT)
        }) {
            return None;
        }
        let weak = dmabuf.weak();
        if let Some(texture) = self.dmabuf_textures.get(&weak).cloned() {
            self.video_acquires.push(texture.id);
            return Some(texture);
        }
        match self.sampled_video(dmabuf, luma, chroma) {
            Ok(texture) => {
                self.dmabuf_textures.insert(weak, texture.clone());
                self.video_acquires.push(texture.id);
                Some(texture)
            }
            Err(error) => {
                tracing::warn!("copying video frames: {error}");
                self.samples_video = false;
                None
            }
        }
    }

    fn import_video_into(
        &mut self,
        dmabuf: &Dmabuf,
        surface: Option<&SurfaceData>,
    ) -> Result<VulkanTexture, RenderError> {
        let (luma, chroma) = video_layout(dmabuf)?;
        if let Some(texture) = self.sample_video_directly(dmabuf, &luma, &chroma) {
            return Ok(texture);
        }
        let size = dmabuf.size();
        let (chroma_width, chroma_height) = chroma_size(size.w, size.h);
        let weak = dmabuf.weak();
        let source = match self.imports.get(&weak) {
            Some(imported) => imported.buffer,
            None => {
                let imported =
                    self.import_dmabuf_memory(luma.fd, vk::BufferUsageFlags::TRANSFER_SRC)?;
                let luma_end = u64::from(luma.offset)
                    + u64::from(size.h.unsigned_abs().saturating_sub(1)) * u64::from(luma.stride)
                    + u64::from(size.w.unsigned_abs());
                let chroma_end = u64::from(chroma.offset)
                    + u64::from(chroma_height.unsigned_abs().saturating_sub(1))
                        * u64::from(chroma.stride)
                    + u64::from(chroma_width.unsigned_abs()) * 2;
                if luma_end.max(chroma_end) > imported.size {
                    self.destroy_imported(&imported);
                    return Err(RenderError::BufferOutOfBounds);
                }
                let buffer = imported.buffer;
                self.imports.insert(weak.clone(), imported);
                buffer
            }
        };
        let cached = self.dmabuf_textures.get(&weak).cloned();
        let (texture, _) = match (surface, cached) {
            (Some(_), _) => self.surface_texture(surface, size.w, size.h, Fourcc::Nv12)?,
            (None, Some(texture)) => (texture, false),
            (None, None) => {
                let texture = self.create_texture(size.w, size.h, Fourcc::Nv12)?;
                self.dmabuf_textures.insert(weak, texture.clone());
                (texture, true)
            }
        };
        let chroma_id = texture
            .chroma
            .as_ref()
            .map(|plane| plane.id)
            .ok_or(RenderError::UnsupportedFormat)?;
        self.copies.push(PendingCopy {
            texture: texture.id,
            source,
            offset: u64::from(luma.offset),
            row_texels: luma.stride,
            rect: Rect::new(0, 0, size.w, size.h),
        });
        self.copies.push(PendingCopy {
            texture: chroma_id,
            source,
            offset: u64::from(chroma.offset),
            row_texels: chroma.stride / 2,
            rect: Rect::new(0, 0, chroma_width, chroma_height),
        });
        self.counters.uploaded_bytes += Rect::new(0, 0, size.w, size.h).area()
            + Rect::new(0, 0, chroma_width, chroma_height).area() * 2;
        Ok(texture)
    }

    fn import_dmabuf_into(
        &mut self,
        dmabuf: &Dmabuf,
        surface: Option<&SurfaceData>,
        damage: Option<&[Rectangle<i32, BufferCoords>]>,
    ) -> Result<VulkanTexture, RenderError> {
        if dmabuf.format().code == Fourcc::Nv12 {
            return self.import_video_into(dmabuf, surface);
        }
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
