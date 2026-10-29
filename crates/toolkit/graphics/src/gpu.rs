use std::sync::mpsc;

use vello_cpu::Glyph;

use vello_gpu::kurbo::{self, Affine, BezPath};
use vello_gpu::{
    ClearSettings, PaintType, RenderSize, RenderTargetConfig, Renderer, Resources, TargetInit,
    TextureBindings,
};

use crate::draw::{self, Canvas, ImageIds};
use crate::outline::GlyphOutlines;
use crate::render::{RenderError, Snapshot};
use crate::scene::Scene;
use crate::text::GlyphRun;

const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub struct GpuRenderer {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    resources: Resources,
    outlines: GlyphOutlines,
    scene: vello_gpu::Scene,
    depth: Option<(RenderSize, wgpu::TextureView)>,
    image_ids: ImageIds,
}

pub fn vulkan_instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::VULKAN;
    descriptor.flags = wgpu::InstanceFlags::empty();
    wgpu::Instance::new(descriptor.with_env())
}

impl GpuRenderer {
    pub fn new() -> Result<Self, RenderError> {
        let instance = vulkan_instance();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .map_err(RenderError::Adapter)?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .map_err(RenderError::Device)?;
        Ok(Self::with_device(adapter, device, queue, OFFSCREEN_FORMAT))
    }

    pub fn with_device(
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let (renderer, resources) = Renderer::new(
            &device,
            &RenderTargetConfig {
                format,
                width: 1,
                height: 1,
            },
        );
        Self {
            adapter,
            device,
            queue,
            renderer,
            resources,
            outlines: GlyphOutlines::default(),
            scene: vello_gpu::Scene::new(1, 1),
            depth: None,
            image_ids: ImageIds::new(),
        }
    }

    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.adapter.get_info()
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn render(&mut self, scene: &Scene) -> Result<Snapshot, RenderError> {
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
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
        let encoder = self.draw(scene, 1.0, &target)?;
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
    ) -> Result<(), RenderError> {
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            other => return Err(RenderError::Gpu(format!("no surface texture: {other:?}"))),
        };
        let encoder = self.draw(scene, scale, &frame.texture)?;
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        Ok(())
    }

    fn draw(
        &mut self,
        scene: &Scene,
        scale: f64,
        target: &wgpu::Texture,
    ) -> Result<wgpu::CommandEncoder, RenderError> {
        let (width, height) = crate::render::extent(scene, scale)?;
        self.scene.reset_and_resize(width, height);
        self.upload_images(scene);
        draw::encode(
            &mut GpuCanvas {
                scene: &mut self.scene,
                resources: &mut self.resources,
            },
            &mut self.outlines,
            &self.image_ids,
            &scene.items,
            scale,
        );
        self.outlines.end_frame();

        let size = RenderSize { width, height };
        if self
            .depth
            .as_ref()
            .is_none_or(|(cached, _)| *cached != size)
        {
            let view = Renderer::create_depth_texture_view(&self.device, &size);
            self.depth = Some((size.clone(), view));
        }
        let depth = self.depth.as_ref().map(|(_, view)| view);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.renderer
            .render(
                &self.scene,
                &mut self.resources,
                &self.device,
                &self.queue,
                &mut encoder,
                &size,
                &view,
                depth,
                &TextureBindings::new(),
                TargetInit::Clear(ClearSettings::Viewport {
                    color: draw::color(scene.background),
                }),
            )
            .map_err(|error| RenderError::Gpu(error.to_string()))?;
        Ok(encoder)
    }

    fn upload_images(&mut self, scene: &Scene) {
        let mut images = Vec::new();
        draw::images(&scene.items, &mut images);
        images.retain(|image| !self.image_ids.contains_key(&image.id()));
        if images.is_empty() {
            return;
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        for image in images {
            let id = self.renderer.upload_image(
                &mut self.resources,
                &self.device,
                &self.queue,
                &mut encoder,
                &image.pixmap,
            );
            self.image_ids.insert(image.id(), id);
        }
        self.queue.submit([encoder.finish()]);
    }

    fn read_back(
        &self,
        mut encoder: wgpu::CommandEncoder,
        target: &wgpu::Texture,
    ) -> Result<Vec<u8>, RenderError> {
        let extent = target.size();
        let row_bytes = extent.width * 4;
        let padded_row_bytes = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
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
        self.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let failed = |error: &dyn std::fmt::Display| RenderError::Gpu(error.to_string());
        self.device
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
