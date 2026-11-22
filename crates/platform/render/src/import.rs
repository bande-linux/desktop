use std::cell::RefCell;
use std::fs::File;
use std::io::{Seek, SeekFrom};
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
use smithay::wayland::dmabuf::{
    DmabufFeedback, DmabufFeedbackBuilder, SurfaceDmabufFeedbackState, get_dmabuf,
};
use smithay::wayland::shm::with_buffer_contents;

use crate::formats::{self, Kind};
use crate::{
    BYTES_PER_PIXEL, Image, PendingCopy, Rect, RenderError, Resource, TEXTURE_FOURCCS,
    TextureImage, VulkanRenderer, VulkanTexture, alpha_swizzle, disjoint, is_opaque, target_format,
};

const GPU_LINEAR_ALIGNMENT: u32 = 16;

pub(crate) struct PlaneLayout<'a> {
    pub(crate) fd: BorrowedFd<'a>,
    pub(crate) offset: u32,
    pub(crate) stride: u32,
}

impl PlaneLayout<'_> {
    pub(crate) fn fits(&self, size: Size<i32, BufferCoords>, length: u64) -> bool {
        let row = u64::from(size.w.unsigned_abs()) * BYTES_PER_PIXEL as u64;
        fits(
            self.offset,
            self.stride,
            (row, size.h.unsigned_abs().into()),
            length,
        )
    }
}

