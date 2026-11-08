use std::marker::PhantomData;
use std::slice;

use ash::vk;
use smithay::backend::renderer::sync::SyncPoint;
use smithay::backend::renderer::{Color32F, ContextId, Frame};
use smithay::utils::{Buffer as BufferCoords, Physical, Rectangle, Transform};

use smithay::reexports::rustix::ioctl;

use crate::{
    ATTACHMENT, BYTES_PER_PIXEL, COLOR_LAYERS, DeviceExport, Export, MappedExport, Program, Rect,
    RenderError, TRANSFER_SOURCE, VulkanFramebuffer, VulkanRenderer, VulkanTexture, barrier,
    disjoint,
};

mod blur;

pub use blur::Blur;
pub(crate) use blur::BlurChain;

pub(crate) const QUAD_BYTES: u32 = 112;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct RoundedRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
}

impl RoundedRect {
    pub fn components(self) -> [f32; 4] {
        [self.x, self.y, self.width, self.height]
    }

    pub fn translated(self, (dx, dy): (i32, i32)) -> Self {
        Self {
            x: self.x + dx as f32,
            y: self.y + dy as f32,
            ..self
        }
    }

    fn turned(self, turn: Transform, area: (i32, i32)) -> Self {
        if turn == Transform::Normal {
            return self;
        }
        let rect = turn.transform_rect_in(
            Rectangle::<f64, Physical>::new(
                (f64::from(self.x), f64::from(self.y)).into(),
                (f64::from(self.width), f64::from(self.height)).into(),
            ),
            &(f64::from(area.0), f64::from(area.1)).into(),
        );
        Self {
            x: rect.loc.x as f32,
            y: rect.loc.y as f32,
            width: rect.size.w as f32,
            height: rect.size.h as f32,
            radius: self.radius,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub caster: RoundedRect,
    pub window: RoundedRect,
    pub blur: f32,
    pub color: [f32; 4],
}

impl Shadow {
    pub fn bounds(&self) -> Rect {
        let reach = 3.0 * self.blur;
        let left = (self.caster.x - reach).floor();
        let top = (self.caster.y - reach).floor();
        let right = (self.caster.x + self.caster.width + reach).ceil();
        let bottom = (self.caster.y + self.caster.height + reach).ceil();
        Rect::new(
            left as i32,
            top as i32,
            (right - left) as i32,
            (bottom - top) as i32,
        )
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Sampling {
    origin: [f64; 2],
    across: [f64; 2],
    down: [f64; 2],
}

const UNIT_CORNERS: [(f64, f64); 4] = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)];

fn unit_point(transform: Transform, (x, y): (f64, f64)) -> (f64, f64) {
    let moved = transform.transform_rect_in(
        Rectangle::<f64, Physical>::new((x, y).into(), (0.0, 0.0).into()),
        &(1.0, 1.0).into(),
    );
    (moved.loc.x, moved.loc.y)
}

impl Sampling {
    fn upright(source: Rectangle<f64, BufferCoords>) -> Self {
        Self::oriented(source, Transform::Normal, Transform::Normal)
    }

    fn oriented(source: Rectangle<f64, BufferCoords>, buffer: Transform, turn: Transform) -> Self {
        let sample = |corner: (f64, f64)| {
            let shown = UNIT_CORNERS
                .into_iter()
                .find(|&unturned| unit_point(turn, unturned) == corner)
                .unwrap_or(corner);
            let (x, y) = unit_point(buffer.invert(), shown);
            [
                source.loc.x + x * source.size.w,
                source.loc.y + y * source.size.h,
            ]
        };
        let origin = sample((0.0, 0.0));
        let [across, down] = [sample((1.0, 0.0)), sample((0.0, 1.0))]
            .map(|corner| [corner[0] - origin[0], corner[1] - origin[1]]);
        Self {
            origin,
            across,
            down,
        }
    }
}

#[derive(Default)]
struct Quad {
    color: [f32; 4],
    destination: Rect,
    sampling: Sampling,
    alpha: f32,
    blur: f32,
    clip: RoundedRect,
    caster: RoundedRect,
}

pub struct VulkanFrame<'frame, 'buffer> {
    renderer: &'frame mut VulkanRenderer,
    target: u64,
    format: vk::Format,
    image: vk::Image,
    target_view: vk::ImageView,
    bounds: Rect,
    bound: Option<Program>,
    clip: Option<RoundedRect>,
    origin: (i32, i32),
    turn: Transform,
    area: (i32, i32),
    drawn: Vec<Rect>,
    finished: bool,
    _framebuffer: PhantomData<&'frame mut VulkanFramebuffer<'buffer>>,
}

fn physical(rect: &Rectangle<i32, Physical>) -> Rect {
    Rect::new(rect.loc.x, rect.loc.y, rect.size.w, rect.size.h)
}

fn scissor(rect: Rect) -> vk::Rect2D {
    vk::Rect2D {
        offset: vk::Offset2D {
            x: rect.x,
            y: rect.y,
        },
        extent: vk::Extent2D {
            width: rect.width.unsigned_abs(),
            height: rect.height.unsigned_abs(),
        },
    }
}

impl<'frame, 'buffer> VulkanFrame<'frame, 'buffer> {
    pub(crate) fn begin(
        renderer: &'frame mut VulkanRenderer,
        framebuffer: &'frame mut VulkanFramebuffer<'buffer>,
        turn: Transform,
    ) -> Result<Self, RenderError> {
        let target = renderer
            .targets
            .get(&framebuffer.id)
            .ok_or(RenderError::ForeignResource)?;
        let (image, view, format, rendered) = (
            target.image.image,
            target.image.view,
            target.format,
            target.rendered,
        );
        let bounds = Rect::new(0, 0, target.image.width, target.image.height);
        let area = turn
            .invert()
            .transform_size(smithay::utils::Size::<i32, Physical>::from((
                bounds.width,
                bounds.height,
            )));
        renderer.begin_commands()?;
        renderer.record_copies();
        let device = &renderer.device;
        let commands = renderer.command_buffer;
        let before = if rendered {
            ATTACHMENT
        } else {
            (
                vk::ImageLayout::UNDEFINED,
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags2::NONE,
            )
        };
        barrier(
            device,
            commands,
            image,
            before,
            (
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags2::COLOR_ATTACHMENT_READ | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            ),
        );
        let frame = Self {
            renderer,
            target: framebuffer.id,
            format,
            image,
            target_view: view,
            bounds,
            bound: None,
            clip: None,
            origin: (0, 0),
            turn,
            area: (area.w, area.h),
            drawn: Vec::new(),
            finished: false,
            _framebuffer: PhantomData,
        };
        frame.begin_rendering(view, bounds, vk::AttachmentLoadOp::LOAD);
        Ok(frame)
    }

    pub(crate) fn turned(&self, rect: Rect) -> Rect {
        if self.turn == Transform::Normal {
            return rect;
        }
        let rect = self.turn.transform_rect_in(
            Rectangle::<i32, Physical>::new(
                (rect.x, rect.y).into(),
                (rect.width, rect.height).into(),
            ),
            &self.area.into(),
        );
        Rect::new(rect.loc.x, rect.loc.y, rect.size.w, rect.size.h)
    }

    pub(crate) fn turned_shape(&self, shape: RoundedRect) -> RoundedRect {
        shape.turned(self.turn, self.area)
    }

    fn placed(&self, destination: Rect, damage: &[Rectangle<i32, Physical>]) -> (Rect, Vec<Rect>) {
        let rects = self.clip(
            damage
                .iter()
                .map(|rect| self.turned(physical(rect).translated(destination.x, destination.y))),
            self.turned(destination),
        );
        (self.turned(destination), rects)
    }

    fn clip(&self, rects: impl IntoIterator<Item = Rect>, within: Rect) -> Vec<Rect> {
        let Some(within) = within.intersection(&self.bounds) else {
            return Vec::new();
        };
        disjoint(rects, within)
    }

    fn fill(&mut self, color: Color32F, rects: &[Rect]) {
        let clear = vk::ClearAttachment {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            color_attachment: 0,
            clear_value: vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: color.components(),
                },
            },
        };
        for &rect in rects {
            unsafe {
                self.renderer.device.cmd_clear_attachments(
                    self.renderer.command_buffer,
                    slice::from_ref(&clear),
                    &[vk::ClearRect {
                        rect: scissor(rect),
                        base_array_layer: 0,
                        layer_count: 1,
                    }],
                );
            }
            self.painted(rect);
        }
    }

