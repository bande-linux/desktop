use std::slice;

use ash::vk;
use smithay::utils::{Physical, Rectangle};

use super::{Quad, RoundedRect, VulkanFrame, physical, scissor};
use crate::{
    ATTACHMENT, COLOR_LAYERS, Image, Program, Rect, RenderError, TRANSFER_SOURCE, Usage, barrier,
};

const MAX_PASSES: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blur {
    pub region: RoundedRect,
    pub passes: u32,
    pub offset: f32,
    pub alpha: f32,
}

pub(crate) struct BlurChain {
    pub(crate) levels: Vec<Image>,
    pub(crate) used: bool,
}

const SAMPLED: Usage = (
    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
    vk::PipelineStageFlags2::FRAGMENT_SHADER,
    vk::AccessFlags2::SHADER_SAMPLED_READ,
);

const COPY_DESTINATION: Usage = (
    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
    vk::PipelineStageFlags2::COPY,
    vk::AccessFlags2::TRANSFER_WRITE,
);

const DISCARDED: Usage = (
    vk::ImageLayout::UNDEFINED,
    vk::PipelineStageFlags2::TOP_OF_PIPE,
    vk::AccessFlags2::NONE,
);

const DRAWN: Usage = (
    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
    vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
);

impl Blur {
    fn passes(&self) -> u32 {
        self.passes.clamp(1, MAX_PASSES)
    }

    pub fn reach(&self) -> i32 {
        (self.offset.max(0.0) * 2.0 * (1u32 << self.passes()) as f32).ceil() as i32
    }

    pub fn bounds(&self) -> Rect {
        let reach = self.reach();
        let left = self.region.x.floor() as i32 - reach;
        let top = self.region.y.floor() as i32 - reach;
        let right = (self.region.x + self.region.width).ceil() as i32 + reach;
        let bottom = (self.region.y + self.region.height).ceil() as i32 + reach;
        Rect::new(left, top, right - left, bottom - top)
    }
}

impl VulkanFrame<'_, '_> {
    pub fn draw_blur(
        &mut self,
        blur: &Blur,
        damage: &[Rectangle<i32, Physical>],
    ) -> Result<(), RenderError> {
        let blur = &Blur {
            region: blur.region.translated(self.origin()),
            ..*blur
        };
        let destination = blur.bounds();
        let rects = self.clip(
            damage
                .iter()
                .map(|rect| physical(rect).translated(destination.x, destination.y)),
            destination,
        );
        let Some(source) = destination.intersection(&self.bounds) else {
            return Ok(());
        };
        if rects.is_empty() {
            return Ok(());
        }
        let passes = blur.passes();
        let levels = self.blur_levels(source, passes)?;
        self.pause();
        self.copy_into(source, levels[0]);
        for pass in 0..passes as usize {
            self.filter(
                Program::BlurDown,
                levels[pass],
                levels[pass + 1],
                blur.offset,
            )?;
        }
        for pass in (0..passes as usize).rev() {
            self.filter(Program::BlurUp, levels[pass + 1], levels[pass], blur.offset)?;
        }
        self.resume();
        self.bind(Program::Texture)?;
        self.push_texture(levels[0].1);
        self.draw(
            &Quad {
                destination: source,
                source: Rectangle::from_size(
                    (f64::from(source.width), f64::from(source.height)).into(),
                ),
                alpha: blur.alpha,
                clip: blur.region,
                ..Quad::default()
            },
            &rects,
        );
        Ok(())
    }

