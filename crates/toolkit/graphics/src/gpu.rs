use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak, mpsc};

use ash::{ext, khr, vk};
use vello_cpu::Glyph;

use vello_gpu::kurbo::{self, Affine, BezPath};
use vello_gpu::peniko::BlendMode;
use vello_gpu::{
    AtlasConfig, ClearSettings, MemorySettings, PaintType, RectU16, RenderSettings, RenderSize,
    RenderTargetConfig, Renderer, Resources, TargetInit, TextureBindings,
};
use wgpu::hal;

use crate::damage::Region;
use crate::dmabuf::{SHARED_FORMAT, SharedImage, Sharing};
use crate::draw::{self, Caches, Canvas, Frame};
use crate::render::{DrawError, Snapshot};
use crate::scene::{Image, Scene};
use crate::text::GlyphRun;

const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const OFFSCREEN_ATLAS: (u32, u32) = (1024, 1024);
const FALLBACK_ATLAS: (u16, u16) = (1024, 1024);
const ATLAS_STEP: u32 = 256;
const ATLAS_BUDGET_PIXELS: u64 = 8 * 4096 * 4096;

pub struct GpuDevice {
    sharing: Option<Sharing>,
    nodes: Vec<u64>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    submission: Mutex<()>,
}

static SHARED: Mutex<Weak<GpuDevice>> = Mutex::new(Weak::new());
static SOFTWARE_ONLY: AtomicBool = AtomicBool::new(false);
const SOFTWARE_DRIVERS: [&str; 2] = ["llvmpipe", "lavapipe"];

fn is_software(info: &wgpu::AdapterInfo) -> bool {
    let name = info.name.to_lowercase();
    info.device_type == wgpu::DeviceType::Cpu
        || SOFTWARE_DRIVERS.iter().any(|driver| name.contains(driver))
}

impl GpuDevice {
    fn offscreen() -> Result<Self, DrawError> {
        let adapter = futures_lite::future::block_on(
            instance().request_adapter(&wgpu::RequestAdapterOptions::default()),
        )
        .map_err(DrawError::Adapter)?;
        Self::open(adapter)
    }

    pub fn known_software_only() -> bool {
        SOFTWARE_ONLY.load(Ordering::Relaxed)
    }

    /// The process's device on the GPU whose DRM node is `node` (the compositor's main device), or
    /// on the only GPU there is when the compositor does not say which one it uses (software
    /// devices such as lavapipe count only when there is no GPU).
    pub fn shared(allow_software: bool, node: Option<u64>) -> Result<Arc<Self>, DrawError> {
        let mut shared = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(gpu) = shared.upgrade()
            && node.is_none_or(|node| gpu.is_node(node))
        {
            return Ok(gpu);
        }
        let adapters =
            futures_lite::future::block_on(instance().enumerate_adapters(wgpu::Backends::VULKAN));
        let adapter = match node {
            Some(node) => adapters
                .into_iter()
                .find(|adapter| drm_nodes(adapter).contains(&node))
                .ok_or_else(|| {
                    DrawError::Gpu("no Vulkan device draws on the compositor's GPU".to_owned())
                })?,
            None => {
                let (software, mut hardware): (Vec<_>, Vec<_>) = adapters
                    .into_iter()
                    .partition(|adapter| is_software(&adapter.get_info()));
                match hardware.len() {
                    1 => hardware.remove(0),
                    0 => software
                        .into_iter()
                        .next()
                        .ok_or_else(|| DrawError::Gpu("no Vulkan device".to_owned()))?,
                    _ => {
                        return Err(DrawError::Gpu(
                            "several GPUs, and the compositor does not say which one it uses"
                                .to_owned(),
                        ));
                    }
                }
            }
        };
        let info = adapter.get_info();
        if !allow_software && is_software(&info) {
            SOFTWARE_ONLY.store(true, Ordering::Relaxed);
            return Err(DrawError::Software(info.name));
        }
        let gpu = Arc::new(Self::open(adapter)?);
        *shared = Arc::downgrade(&gpu);
        Ok(gpu)
    }