    fn painted(&mut self, rect: Rect) {
        self.renderer.counters.shaded_pixels += rect.area();
        self.drawn.push(rect);
    }

    pub fn clip_to(&mut self, clip: Option<RoundedRect>) {
        self.clip = clip.map(|clip| self.turned_shape(clip.translated(self.origin)));
    }

    pub fn move_origin(&mut self, by: (i32, i32)) {
        self.origin = by;
    }

    pub(crate) fn origin(&self) -> (i32, i32) {
        self.origin
    }

    pub fn draw_shadow(
        &mut self,
        shadow: &Shadow,
        damage: &[Rectangle<i32, Physical>],
    ) -> Result<(), RenderError> {
        let shadow = &Shadow {
            caster: shadow.caster.translated(self.origin),
            window: shadow.window.translated(self.origin),
            ..*shadow
        };
        let (_, rects) = self.placed(shadow.bounds(), damage);
        let shadow = &Shadow {
            caster: self.turned_shape(shadow.caster),
            window: self.turned_shape(shadow.window),
            ..*shadow
        };
        let destination = shadow.bounds();
        if rects.is_empty() {
            return Ok(());
        }
        self.bind(Program::Shadow)?;
        self.draw(
            &Quad {
                color: shadow.color,
                destination,
                alpha: 1.0,
                blur: shadow.blur,
                clip: shadow.window,
                caster: shadow.caster,
                ..Quad::default()
            },
            &rects,
        );
        Ok(())
    }