    fn blur_levels(
        &mut self,
        source: Rect,
        passes: u32,
    ) -> Result<Vec<(vk::Image, vk::ImageView, i32, i32)>, RenderError> {
        let key = (source.width, source.height, self.format, passes);
        if !self.renderer.blur_chains.contains_key(&key) {
            let mut levels = Vec::new();
            for level in 0..=passes {
                let created = self.renderer.create_image(
                    (source.width >> level).max(1),
                    (source.height >> level).max(1),
                    self.format,
                    vk::ImageUsageFlags::COLOR_ATTACHMENT
                        | vk::ImageUsageFlags::SAMPLED
                        | vk::ImageUsageFlags::TRANSFER_DST,
                    vk::ComponentSwizzle::IDENTITY,
                );
                match created {
                    Ok(image) => levels.push(image),
                    Err(error) => {
                        levels
                            .iter()
                            .for_each(|image| self.renderer.destroy_image(image));
                        return Err(error);
                    }
                }
            }
            self.renderer.blur_chains.insert(
                key,
                BlurChain {
                    levels,
                    used: false,
                },
            );
        }
        let chain = self
            .renderer
            .blur_chains
            .get_mut(&key)
            .ok_or(RenderError::ForeignResource)?;
        chain.used = true;
        Ok(chain
            .levels
            .iter()
            .map(|image| (image.image, image.view, image.width, image.height))
            .collect())
    }

    fn pause(&mut self) {
        unsafe {
            self.renderer
                .device
                .cmd_end_rendering(self.renderer.command_buffer)
        };
    }

    fn resume(&mut self) {
        self.begin_rendering(self.target_view, self.bounds, vk::AttachmentLoadOp::LOAD);
        self.bound = None;
    }

    fn copy_into(&mut self, source: Rect, level: (vk::Image, vk::ImageView, i32, i32)) {
        let device = &self.renderer.device;
        let commands = self.renderer.command_buffer;
        barrier(device, commands, self.image, ATTACHMENT, TRANSFER_SOURCE);
        barrier(device, commands, level.0, DISCARDED, COPY_DESTINATION);
        let region = vk::ImageCopy::default()
            .src_subresource(COLOR_LAYERS)
            .src_offset(vk::Offset3D {
                x: source.x,
                y: source.y,
                z: 0,
            })
            .dst_subresource(COLOR_LAYERS)
            .extent(vk::Extent3D {
                width: source.width.unsigned_abs(),
                height: source.height.unsigned_abs(),
                depth: 1,
            });
        unsafe {
            device.cmd_copy_image(
                commands,
                self.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                level.0,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                slice::from_ref(&region),
            );
        }
        barrier(device, commands, self.image, TRANSFER_SOURCE, ATTACHMENT);
        barrier(device, commands, level.0, COPY_DESTINATION, SAMPLED);
        self.renderer.counters.shaded_pixels += source.area();
    }

    fn filter(
        &mut self,
        program: Program,
        from: (vk::Image, vk::ImageView, i32, i32),
        to: (vk::Image, vk::ImageView, i32, i32),
        offset: f32,
    ) -> Result<(), RenderError> {
        let bounds = Rect::new(0, 0, to.2, to.3);
        barrier(
            &self.renderer.device,
            self.renderer.command_buffer,
            to.0,
            DISCARDED,
            DRAWN,
        );
        self.begin_rendering(to.1, bounds, vk::AttachmentLoadOp::CLEAR);
        self.bound = None;
        self.bind(program)?;
        self.push_texture(from.1);
        let target = self.bounds;
        self.bounds = bounds;
        self.draw_quad(
            &Quad {
                destination: bounds,
                source: Rectangle::from_size((f64::from(from.2), f64::from(from.3)).into()),
                blur: offset,
                alpha: 1.0,
                ..Quad::default()
            },
            &[bounds],
        );
        self.bounds = target;
        self.renderer.counters.shaded_pixels += bounds.area();
        unsafe {
            self.renderer
                .device
                .cmd_end_rendering(self.renderer.command_buffer)
        };
        barrier(
            &self.renderer.device,
            self.renderer.command_buffer,
            to.0,
            DRAWN,
            SAMPLED,
        );
        Ok(())
    }

    pub(super) fn begin_rendering(
        &self,
        view: vk::ImageView,
        bounds: Rect,
        load: vk::AttachmentLoadOp,
    ) {
        let attachment = vk::RenderingAttachmentInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(load)
            .clear_value(vk::ClearValue {
                color: vk::ClearColorValue { float32: [0.0; 4] },
            })
            .store_op(vk::AttachmentStoreOp::STORE);
        let device = &self.renderer.device;
        let commands = self.renderer.command_buffer;
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
    }
}