    pub fn is_node(&self, node: u64) -> bool {
        self.nodes.contains(&node)
    }

    fn open(adapter: wgpu::Adapter) -> Result<Self, DrawError> {
        let descriptor = wgpu::DeviceDescriptor {
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..wgpu::DeviceDescriptor::default()
        };
        // SAFETY: the hal adapter is only used to open the device and is released before wgpu
        // wraps it.
        let hal_adapter = unsafe { adapter.as_hal::<hal::api::Vulkan>() }
            .ok_or_else(|| DrawError::Gpu("the adapter is not a Vulkan adapter".to_owned()))?;
        let wanted: Vec<&'static std::ffi::CStr> = [
            khr::external_semaphore_fd::NAME,
            ext::queue_family_foreign::NAME,
        ]
        .into_iter()
        .filter(|name| {
            hal_adapter
                .physical_device_capabilities()
                .supports_extension(name)
        })
        .collect();
        // SAFETY: the features and limits are wgpu's defaults, which every adapter supports, and
        // the callback only adds an extension the adapter supports.
        let opened = unsafe {
            hal_adapter.open_with_callback(
                descriptor.required_features,
                &descriptor.required_limits,
                &descriptor.memory_hints,
                Some(Box::new(move |arguments| {
                    for name in wanted {
                        if !arguments.extensions.contains(&name) {
                            arguments.extensions.push(name);
                        }
                    }
                })),
            )
        }
        .map_err(|error| DrawError::Gpu(format!("cannot open the Vulkan device: {error}")))?;
        drop(hal_adapter);
        // SAFETY: opened was just created from this adapter with descriptor's features and limits.
        let (device, queue) = unsafe { adapter.create_device_from_hal(opened, &descriptor) }
            .map_err(DrawError::Device)?;
        device.on_uncaptured_error(Arc::new(|error| tracing::error!(%error, "GPU error")));
        Ok(Self {
            sharing: Sharing::new(&device),
            nodes: drm_nodes(&adapter),
            adapter,
            device,
            queue,
            submission: Mutex::new(()),
        })
    }

    pub fn shares_images(&self, offered: &[Vec<u64>]) -> bool {
        self.sharing
            .as_ref()
            .is_some_and(|sharing| sharing.accepts_any(offered))
    }

    pub fn shared_image(
        &self,
        size: (u32, u32),
        offered: &[Vec<u64>],
    ) -> Result<SharedImage, DrawError> {
        self.sharing
            .as_ref()
            .ok_or_else(|| DrawError::Gpu("the GPU cannot share images".to_owned()))?
            .image(&self.device, size, offered)
    }

    fn submitting(&self) -> MutexGuard<'_, ()> {
        self.submission
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

pub struct GpuRenderer {
    gpu: Arc<GpuDevice>,
    format: wgpu::TextureFormat,
    atlas: (u16, u16),
    renderer: Renderer,
    resources: Resources,
    caches: Caches,
    scene: vello_gpu::Scene,
    depth: Option<(RenderSize, wgpu::TextureView)>,
    uploaded: bool,
}

fn drm_nodes(adapter: &wgpu::Adapter) -> Vec<u64> {
    // SAFETY: the hal adapter is only read here.
    let Some(hal_adapter) = (unsafe { adapter.as_hal::<hal::api::Vulkan>() }) else {
        return Vec::new();
    };
    if !hal_adapter
        .physical_device_capabilities()
        .supports_extension(ext::physical_device_drm::NAME)
    {
        return Vec::new();
    }
    let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut drm);
    // SAFETY: the physical device belongs to this instance and properties, with the structure it
    // chains, lives across the call.
    unsafe {
        hal_adapter
            .shared_instance()
            .raw_instance()
            .get_physical_device_properties2(hal_adapter.raw_physical_device(), &mut properties);
    }
    let node = |major: i64, minor: i64| {
        let (major, minor) = (u32::try_from(major).ok()?, u32::try_from(minor).ok()?);
        Some(libc::makedev(major, minor))
    };
    [
        (drm.has_primary == vk::TRUE).then(|| node(drm.primary_major, drm.primary_minor)),
        (drm.has_render == vk::TRUE).then(|| node(drm.render_major, drm.render_minor)),
    ]
    .into_iter()
    .flatten()
    .flatten()
    .collect()
}