    fn bind(&mut self, program: Program) -> Result<(), RenderError> {
        if self.bound == Some(program) {
            return Ok(());
        }
        let pipeline = *self
            .renderer
            .pipelines
            .get(&(self.format, program))
            .ok_or(RenderError::UnsupportedFormat)?;
        unsafe {
            self.renderer.device.cmd_bind_pipeline(
                self.renderer.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline,
            );
        }
        self.bound = Some(program);
        Ok(())
    }

    fn draw(&mut self, quad: &Quad, rects: &[Rect]) {
        self.draw_quad(quad, rects);
        for &rect in rects {
            self.painted(rect);
        }
    }

    fn push_chroma(&mut self, view: vk::ImageView) {
        let image_info = vk::DescriptorImageInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let write = vk::WriteDescriptorSet::default()
            .dst_binding(2)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .image_info(slice::from_ref(&image_info));
        unsafe {
            self.renderer.push_descriptor.cmd_push_descriptor_set(
                self.renderer.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.renderer.pipeline_layout,
                0,
                slice::from_ref(&write),
            );
        }
    }

    fn push_texture(&mut self, view: vk::ImageView) {
        let image_info = vk::DescriptorImageInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let write = vk::WriteDescriptorSet::default()
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .image_info(slice::from_ref(&image_info));
        unsafe {
            self.renderer.push_descriptor.cmd_push_descriptor_set(
                self.renderer.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.renderer.pipeline_layout,
                0,
                slice::from_ref(&write),
            );
        }
    }

    fn draw_quad(&mut self, quad: &Quad, rects: &[Rect]) {
        let constants: Vec<u8> = quad
            .color
            .iter()
            .copied()
            .chain([
                self.bounds.width as f32,
                self.bounds.height as f32,
                quad.destination.x as f32,
                quad.destination.y as f32,
                quad.destination.width as f32,
                quad.destination.height as f32,
            ])
            .chain(
                [quad.sampling.origin, quad.sampling.across]
                    .into_iter()
                    .flatten()
                    .map(|component| component as f32),
            )
            .chain([quad.alpha, quad.blur])
            .chain(quad.clip.components())
            .chain(quad.caster.components())
            .chain([quad.clip.radius, quad.caster.radius])
            .chain(quad.sampling.down.map(|component| component as f32))
            .flat_map(f32::to_ne_bytes)
            .collect();
        let device = &self.renderer.device;
        let commands = self.renderer.command_buffer;
        unsafe {
            device.cmd_push_constants(
                commands,
                self.renderer.pipeline_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                &constants,
            );
        }
        for &rect in rects {
            unsafe {
                device.cmd_set_scissor(commands, 0, slice::from_ref(&scissor(rect)));
                device.cmd_draw(commands, 4, 1, 0, 0);
            }
        }
    }

