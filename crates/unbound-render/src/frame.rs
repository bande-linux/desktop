use std::marker::PhantomData;
use std::slice;

use ash::vk;
use smithay::backend::renderer::sync::SyncPoint;
use smithay::backend::renderer::{Color32F, ContextId, Frame};
use smithay::utils::{Buffer as BufferCoords, Physical, Rectangle, Transform};

use crate::{
    ATTACHMENT, COLOR_LAYERS, Rect, RenderError, TRANSFER_SOURCE, VulkanFramebuffer,
    VulkanRenderer, VulkanTexture, barrier, disjoint,
};

pub(crate) const QUAD_BYTES: u32 = 64;

struct Quad {
    color: [f32; 4],
    destination: Rect,
    source: Rectangle<f64, BufferCoords>,
    alpha: f32,
}

pub struct VulkanFrame<'frame, 'buffer> {
    renderer: &'frame mut VulkanRenderer,
    target: u64,
    format: vk::Format,
    image: vk::Image,
    bounds: Rect,
    bound: Option<bool>,
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
        let attachment = vk::RenderingAttachmentInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::LOAD)
            .store_op(vk::AttachmentStoreOp::STORE);
        unsafe {
            device.cmd_begin_rendering(
                commands,
                &vk::RenderingInfo::default()
                    .render_area(scissor(bounds))
                    .layer_count(1)
                    .color_attachments(slice::from_ref(&attachment)),
            );
            device.cmd_set_viewport(
                commands,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: bounds.width as f32,
                    height: bounds.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
        }
        Ok(Self {
            renderer,
            target: framebuffer.id,
            format,
            image,
            bounds,
            bound: None,
            drawn: Vec::new(),
            finished: false,
            _framebuffer: PhantomData,
        })
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

    fn bind(&mut self, solid: bool) -> Result<(), RenderError> {
        if self.bound == Some(solid) {
            return Ok(());
        }
        let pipeline = *self
            .renderer
            .pipelines
            .get(&(self.format, solid))
            .ok_or(RenderError::UnsupportedFormat)?;
        unsafe {
            self.renderer.device.cmd_bind_pipeline(
                self.renderer.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline,
            );
        }
        self.bound = Some(solid);
        Ok(())
    }

    fn draw(&mut self, quad: &Quad, rects: &[Rect]) {
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
                quad.source.loc.x as f32,
                quad.source.loc.y as f32,
                quad.source.size.w as f32,
                quad.source.size.h as f32,
                quad.alpha,
                0.0,
            ])
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
        for &rect in rects {
            self.painted(rect);
        }
    }

    fn finish_frame(&mut self) -> Result<(), RenderError> {
        self.finished = true;
        let device = &self.renderer.device;
        let commands = self.renderer.command_buffer;
        unsafe { device.cmd_end_rendering(commands) };
        let drawn = disjoint(std::mem::take(&mut self.drawn), self.bounds);
        if let Some(export) = self
            .renderer
            .targets
            .get(&self.target)
            .and_then(|target| target.export.as_ref())
            && !drawn.is_empty()
        {
            let regions: Vec<vk::BufferImageCopy> = drawn
                .iter()
                .map(|rect| {
                    let first_texel = u64::from(rect.y.unsigned_abs())
                        * u64::from(export.row_texels)
                        + u64::from(rect.x.unsigned_abs());
                    vk::BufferImageCopy::default()
                        .buffer_offset(export.offset + first_texel * 4)
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
            barrier(device, commands, self.image, ATTACHMENT, TRANSFER_SOURCE);
            unsafe {
                device.cmd_copy_image_to_buffer(
                    commands,
                    self.image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    export.imported.buffer,
                    &regions,
                );
            }
            barrier(device, commands, self.image, TRANSFER_SOURCE, ATTACHMENT);
        }
        unsafe { device.end_command_buffer(commands) }?;
        self.renderer.submit_and_wait()?;
        self.renderer.recycle_staging();
        if let Some(target) = self.renderer.targets.get_mut(&self.target) {
            target.rendered = true;
        }
        Ok(())
    }
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
        let rects = self.clip(at.iter().map(physical), self.bounds);
        self.fill(color, &rects);
        Ok(())
    }

    fn draw_solid(
        &mut self,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        color: Color32F,
    ) -> Result<(), RenderError> {
        let destination = physical(&dst);
        let rects = self.clip(
            damage
                .iter()
                .map(|rect| physical(rect).translated(destination.x, destination.y)),
            destination,
        );
        if color.is_opaque() {
            self.fill(color, &rects);
            return Ok(());
        }
        self.bind(true)?;
        self.draw(
            &Quad {
                color: color.components(),
                destination,
                source: Rectangle::default(),
                alpha: 1.0,
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
        if src_transform != Transform::Normal {
            return Err(RenderError::UnsupportedTransform);
        }
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
        let destination = physical(&dst);
        let rects = self.clip(
            damage
                .iter()
                .map(|rect| physical(rect).translated(destination.x, destination.y)),
            destination,
        );
        if rects.is_empty() {
            return Ok(());
        }
        self.bind(false)?;
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
        self.draw(
            &Quad {
                color: [0.0; 4],
                destination,
                source: src,
                alpha,
            },
            &rects,
        );
        Ok(())
    }

    fn transformation(&self) -> Transform {
        Transform::Normal
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