fn instance() -> wgpu::Instance {
    crate::vulkan::hardware_instance().unwrap_or_else(vulkan_instance)
}

fn vulkan_instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::VULKAN;
    descriptor.flags = wgpu::InstanceFlags::empty();
    wgpu::Instance::new(descriptor.with_env())
}

fn atlas_size(needed: (u32, u32), limit: u32) -> Option<(u16, u16)> {
    if needed.0 > limit || needed.1 > limit {
        return None;
    }
    let side =
        |length: u32| u16::try_from(length.max(1).next_multiple_of(ATLAS_STEP).min(limit)).ok();
    Some((side(needed.0)?, side(needed.1)?))
}

fn atlas_settings(atlas: (u16, u16)) -> RenderSettings {
    let pixels = u64::from(atlas.0) * u64::from(atlas.1);
    let max_atlases = usize::try_from(ATLAS_BUDGET_PIXELS / pixels.max(1))
        .unwrap_or(usize::MAX)
        .max(1);
    RenderSettings {
        memory_settings: MemorySettings {
            image_atlas_config: AtlasConfig {
                atlas_size: atlas,
                max_atlases,
                ..AtlasConfig::default()
            },
            ..MemorySettings::default()
        },
        ..RenderSettings::default()
    }
}

fn renderer(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    atlas: (u16, u16),
) -> (Renderer, Resources) {
    Renderer::new_with(
        device,
        &RenderTargetConfig {
            format,
            width: 1,
            height: 1,
        },
        atlas_settings(atlas),
    )
}

impl GpuRenderer {
    pub fn new() -> Result<Self, DrawError> {
        Ok(Self::create(
            Arc::new(GpuDevice::offscreen()?),
            OFFSCREEN_FORMAT,
            OFFSCREEN_ATLAS,
        ))
    }

    pub fn with_device(gpu: Arc<GpuDevice>, atlas_hint: (u32, u32)) -> Self {
        Self::create(gpu, SHARED_FORMAT, atlas_hint)
    }