    fn finish_frame(&mut self) -> Result<(), RenderError> {
        self.finished = true;
        unsafe {
            self.renderer
                .device
                .cmd_end_rendering(self.renderer.command_buffer)
        };
        let drawn = disjoint(std::mem::take(&mut self.drawn), self.bounds);
        let export = self
            .renderer
            .targets
            .get(&self.target)
            .and_then(|target| target.export.as_ref());
        let mut readbacks = Vec::new();
        if let Some(Export::OnDevice(device)) = export {
            let device = *device;
            self.record_output(&device, &drawn);
        } else if export.is_some() {
            for rect in drawn {
                let bytes = rect.area() as usize * BYTES_PER_PIXEL;
                let (index, offset) = self.renderer.reserve(bytes)?;
                readbacks.push(Readback {
                    rect,
                    staging: index,
                    offset,
                });
            }
            self.record_readbacks(&readbacks);
        }
        unsafe {
            self.renderer
                .device
                .end_command_buffer(self.renderer.command_buffer)
        }?;
        self.renderer.submit_and_wait()?;
        self.renderer.retire_unused_blur_chains();
        let written = self.write_back(&readbacks);
        self.renderer.recycle_staging();
        if let Some(target) = self.renderer.targets.get_mut(&self.target) {
            target.rendered = true;
        }
        written
    }

    fn record_output(&mut self, export: &DeviceExport, drawn: &[Rect]) {
        if drawn.is_empty() {
            return;
        }
        let Some(colour) = self.renderer.output_colour.clone() else {
            self.record_output_copy(export, self.image, drawn);
            barrier(
                &self.renderer.device,
                self.renderer.command_buffer,
                self.image,
                TRANSFER_SOURCE,
                ATTACHMENT,
            );
            return;
        };
        match self.convert_output(colour, drawn) {
            Ok(scratch) => {
                self.record_output_copy(export, scratch, drawn);
                barrier(
                    &self.renderer.device,
                    self.renderer.command_buffer,
                    self.image,
                    blur::SAMPLED,
                    ATTACHMENT,
                );
            }
            Err(error) => {
                tracing::warn!("showing colours unconverted: {error}");
                self.renderer.output_colour = None;
                self.record_output_copy(export, self.image, drawn);
                barrier(
                    &self.renderer.device,
                    self.renderer.command_buffer,
                    self.image,
                    TRANSFER_SOURCE,
                    ATTACHMENT,
                );
            }
        }
    }

    fn convert_output(
        &mut self,
        colour: crate::OutputColour,
        drawn: &[Rect],
    ) -> Result<vk::Image, RenderError> {
        let key = (self.bounds.width, self.bounds.height, self.format);
        if !self.renderer.colour_scratch.contains_key(&key) {
            let scratch = self.renderer.create_image(
                self.bounds.width,
                self.bounds.height,
                self.format,
                vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
                vk::ComponentSwizzle::IDENTITY,
            )?;
            self.renderer.colour_scratch.insert(key, scratch);
        }
        let scratch = self
            .renderer
            .colour_scratch
            .get(&key)
            .map(|scratch| (scratch.image, scratch.view))
            .ok_or(RenderError::ForeignResource)?;
        self.bound = None;
        self.bind(Program::DisplayColour)?;
        let device = &self.renderer.device;
        let commands = self.renderer.command_buffer;
        barrier(device, commands, self.image, ATTACHMENT, blur::SAMPLED);
        barrier(device, commands, scratch.0, blur::DISCARDED, blur::DRAWN);
        self.begin_rendering(scratch.1, self.bounds, vk::AttachmentLoadOp::DONT_CARE);
        self.push_texture(self.target_view);
        let table = self
            .renderer
            .encode_table
            .as_ref()
            .filter(|_| colour.encode.is_some())
            .and_then(|(_, texture)| self.renderer.textures.get(&texture.id))
            .map(|table| table.image.view);
        self.push_chroma(table.unwrap_or(self.target_view));
        let matrix = colour.matrix;
        let row = |values: [f32; 3]| RoundedRect {
            x: values[0],
            y: values[1],
            width: values[2],
            ..RoundedRect::default()
        };
        self.draw_quad(
            &Quad {
                color: [matrix[0][0], matrix[0][1], matrix[0][2], 0.0],
                destination: self.bounds,
                clip: row(matrix[1]),
                caster: row(matrix[2]),
                alpha: if table.is_some() { 1.0 } else { 0.0 },
                ..Quad::default()
            },
            drawn,
        );
        let device = &self.renderer.device;
        unsafe { device.cmd_end_rendering(commands) };
        barrier(device, commands, scratch.0, blur::DRAWN, TRANSFER_SOURCE);
        Ok(scratch.0)
    }