pub(crate) fn fits(offset: u32, stride: u32, (row, rows): (u64, u64), length: u64) -> bool {
    let (offset, stride) = (u64::from(offset), u64::from(stride));
    stride >= row && offset + stride * rows.saturating_sub(1) + row <= length
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct MemoryPlane {
    pub(crate) offset: u32,
    pub(crate) stride: u32,
    pub(crate) object_size: u64,
}

pub(crate) fn memory_layouts(
    planes: &[MemoryPlane],
    expected: usize,
    rows: &[(u64, u64)],
    linear: bool,
) -> Result<Vec<vk::SubresourceLayout>, RenderError> {
    if planes.len() != expected || linear && rows.len() != planes.len() {
        return Err(RenderError::UnsupportedFormat);
    }
    planes
        .iter()
        .enumerate()
        .map(|(index, plane)| {
            let rows = rows.get(index);
            if linear
                && !(plane.offset.is_multiple_of(GPU_LINEAR_ALIGNMENT)
                    && plane.stride.is_multiple_of(GPU_LINEAR_ALIGNMENT))
            {
                return Err(RenderError::UnalignedDmabuf);
            }
            if rows.is_some() && plane.stride == 0
                || u64::from(plane.offset) >= plane.object_size
                || linear
                    && rows.is_some_and(|&rows| {
                        !fits(plane.offset, plane.stride, rows, plane.object_size)
                    })
            {
                return Err(RenderError::BufferOutOfBounds);
            }
            Ok(vk::SubresourceLayout {
                offset: u64::from(plane.offset),
                size: 0,
                row_pitch: u64::from(plane.stride),
                array_pitch: 0,
                depth_pitch: 0,
            })
        })
        .collect()
}

pub(crate) struct Planes {
    pub(crate) layouts: Vec<vk::SubresourceLayout>,
    pub(crate) objects: Vec<File>,
    pub(crate) disjoint: bool,
}

pub(crate) fn dmabuf_planes(
    dmabuf: &Dmabuf,
    capability: formats::Capability,
    texel: formats::Texel,
) -> Result<Planes, RenderError> {
    let Some(first) = dmabuf.handles().next() else {
        return Err(RenderError::UnsupportedFormat);
    };
    let mut disjoint = false;
    for fd in dmabuf.handles().skip(1) {
        disjoint |= !same_object(first, fd)?;
    }
    if disjoint && !capability.disjoint {
        return Err(RenderError::UnsupportedFormat);
    }
    let mut objects = Vec::new();
    let mut planes = Vec::new();
    for ((fd, offset), stride) in dmabuf.handles().zip(dmabuf.offsets()).zip(dmabuf.strides()) {
        let mut object = File::from(fd.try_clone_to_owned()?);
        let object_size = object.seek(SeekFrom::End(0))?;
        planes.push(MemoryPlane {
            offset,
            stride,
            object_size,
        });
        if disjoint || objects.is_empty() {
            objects.push(object);
        }
    }
    let extent = dmabuf.size();
    let rows = texel.plane_rows(extent.w.unsigned_abs(), extent.h.unsigned_abs());
    let linear = dmabuf.format().modifier == Modifier::Linear;
    let layouts = memory_layouts(&planes, capability.planes, &rows, linear)?;
    Ok(Planes {
        layouts,
        objects,
        disjoint,
    })
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

#[derive(Default)]
struct LastShown(RefCell<Option<VulkanTexture>>);

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
    fn linear_feedback(&mut self) -> Option<DmabufFeedback> {
        if self.linear_feedback.is_none() {
            let device = self.info.render_device?;
            let linear = self
                .dmabuf_planes()
                .into_keys()
                .filter(|format| format.modifier == Modifier::Linear);
            self.linear_feedback = DmabufFeedbackBuilder::new(device, linear)
                .build()
                .inspect_err(|error| tracing::warn!("cannot build a LINEAR feedback: {error}"))
                .ok();
        }
        self.linear_feedback.clone()
    }

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
            // SAFETY: `reserve` set aside `row_bytes * rows` bytes at `offset` in this mapped
            // staging buffer, so each row's destination stays within it, and `bytes` is a separate
            // slice of exactly `row_bytes`.
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

    pub(crate) fn sampled_views(
        &self,
        image: vk::Image,
        texel: formats::Texel,
    ) -> Result<(vk::ImageView, Option<vk::ImageView>), RenderError> {
        let (luma, chroma) = match texel.kind {
            Kind::Colour { opaque } => {
                return Ok((
                    self.create_view(image, texel.format, alpha_swizzle(opaque))?,
                    None,
                ));
            }
            Kind::Video { luma, chroma } => (luma, chroma),
        };
        let luma = self.plane_view(image, luma, vk::ImageAspectFlags::PLANE_0)?;
        match self.plane_view(image, chroma, vk::ImageAspectFlags::PLANE_1) {
            Ok(chroma) => Ok((luma, Some(chroma))),
            Err(error) => {
                // SAFETY: `luma` was just created and never used.
                unsafe { self.device.destroy_image_view(luma, None) };
                Err(error)
            }
        }
    }

    fn sampled_dmabuf(
        &mut self,
        dmabuf: &Dmabuf,
        capability: formats::Capability,
    ) -> Result<VulkanTexture, RenderError> {
        let code = dmabuf.format().code;
        let texel = formats::texel(code).ok_or(RenderError::UnsupportedFormat)?;
        let image = self.import_image(dmabuf, texel, (formats::SAMPLED, capability))?;
        let size = (image.width, image.height);
        let (view, chroma_view) = match self.sampled_views(image.image, texel) {
            Ok(views) => views,
            Err(error) => {
                self.destroy_image(&image);
                return Err(error);
            }
        };
        let Some(chroma_view) = chroma_view else {
            let id = self.imported_texture(Image { view, ..image });
            return Ok(self.texture_of(id, None, size, code));
        };
        let chroma_size = chroma_size(size.0, size.1);
        let chroma_id = self.imported_texture(Image {
            image: vk::Image::null(),
            memory: Vec::new(),
            view: chroma_view,
            width: chroma_size.0,
            height: chroma_size.1,
        });
        let chroma = self.texture_of(chroma_id, None, chroma_size, Fourcc::R8);
        let id = self.imported_texture(Image { view, ..image });
        Ok(self.texture_of(id, Some(Arc::new(chroma)), size, code))
    }

    fn imported_texture(&mut self, image: Image) -> u64 {
        let id = self.next_id();
        self.textures.insert(
            id,
            TextureImage {
                image,
                readable: true,
                imported: true,
            },
        );
        id
    }

    fn texture_of(
        &self,
        id: u64,
        chroma: Option<Arc<VulkanTexture>>,
        (width, height): (i32, i32),
        format: Fourcc,
    ) -> VulkanTexture {
        VulkanTexture {
            _owned: Arc::new(self.owned(Resource::Texture(id))),
            id,
            chroma,
            renderer: self.id,
            width,
            height,
            format,
        }
    }

    fn sample_directly(
        &mut self,
        dmabuf: &Dmabuf,
        capability: formats::Capability,
    ) -> Result<VulkanTexture, RenderError> {
        let weak = dmabuf.weak();
        if let Some(texture) = self.sampled_dmabufs.get(&weak) {
            return Ok(texture.clone());
        }
        let texture = self.sampled_dmabuf(dmabuf, capability)?;
        self.sampled_dmabufs.insert(weak, texture.clone());
        Ok(texture)
    }

    fn import_video_into(
        &mut self,
        dmabuf: &Dmabuf,
        surface: Option<&SurfaceData>,
    ) -> Result<VulkanTexture, RenderError> {
        let (luma, chroma) = video_layout(dmabuf)?;
        let size = dmabuf.size();
        let (chroma_width, chroma_height) = chroma_size(size.w, size.h);
        let weak = dmabuf.weak();
        let source = match self.imports.get(&weak) {
            Some(imported) => imported.buffer,
            None => {
                let imported =
                    self.import_dmabuf_memory(luma.fd, vk::BufferUsageFlags::TRANSFER_SRC)?;
                let luma_rows = (size.w.unsigned_abs().into(), size.h.unsigned_abs().into());
                let chroma_rows = (
                    u64::from(chroma_width.unsigned_abs()) * 2,
                    chroma_height.unsigned_abs().into(),
                );
                if !fits(luma.offset, luma.stride, luma_rows, imported.size)
                    || !fits(chroma.offset, chroma.stride, chroma_rows, imported.size)
                {
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
        if let Some(capability) = self.modifiers.sampled.usable(&dmabuf.format()) {
            match self.sample_directly(dmabuf, capability) {
                Ok(texture) => return Ok(texture),
                Err(error) if dmabuf.format().modifier != Modifier::Linear => return Err(error),
                Err(error) => tracing::debug!("copying a dma-buf that cannot be sampled: {error}"),
            }
        }
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
                if !layout.fits(size, imported.size) {
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
            // SAFETY: `pointer` and `length` are smithay's mapping of the client's pool, valid
            // for the whole closure (smithay survives the client shrinking it); the bytes are only
            // copied into staging, never trusted as anything but pixels.
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
        self.dmabuf_planes().into_keys().collect()
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
        let weak = dmabuf.weak();
        let imported = match self.failed_dmabufs.contains(&weak) {
            true => Err(RenderError::FailedBefore),
            false => self.import_dmabuf_into(dmabuf, surface, Some(damage)),
        };
        if let Err(error) = &imported
            && !matches!(
                error,
                RenderError::Vulkan(
                    vk::Result::ERROR_OUT_OF_DEVICE_MEMORY | vk::Result::ERROR_OUT_OF_HOST_MEMORY
                )
            )
            && self.failed_dmabufs.insert(weak)
        {
            let format = dmabuf.format();
            tracing::warn!(
                "cannot show a {:?} dma-buf with modifier {:?}: {error}",
                format.code,
                format.modifier
            );
            if format.modifier != Modifier::Linear
                && let Some(states) = surface
                && let Some(feedback) = SurfaceDmabufFeedbackState::from_states(states)
                && let Some(linear) = self.linear_feedback()
            {
                feedback.set_feedback(&linear);
            }
        }
        let Some(states) = surface else {
            return imported;
        };
        states.data_map.insert_if_missing(LastShown::default);
        let Some(last) = states.data_map.get::<LastShown>() else {
            return imported;
        };
        let mut last = last.0.borrow_mut();
        match imported {
            Ok(texture) => {
                *last = Some(texture.clone());
                Ok(texture)
            }
            Err(error) => match last.as_ref().filter(|texture| texture.renderer == self.id) {
                Some(texture) => {
                    tracing::debug!("showing a surface's last buffer: {error}");
                    Ok(texture.clone())
                }
                None => Err(error),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MemoryPlane, RenderError, fits, memory_layouts};

    fn plane(offset: u32, stride: u32) -> MemoryPlane {
        MemoryPlane {
            offset,
            stride,
            object_size: 1 << 20,
        }
    }

    #[test]
    fn every_memory_plane_of_a_modifier_gets_its_own_layout() {
        let layouts = |planes: &[MemoryPlane], expected, rows: Option<&[(u64, u64)]>| {
            let linear = rows.is_some();
            let rows = rows.unwrap_or(&[(512, 64)]);
            memory_layouts(planes, expected, rows, linear).map(|layouts| {
                layouts
                    .iter()
                    .map(|layout| {
                        assert_eq!(
                            (layout.size, layout.array_pitch, layout.depth_pitch),
                            (0, 0, 0)
                        );
                        (layout.offset, layout.row_pitch)
                    })
                    .collect::<Vec<_>>()
            })
        };
        let rows = [(256, 64)];
        assert_eq!(
            layouts(&[plane(0, 256)], 1, Some(&rows)).ok(),
            Some(vec![(0, 256)])
        );
        let pixels_and_aux = [plane(0, 512), plane(65536, 128)];
        assert_eq!(
            layouts(&pixels_and_aux, 2, None).ok(),
            Some(vec![(0, 512), (65536, 128)])
        );
        let pixels_and_two_aux = [plane(0, 1024), plane(262144, 256), plane(327680, 64)];
        assert_eq!(
            layouts(&pixels_and_two_aux, 3, None).ok(),
            Some(vec![(0, 1024), (262144, 256), (327680, 64)])
        );
        let pixels_aux_and_clear_colour = [plane(0, 512), plane(65536, 128), plane(98304, 0)];
        assert_eq!(
            layouts(&pixels_aux_and_clear_colour, 3, None).ok(),
            Some(vec![(0, 512), (65536, 128), (98304, 0)])
        );
        assert!(matches!(
            layouts(&pixels_and_aux[..1], 2, None),
            Err(RenderError::UnsupportedFormat)
        ));
        assert!(matches!(
            layouts(&pixels_and_aux, 1, None),
            Err(RenderError::UnsupportedFormat)
        ));
        assert!(matches!(
            layouts(&[plane(1 << 20, 512)], 1, None),
            Err(RenderError::BufferOutOfBounds)
        ));
        assert!(matches!(
            layouts(&[plane(0, 0)], 1, None),
            Err(RenderError::BufferOutOfBounds)
        ));
        assert!(matches!(
            layouts(&[plane(0, 128)], 1, Some(&rows)),
            Err(RenderError::BufferOutOfBounds)
        ));
        assert!(matches!(
            layouts(&[plane(8, 256)], 1, Some(&rows)),
            Err(RenderError::UnalignedDmabuf)
        ));
    }

    #[test]
    fn a_plane_fits_only_with_whole_rows_inside_the_buffer() {
        let row = (64 * 4, 32);
        assert!(fits(0, 256, row, 256 * 32));
        assert!(fits(128, 512, row, 128 + 512 * 31 + 256));
        assert!(!fits(128, 512, row, 128 + 512 * 31 + 255));
        assert!(!fits(0, 252, row, 1 << 20));
        assert!(!fits(0, 0, row, 1 << 20));
        assert!(!fits(
            u32::MAX,
            u32::MAX,
            (u64::from(u32::MAX), u64::from(i32::MAX.unsigned_abs())),
            u64::from(u32::MAX) << 8
        ));
    }
}