    fn create(gpu: Arc<GpuDevice>, format: wgpu::TextureFormat, atlas_hint: (u32, u32)) -> Self {
        let limit = gpu.device.limits().max_texture_dimension_2d;
        let clamped = (atlas_hint.0.min(limit), atlas_hint.1.min(limit));
        let atlas = atlas_size(clamped, limit).unwrap_or(FALLBACK_ATLAS);
        let (renderer, resources) = renderer(&gpu.device, format, atlas);
        Self {
            gpu,
            format,
            atlas,
            renderer,
            resources,
            caches: Caches::default(),
            scene: vello_gpu::Scene::new(1, 1),
            depth: None,
            uploaded: false,
        }
    }

    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.gpu.adapter.get_info()
    }

    pub fn render(&mut self, scene: &Scene) -> Result<Snapshot, DrawError> {
        let gpu = Arc::clone(&self.gpu);
        let _submitting = gpu.submitting();
        let target = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("toolkit target"),
            size: wgpu::Extent3d {
                width: scene.width,
                height: scene.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let encoder = self.draw(scene, 1.0, &target, None)?;
        let rgba = self.read_back(encoder, &target)?;
        Ok(Snapshot {
            width: scene.width,
            height: scene.height,
            rgba,
        })
    }

    pub fn release_uploads(&mut self) {
        if std::mem::take(&mut self.uploaded)
            && let Err(error) = self.gpu.device.poll(wgpu::PollType::Poll)
        {
            tracing::warn!("cannot release uploaded images: {error}");
        }
    }

    pub fn present(
        &mut self,
        scene: &Scene,
        scale: f64,
        image: &SharedImage,
        damage: Option<&[Region]>,
    ) -> Result<(), DrawError> {
        if damage.is_some_and(<[Region]>::is_empty) {
            return Ok(());
        }
        let gpu = Arc::clone(&self.gpu);
        let _submitting = gpu.submitting();
        let encoder = self.draw(scene, scale, &image.texture, damage)?;
        let sharing = gpu
            .sharing
            .as_ref()
            .ok_or_else(|| DrawError::Gpu("the GPU cannot share images".to_owned()))?;
        sharing.submit(&gpu.device, &gpu.queue, encoder.finish(), image)
    }

    fn draw(
        &mut self,
        scene: &Scene,
        scale: f64,
        target: &wgpu::Texture,
        damage: Option<&[Region]>,
    ) -> Result<wgpu::CommandEncoder, DrawError> {
        let (width, height) = crate::render::extent(scene, scale)?;
        self.scene.reset_and_resize(width, height);
        self.upload_images(scene);
        let mut canvas = GpuCanvas {
            scene: &mut self.scene,
            resources: &mut self.resources,
        };
        draw::encode(
            &mut canvas,
            &mut self.caches,
            &scene.items,
            &Frame {
                scale,
                origin: (0, 0),
                size: (width, height),
            },
            damage.map(outline),
        );
        let cleared: Vec<RectU16> = damage
            .unwrap_or_default()
            .iter()
            .map(|region| {
                let edge = |value: u32| u16::try_from(value).unwrap_or(u16::MAX);
                RectU16::new(
                    edge(region.x),
                    edge(region.y),
                    edge(region.x + region.width),
                    edge(region.y + region.height),
                )
            })
            .collect();
        let background = draw::color(scene.background);
        let clear = match damage {
            Some(_) => ClearSettings::Rects {
                color: background,
                rects: &cleared,
            },
            None => ClearSettings::Viewport { color: background },
        };

        let size = RenderSize { width, height };
        if self
            .depth
            .as_ref()
            .is_none_or(|(cached, _)| *cached != size)
        {
            let view = Renderer::create_depth_texture_view(&self.gpu.device, &size);
            self.depth = Some((size.clone(), view));
        }
        let depth = self.depth.as_ref().map(|(_, view)| view);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.renderer
            .render(
                &self.scene,
                &mut self.resources,
                &self.gpu.device,
                &self.gpu.queue,
                &mut encoder,
                &size,
                &view,
                depth,
                &TextureBindings::new(),
                TargetInit::Clear(clear),
            )
            .map_err(|error| DrawError::Gpu(error.to_string()))?;
        Ok(encoder)
    }

    fn upload_images(&mut self, scene: &Scene) {
        let mut images = Vec::new();
        draw::images(&scene.items, &mut images);
        images.sort_by_key(Image::id);
        images.dedup_by_key(|image| image.id());
        let needed = images
            .iter()
            .filter(|image| self.caches.images.get(image.id()).is_none())
            .fold((0, 0), |(width, height), image| {
                (width.max(image.width()), height.max(image.height()))
            });
        if needed.0 > u32::from(self.atlas.0) || needed.1 > u32::from(self.atlas.1) {
            self.grow_atlas(needed);
        }
        let atlas = self.atlas;
        images.retain(|image| {
            self.caches.images.get(image.id()).is_none()
                && image.width() <= u32::from(atlas.0)
                && image.height() <= u32::from(atlas.1)
        });
        let dropped = self.caches.images.take_dropped();
        if !dropped.is_empty() {
            let mut clearing = self
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            for id in dropped {
                self.renderer
                    .destroy_image(&mut self.resources, &mut clearing, id);
            }
            self.gpu.queue.submit([clearing.finish()]);
        }
        if images.is_empty() {
            return;
        }
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        for image in images {
            let Some(pixmap) = image.pixmap() else {
                continue;
            };
            let id = self.renderer.upload_image(
                &mut self.resources,
                &self.gpu.device,
                &self.gpu.queue,
                &mut encoder,
                &pixmap,
            );
            self.caches.images.insert(image.id(), id, image.liveness());
            image.release_pixels();
        }
        self.gpu.queue.submit([encoder.finish()]);
        self.uploaded = true;
    }

    fn grow_atlas(&mut self, needed: (u32, u32)) {
        let limit = self.gpu.device.limits().max_texture_dimension_2d;
        let wanted = (
            needed.0.max(u32::from(self.atlas.0)).min(limit),
            needed.1.max(u32::from(self.atlas.1)).min(limit),
        );
        let Some(atlas) = atlas_size(wanted, limit) else {
            return;
        };
        if atlas == self.atlas {
            return;
        }
        (self.renderer, self.resources) = renderer(&self.gpu.device, self.format, atlas);
        self.atlas = atlas;
        self.caches.images = Default::default();
    }

    fn read_back(
        &self,
        mut encoder: wgpu::CommandEncoder,
        target: &wgpu::Texture,
    ) -> Result<Vec<u8>, DrawError> {
        let extent = target.size();
        let row_bytes = extent.width * 4;
        let padded_row_bytes = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded_row_bytes) * u64::from(extent.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row_bytes),
                    rows_per_image: None,
                },
            },
            extent,
        );
        self.gpu.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let failed = |error: &dyn std::fmt::Display| DrawError::Gpu(error.to_string());
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| failed(&error))?;
        receiver
            .recv()
            .map_err(|error| failed(&error))?
            .map_err(|error| failed(&error))?;
        let mapped = slice.get_mapped_range().map_err(|error| failed(&error))?;
        Ok(mapped
            .chunks(padded_row_bytes as usize)
            .flat_map(|row| &row[..row_bytes as usize])
            .copied()
            .collect())
    }
}