    fn record_output_copy(&self, export: &DeviceExport, source: vk::Image, drawn: &[Rect]) {
        let device = &self.renderer.device;
        let commands = self.renderer.command_buffer;
        let family = self.renderer.queue_family;
        let buffer = export.imported.buffer;
        let row_bytes = u64::from(export.row_texels) * BYTES_PER_PIXEL as u64;
        let regions: Vec<_> = drawn
            .iter()
            .map(|rect| {
                vk::BufferImageCopy::default()
                    .buffer_offset(
                        export.offset
                            + u64::from(rect.y.unsigned_abs()) * row_bytes
                            + u64::from(rect.x.unsigned_abs()) * BYTES_PER_PIXEL as u64,
                    )
                    .buffer_row_length(export.row_texels)
                    .image_subresource(COLOR_LAYERS)
                    .image_offset(vk::Offset3D {
                        x: rect.x,
                        y: rect.y,
                        z: 0,
                    })
                    .image_extent(vk::Extent3D {
                        width: rect.width.unsigned_abs(),
                        height: rect.height.unsigned_abs(),
                        depth: 1,
                    })
            })
            .collect();
        let acquire = vk::BufferMemoryBarrier2::default()
            .src_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
            .dst_queue_family_index(family)
            .dst_stage_mask(vk::PipelineStageFlags2::COPY)
            .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .buffer(buffer)
            .size(vk::WHOLE_SIZE);
        let release = vk::BufferMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .src_queue_family_index(family)
            .dst_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
            .buffer(buffer)
            .size(vk::WHOLE_SIZE);
        if source == self.image {
            barrier(device, commands, self.image, ATTACHMENT, TRANSFER_SOURCE);
        }
        unsafe {
            device.cmd_pipeline_barrier2(
                commands,
                &vk::DependencyInfo::default().buffer_memory_barriers(slice::from_ref(&acquire)),
            );
            device.cmd_copy_image_to_buffer(
                commands,
                source,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer,
                &regions,
            );
            device.cmd_pipeline_barrier2(
                commands,
                &vk::DependencyInfo::default().buffer_memory_barriers(slice::from_ref(&release)),
            );
        }
    }

    fn record_readbacks(&self, readbacks: &[Readback]) {
        if readbacks.is_empty() {
            return;
        }
        let device = &self.renderer.device;
        let commands = self.renderer.command_buffer;
        barrier(device, commands, self.image, ATTACHMENT, TRANSFER_SOURCE);
        for readback in readbacks {
            let rect = readback.rect;
            let region = vk::BufferImageCopy::default()
                .buffer_offset(readback.offset as u64)
                .image_subresource(COLOR_LAYERS)
                .image_offset(vk::Offset3D {
                    x: rect.x,
                    y: rect.y,
                    z: 0,
                })
                .image_extent(vk::Extent3D {
                    width: rect.width.unsigned_abs(),
                    height: rect.height.unsigned_abs(),
                    depth: 1,
                });
            unsafe {
                device.cmd_copy_image_to_buffer(
                    commands,
                    self.image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    self.renderer.staging[readback.staging].buffer,
                    slice::from_ref(&region),
                );
            }
        }
        barrier(device, commands, self.image, TRANSFER_SOURCE, ATTACHMENT);
        let host_read = vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::HOST)
            .dst_access_mask(vk::AccessFlags2::HOST_READ);
        unsafe {
            device.cmd_pipeline_barrier2(
                commands,
                &vk::DependencyInfo::default().memory_barriers(slice::from_ref(&host_read)),
            );
        }
    }

    fn write_back(&self, readbacks: &[Readback]) -> Result<(), RenderError> {
        let Some(export) = self
            .renderer
            .targets
            .get(&self.target)
            .and_then(|target| target.export.as_ref())
        else {
            return Ok(());
        };
        let Export::Mapped(export) = export else {
            return Ok(());
        };
        if readbacks.is_empty() {
            return Ok(());
        }
        dma_buf_sync(export, DMA_BUF_SYNC_START)?;
        let copied = self.copy_readbacks(export, readbacks);
        dma_buf_sync(export, DMA_BUF_SYNC_END)?;
        copied
    }

    fn copy_readbacks(
        &self,
        export: &MappedExport,
        readbacks: &[Readback],
    ) -> Result<(), RenderError> {
        for readback in readbacks {
            let rect = readback.rect;
            let row_bytes = rect.width.unsigned_abs() as usize * BYTES_PER_PIXEL;
            let source = self.renderer.staging[readback.staging].mapped;
            for row in 0..rect.height.unsigned_abs() as usize {
                let destination = export.offset
                    + (rect.y.unsigned_abs() as usize + row) * export.stride
                    + rect.x.unsigned_abs() as usize * BYTES_PER_PIXEL;
                if destination + row_bytes > export.length {
                    return Err(RenderError::BufferOutOfBounds);
                }
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        source.add(readback.offset + row * row_bytes),
                        export.mapping.add(destination),
                        row_bytes,
                    );
                }
            }
        }
        Ok(())
    }
}

