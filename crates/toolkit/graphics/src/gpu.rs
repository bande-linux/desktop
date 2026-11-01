use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak, mpsc};

use vello_cpu::Glyph;

use vello_gpu::kurbo::{self, Affine, BezPath};
use vello_gpu::{
    AtlasConfig, ClearSettings, MemorySettings, PaintType, RectU16, RenderSettings, RenderSize,
    RenderTargetConfig, Renderer, Resources, TargetInit, TextureBindings,
};

use crate::damage::Region;
use crate::draw::{self, Canvas, ImageIds};
use crate::outline::GlyphOutlines;
use crate::render::{DrawError, Snapshot};
use crate::scene::{Image, Scene};
use crate::text::GlyphRun;

const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const OFFSCREEN_ATLAS: (u32, u32) = (1024, 1024);
const FALLBACK_ATLAS: (u16, u16) = (1024, 1024);
const ATLAS_STEP: u32 = 256;
const ATLAS_BUDGET_PIXELS: u64 = 8 * 4096 * 4096;

pub struct GpuDevice {
    instance: wgpu::Instance,
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
        let instance = vulkan_instance();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .map_err(DrawError::Adapter)?;
        Self::open(instance, adapter)
    }

    pub fn known_software_only() -> bool {
        SOFTWARE_ONLY.load(Ordering::Relaxed)
    }

    /// # Safety
    ///
    /// The handles in `target` must stay valid until the returned surface is dropped.
    pub unsafe fn shared_for(
        target: wgpu::SurfaceTargetUnsafe,
        allow_software: bool,
    ) -> Result<(Arc<Self>, wgpu::Surface<'static>), DrawError> {
        let mut shared = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(gpu) = shared.upgrade() {
            let surface = unsafe { gpu.instance.create_surface_unsafe(target) }
                .map_err(|error| DrawError::Gpu(error.to_string()))?;
            if !gpu.adapter.is_surface_supported(&surface) {
                return Err(DrawError::Gpu(
                    "the shared device cannot present to this surface".to_owned(),
                ));
            }
            return Ok((gpu, surface));
        }
        let instance = vulkan_instance();
        let surface = unsafe { instance.create_surface_unsafe(target) }
            .map_err(|error| DrawError::Gpu(error.to_string()))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..wgpu::RequestAdapterOptions::default()
        }))
        .map_err(DrawError::Adapter)?;
        let info = adapter.get_info();
        if !allow_software && is_software(&info) {
            SOFTWARE_ONLY.store(true, Ordering::Relaxed);
            return Err(DrawError::Software(info.name));
        }
        let gpu = Arc::new(Self::open(instance, adapter)?);
        *shared = Arc::downgrade(&gpu);
        Ok((gpu, surface))
    }

    fn open(instance: wgpu::Instance, adapter: wgpu::Adapter) -> Result<Self, DrawError> {
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .map_err(DrawError::Device)?;
        device.on_uncaptured_error(Arc::new(|error| tracing::error!(%error, "GPU error")));
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            submission: Mutex::new(()),
        })
    }

    pub fn configure(
        &self,
        surface: &wgpu::Surface<'static>,
        config: &wgpu::SurfaceConfiguration,
    ) -> Result<(), DrawError> {
        let _submitting = self.submitting();
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        surface.configure(&self.device, config);
        match pollster::block_on(scope.pop()) {
            Some(error) => Err(DrawError::Gpu(error.to_string())),
            None => Ok(()),
        }
    }

    fn submitting(&self) -> MutexGuard<'_, ()> {
        self.submission
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }
}

pub struct GpuRenderer {
    gpu: Arc<GpuDevice>,
    format: wgpu::TextureFormat,
    atlas: (u16, u16),
    renderer: Renderer,
    resources: Resources,
    outlines: GlyphOutlines,
    scene: vello_gpu::Scene,
    depth: Option<(RenderSize, wgpu::TextureView)>,
    image_ids: ImageIds,
    frame: Option<wgpu::Texture>,
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
        Ok(Self::with_device(
            Arc::new(GpuDevice::offscreen()?),
            OFFSCREEN_FORMAT,
            OFFSCREEN_ATLAS,
        ))
    }

    pub fn with_device(
        gpu: Arc<GpuDevice>,
        format: wgpu::TextureFormat,
        atlas_hint: (u32, u32),
    ) -> Self {
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
            outlines: GlyphOutlines::default(),
            scene: vello_gpu::Scene::new(1, 1),
            depth: None,
            image_ids: ImageIds::default(),
            frame: None,
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

    pub fn present(
        &mut self,
        scene: &Scene,
        scale: f64,
        surface: &wgpu::Surface<'static>,
        damage: Option<&[Region]>,
    ) -> Result<(), DrawError> {
        let output = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(output)
            | wgpu::CurrentSurfaceTexture::Suboptimal(output) => output,
            other => return Err(DrawError::Gpu(format!("no surface texture: {other:?}"))),
        };
        let gpu = Arc::clone(&self.gpu);
        let _submitting = gpu.submitting();
        let size = output.texture.size();
        let kept = self
            .frame
            .as_ref()
            .is_some_and(|frame| frame.size() == size && frame.format() == output.texture.format());
        if !kept {
            self.frame = Some(self.gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("toolkit frame"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: output.texture.format(),
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }));
        }
        let frame = self
            .frame
            .clone()
            .ok_or_else(|| DrawError::Gpu("no frame texture".to_owned()))?;
        let damage = damage.filter(|_| kept);
        let mut encoder = match damage {
            Some([]) => self
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default()),
            _ => self.draw(scene, scale, &frame, damage)?,
        };
        encoder.copy_texture_to_texture(
            frame.as_image_copy(),
            output.texture.as_image_copy(),
            size,
        );
        self.gpu.queue.submit([encoder.finish()]);
        self.gpu.queue.present(output);
        Ok(())
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
        if let Some(regions) = damage {
            canvas.set_transform(Affine::IDENTITY);
            canvas.push_clip_layer(&outline(regions));
        }
        draw::encode(
            &mut canvas,
            &mut self.outlines,
            &self.image_ids,
            &scene.items,
            scale,
            (0, 0),
        );
        if damage.is_some() {
            canvas.pop_layer();
        }
        self.outlines.end_frame();
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
            .filter(|image| self.image_ids.get(image).is_none())
            .fold((0, 0), |(width, height), image| {
                (width.max(image.width()), height.max(image.height()))
            });
        if needed.0 > u32::from(self.atlas.0) || needed.1 > u32::from(self.atlas.1) {
            self.grow_atlas(needed);
        }
        let atlas = self.atlas;
        images.retain(|image| {
            self.image_ids.get(image).is_none()
                && image.width() <= u32::from(atlas.0)
                && image.height() <= u32::from(atlas.1)
        });
        let dropped = self.image_ids.take_dropped();
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
            self.image_ids.insert(&image, id);
            image.release_pixels();
        }
        self.gpu.queue.submit([encoder.finish()]);
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
        self.image_ids = ImageIds::default();
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

    fn push_opacity_layer(&mut self, alpha: f32) {
        self.scene.push_opacity_layer(alpha);
    }

    fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }

    fn fill_glyphs(&mut self, run: &GlyphRun, glyphs: &[Glyph]) {
        let _ = self
            .scene
            .glyph_run(self.resources, &run.font)
            .font_size(run.size)
            .hint(false)
            .normalized_coords(&run.coords)
            .fill_glyphs(glyphs.iter().copied());
    }
}