fn outline(regions: &[Region]) -> BezPath {
    let mut path = BezPath::new();
    for region in regions {
        let (left, top) = (f64::from(region.x), f64::from(region.y));
        let (right, bottom) = (
            left + f64::from(region.width),
            top + f64::from(region.height),
        );
        path.move_to((left, top));
        path.line_to((right, top));
        path.line_to((right, bottom));
        path.line_to((left, bottom));
        path.close_path();
    }
    path
}

struct GpuCanvas<'a> {
    scene: &'a mut vello_gpu::Scene,
    resources: &'a mut Resources,
}

impl Canvas for GpuCanvas<'_> {
    fn set_paint(&mut self, paint: PaintType) {
        self.scene.set_paint(paint);
    }

    fn set_transform(&mut self, transform: Affine) {
        self.scene.set_transform(transform);
    }

    fn set_paint_transform(&mut self, transform: Affine) {
        self.scene.set_paint_transform(transform);
    }

    fn reset_paint_transform(&mut self) {
        self.scene.reset_paint_transform();
    }

    fn fill_path(&mut self, path: &BezPath) {
        self.scene.fill_path(path);
    }

    fn fill_blurred_rounded_rect(&mut self, rect: &kurbo::Rect, radius: f32, std_dev: f32) {
        self.scene
            .fill_blurred_rounded_rect(rect, radius, std_dev, false);
    }

    fn push_clip_layer(&mut self, path: &BezPath) {
        self.scene.push_clip_layer(path);
    }

    fn push_layer(&mut self, blend: BlendMode, alpha: f32) {
        self.scene
            .push_layer(None, Some(blend), Some(alpha), None, None);
    }

    fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }

    fn push_clip_path(&mut self, path: &BezPath) {
        self.scene.push_clip_path(path);
    }

    fn pop_clip(&mut self) {
        self.scene.pop_clip();
    }

    fn clear_path(&mut self, path: &BezPath) {
        self.scene
            .push_layer(Some(path), Some(draw::CLEAR), None, None, None);
        self.scene.pop_layer();
    }

    fn fill_glyphs(&mut self, run: &GlyphRun, glyphs: &[Glyph]) {
        if let Err(error) = self
            .scene
            .glyph_run(self.resources, &run.font)
            .font_size(run.size)
            .hint(false)
            .normalized_coords(&run.coords)
            .fill_glyphs(glyphs.iter().copied())
        {
            tracing::warn!("cannot draw glyphs: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsRawFd;

    use super::*;
    use crate::scene::{Item, Paint, Rect, Shape};
    use bande_design::Color;

    const LINEAR: u64 = 0;
    const SIDE: u32 = 64;
    const DMA_BUF_IOCTL_SYNC: libc::c_ulong = 0x4008_6200;
    const SYNC_READ_START: u64 = 1;
    const SYNC_READ_END: u64 = 1 | 4;

    fn scene(square: Color) -> Scene {
        Scene {
            width: SIDE,
            height: SIDE,
            background: Color::hex(0xff0000),
            items: vec![Item::Fill {
                shape: Shape::rect(Rect::new(32.0, 32.0, 16.0, 16.0)),
                paint: Paint::Solid(square),
            }],
        }
    }

    fn bgra(color: Color) -> [u8; 4] {
        [color.b, color.g, color.r, color.a]
    }

    fn pixels(image: &SharedImage, at: &[(u32, u32)]) -> Vec<[u8; 4]> {
        let mut ready = libc::pollfd {
            fd: image.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let length = image.offset as usize + image.stride as usize * SIDE as usize;
        // SAFETY: ready is a valid pollfd; the mapping covers the image's one plane, is only read
        // between the dma-buf's read sync calls and is unmapped before returning.
        unsafe {
            assert_eq!(
                libc::poll(&mut ready, 1, 5000),
                1,
                "the frame's fence never signalled"
            );
            libc::ioctl(image.fd.as_raw_fd(), DMA_BUF_IOCTL_SYNC, &SYNC_READ_START);
            let mapped = libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ,
                libc::MAP_SHARED,
                image.fd.as_raw_fd(),
                0,
            );
            assert_ne!(mapped, libc::MAP_FAILED, "cannot map the shared image");
            let bytes = std::slice::from_raw_parts(mapped.cast::<u8>(), length);
            let read = at
                .iter()
                .map(|&(x, y)| {
                    let start =
                        image.offset as usize + y as usize * image.stride as usize + x as usize * 4;
                    [
                        bytes[start],
                        bytes[start + 1],
                        bytes[start + 2],
                        bytes[start + 3],
                    ]
                })
                .collect();
            libc::munmap(mapped, length);
            libc::ioctl(image.fd.as_raw_fd(), DMA_BUF_IOCTL_SYNC, &SYNC_READ_END);
            read
        }
    }

    #[test]
    fn frames_reach_a_reused_shared_image_with_their_fence_and_after_waiting_without_one() {
        let gpu = match GpuDevice::shared(false, None) {
            Ok(gpu) => gpu,
            Err(error) => {
                eprintln!("skipped, no GPU to share images from: {error}");
                return;
            }
        };
        let linear = [vec![LINEAR]];
        let Some(sharing) = gpu
            .sharing
            .as_ref()
            .filter(|sharing| sharing.accepts_any(&linear))
        else {
            eprintln!("skipped, the GPU cannot share linear images");
            return;
        };
        let mut renderer = GpuRenderer::with_device(Arc::clone(&gpu), (SIDE, SIDE));
        let image = gpu
            .shared_image((SIDE, SIDE), &linear)
            .expect("a shared image");
        let (blue, green) = (Color::hex(0x0000ff), Color::hex(0x00ff00));
        let square = [Region {
            x: 32,
            y: 32,
            width: 16,
            height: 16,
        }];
        let (inside, outside) = ((40, 40), (8, 8));

        renderer
            .present(&scene(blue), 1.0, &image, None)
            .expect("the first frame");
        assert_eq!(
            pixels(&image, &[inside, outside]),
            [bgra(blue), bgra(Color::hex(0xff0000))]
        );

        renderer
            .present(&scene(green), 1.0, &image, Some(&square))
            .expect("a partial frame into the image the compositor had");
        assert_eq!(
            pixels(&image, &[inside, outside]),
            [bgra(green), bgra(Color::hex(0xff0000))]
        );

        sharing.give_up_fences("the test turns fences off");
        renderer
            .present(&scene(blue), 1.0, &image, Some(&square))
            .expect("a partial frame without a fence");
        assert_eq!(
            pixels(&image, &[inside, outside]),
            [bgra(blue), bgra(Color::hex(0xff0000))]
        );
    }
}