struct Readback {
    rect: Rect,
    staging: usize,
    offset: usize,
}

const DMA_BUF_SYNC_WRITE: u64 = 2;
const DMA_BUF_SYNC_START: u64 = DMA_BUF_SYNC_WRITE;
const DMA_BUF_SYNC_END: u64 = 4 | DMA_BUF_SYNC_WRITE;

fn dma_buf_sync(export: &MappedExport, flags: u64) -> Result<(), RenderError> {
    const SYNC: ioctl::Opcode = ioctl::opcode::write::<u64>(b'b', 0);
    let request = unsafe { ioctl::Setter::<SYNC, u64>::new(flags) };
    unsafe { ioctl::ioctl(&export.fd, request) }.map_err(std::io::Error::from)?;
    Ok(())
}

impl Frame for VulkanFrame<'_, '_> {
    type Error = RenderError;
    type TextureId = VulkanTexture;

    fn context_id(&self) -> ContextId<VulkanTexture> {
        self.renderer.context.clone()
    }

    fn clear(
        &mut self,
        color: Color32F,
        at: &[Rectangle<i32, Physical>],
    ) -> Result<(), RenderError> {
        let rects = self.clip(
            at.iter().map(|rect| self.turned(physical(rect))),
            self.bounds,
        );
        self.fill(color, &rects);
        Ok(())
    }

    fn draw_solid(
        &mut self,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        color: Color32F,
    ) -> Result<(), RenderError> {
        let (destination, rects) = self.placed(physical(&dst), damage);
        if color.is_opaque() && self.clip.is_none() {
            self.fill(color, &rects);
            return Ok(());
        }
        self.bind(Program::Solid)?;
        self.draw(
            &Quad {
                color: color.components(),
                destination,
                alpha: 1.0,
                clip: self.clip.unwrap_or_default(),
                ..Quad::default()
            },
            &rects,
        );
        Ok(())
    }

    fn render_texture_from_to(
        &mut self,
        texture: &VulkanTexture,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        src_transform: Transform,
        alpha: f32,
    ) -> Result<(), RenderError> {
        if texture.renderer != self.renderer.id {
            return Err(RenderError::ForeignResource);
        }
        let Some(view) = self
            .renderer
            .textures
            .get(&texture.id)
            .filter(|image| image.readable)
            .map(|image| image.image.view)
        else {
            return Ok(());
        };
        let (destination, rects) = self.placed(physical(&dst), damage);
        if rects.is_empty() {
            return Ok(());
        }
        let chroma = texture.chroma.as_ref().and_then(|chroma| {
            self.renderer
                .textures
                .get(&chroma.id)
                .filter(|image| image.readable)
                .map(|image| image.image.view)
        });
        match (texture.chroma.is_some(), chroma) {
            (true, None) => return Ok(()),
            (true, Some(chroma)) => {
                self.bind(Program::Video)?;
                self.push_chroma(chroma);
            }
            (false, _) => self.bind(Program::Texture)?,
        }
        self.push_texture(view);
        self.draw(
            &Quad {
                destination,
                sampling: Sampling::oriented(src, src_transform, self.turn),
                alpha,
                clip: self.clip.unwrap_or_default(),
                ..Quad::default()
            },
            &rects,
        );
        Ok(())
    }

    fn transformation(&self) -> Transform {
        self.turn
    }

    fn wait(&mut self, sync: &SyncPoint) -> Result<(), RenderError> {
        sync.wait().map_err(|_| RenderError::Interrupted)
    }

    fn finish(mut self) -> Result<SyncPoint, RenderError> {
        self.finish_frame()?;
        Ok(SyncPoint::signaled())
    }
}

impl Drop for VulkanFrame<'_, '_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.finish_frame();
        }
    }
}
