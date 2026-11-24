use std::slice;
use std::time::{Duration, Instant};

use ash::vk;
use smithay::utils::{Physical, Rectangle};

use super::{Quad, RoundedRect, Sampling, VulkanFrame, scissor};
use crate::{
    ATTACHMENT, COLOR_LAYERS, DRAWING, Image, Program, Rect, RenderError, SAMPLED, TRANSFER_SOURCE,
    Usage, VulkanRenderer, barrier,
};

const MAX_PASSES: u32 = 6;
pub(crate) const KEEP_UNUSED: Duration = Duration::from_secs(2);

type Level = (vk::Image, vk::ImageView, i32, i32);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blur {
    pub region: RoundedRect,
    pub passes: u32,
    pub offset: f32,
    pub alpha: f32,
    pub tone: Option<[f32; 4]>,
}

/// What a blur samples, from [`VulkanRenderer::backdrop`]. While `drawing` is the same, what was
/// drawn for it is kept and any part may be redrawn; a new `drawing` is redrawn whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backdrop {
    owner: u64,
    below: u64,
    pub drawing: u64,
}

pub(crate) struct BlurChain {
    pub(crate) levels: Vec<Image>,
    pub(crate) last_used: Instant,
    source: Rect,
    format: vk::Format,
    drawn: bool,
    backdrop: Option<Backdrop>,
}

impl VulkanRenderer {
    pub fn backdrop(&mut self, owner: u64, below: u64) -> Backdrop {
        let drawing = self.next_id();
        let kept = self.blur_chains.get_mut(&owner).and_then(|chain| {
            chain.last_used = Instant::now();
            chain.backdrop.filter(|kept| kept.below == below)
        });
        kept.unwrap_or(Backdrop {
            owner,
            below,
            drawing,
        })
    }
}

const COPY_DESTINATION: Usage = (
    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
    vk::PipelineStageFlags2::COPY,
    vk::AccessFlags2::TRANSFER_WRITE,
);

pub(super) const DISCARDED: Usage = (
    vk::ImageLayout::UNDEFINED,
    vk::PipelineStageFlags2::TOP_OF_PIPE,
    vk::AccessFlags2::NONE,
);

pub(super) const DRAWN: Usage = (
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
        backdrop: Backdrop,
        damage: &[Rectangle<i32, Physical>],
    ) -> Result<(), RenderError> {
        let blur = &Blur {
            region: blur.region.translated(self.origin()),
            ..*blur
        };
        let (_, rects) = self.placed(blur.bounds(), damage);
        let blur = &Blur {
            region: self.turned_shape(blur.region),
            ..*blur
        };
        let destination = blur.bounds();

        let Some(source) = destination.intersection(&self.bounds) else {
            return Ok(());
        };
        if rects.is_empty() {
            return Ok(());
        }
        let passes = blur.passes();
        let key = backdrop.owner;
        let (levels, prior, kept) = self.blur_levels(backdrop, source, passes)?;
        if !kept {
            for program in [Program::BlurDown, Program::BlurUp] {
                self.bind(program)?;
            }
            for level in &levels {
                self.renderer.descriptor_set(level.1, level.1)?;
            }
            self.pause();
            self.copy_into(source, levels[0], prior);
            for pass in 0..passes as usize {
                self.filter(
                    Program::BlurDown,
                    levels[pass],
                    (levels[pass + 1], prior),
                    blur.offset,
                )?;
            }
            for pass in (0..passes as usize).rev() {
                self.filter(
                    Program::BlurUp,
                    levels[pass + 1],
                    (levels[pass], SAMPLED),
                    blur.offset,
                )?;
            }
            self.resume();
            if let Some(chain) = self.renderer.blur_chains.get_mut(&key) {
                chain.drawn = true;
                chain.source = source;
                chain.backdrop = Some(backdrop);
            }
        }
        self.bind(match blur.tone {
            Some(_) => Program::BlurTone,
            None => Program::Texture,
        })?;
        self.sample(levels[0].1, levels[0].1)?;
        self.draw(
            &Quad {
                color: blur.tone.unwrap_or_default(),
                destination: source,
                sampling: Sampling::upright(Rectangle::from_size(
                    (f64::from(source.width), f64::from(source.height)).into(),
                )),
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
        backdrop: Backdrop,
        source: Rect,
        passes: u32,
    ) -> Result<(Vec<Level>, Usage, bool), RenderError> {
        let key = backdrop.owner;
        let fits = self.renderer.blur_chains.get(&key).is_some_and(|chain| {
            (chain.source.width, chain.source.height) == (source.width, source.height)
                && chain.format == self.format
                && chain.levels.len() == passes as usize + 1
        });
        if !fits {
            if let Some(chain) = self.renderer.blur_chains.remove(&key) {
                chain
                    .levels
                    .iter()
                    .for_each(|image| self.renderer.destroy_image(image));
            }
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
                    last_used: Instant::now(),
                    source,
                    format: self.format,
                    drawn: false,
                    backdrop: None,
                },
            );
        }
        let chain = self
            .renderer
            .blur_chains
            .get_mut(&key)
            .ok_or(RenderError::ForeignResource)?;
        chain.last_used = Instant::now();
        let kept = chain.backdrop == Some(backdrop) && chain.source == source;
        let prior = match chain.drawn {
            true => SAMPLED,
            false => DISCARDED,
        };
        let levels = chain
            .levels
            .iter()
            .map(|image| (image.image, image.view, image.width, image.height))
            .collect();
        Ok((levels, prior, kept))
    }

    pub(super) fn pause(&mut self) {
        // SAFETY: the frame holds the renderer mutably while `command_buffer` records, and the
        // rendering on the target is open; `resume` begins it again.
        unsafe {
            self.renderer
                .device
                .cmd_end_rendering(self.renderer.command_buffer)
        };
    }

    pub(super) fn resume(&mut self) {
        self.begin_rendering(self.target_view, self.bounds, vk::AttachmentLoadOp::LOAD);
        self.bound = None;
    }

    fn copy_into(&mut self, source: Rect, level: Level, prior: Usage) {
        let device = &self.renderer.device;
        let commands = self.renderer.command_buffer;
        barrier(device, commands, self.image, ATTACHMENT, TRANSFER_SOURCE);
        barrier(device, commands, level.0, prior, COPY_DESTINATION);
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
        // SAFETY: the frame holds the renderer mutably while `command_buffer` records; the barriers
        // above moved both live images to the copy layouts, and `source` lies within the target
        // while level 0 has exactly its size.
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
        barrier(device, commands, self.image, TRANSFER_SOURCE, DRAWING);
        barrier(device, commands, level.0, COPY_DESTINATION, SAMPLED);
        self.renderer.counters.shaded_pixels += source.area();
    }

    fn filter(
        &mut self,
        program: Program,
        from: Level,
        (to, prior): (Level, Usage),
        offset: f32,
    ) -> Result<(), RenderError> {
        let bounds = Rect::new(0, 0, to.2, to.3);
        barrier(
            &self.renderer.device,
            self.renderer.command_buffer,
            to.0,
            prior,
            DRAWN,
        );
        self.begin_rendering(to.1, bounds, vk::AttachmentLoadOp::DONT_CARE);
        self.bound = None;
        self.bind(program)?;
        self.sample(from.1, from.1)?;
        let target = self.bounds;
        self.bounds = bounds;
        self.draw_quad(
            &Quad {
                destination: bounds,
                sampling: Sampling::upright(Rectangle::from_size(
                    (f64::from(from.2), f64::from(from.3)).into(),
                )),
                blur: offset,
                alpha: 1.0,
                ..Quad::default()
            },
            &[bounds],
        );
        self.bounds = target;
        self.renderer.counters.shaded_pixels += bounds.area();
        // SAFETY: the frame holds the renderer mutably while `command_buffer` records, and this
        // ends the rendering begun on `to` above.
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
            .store_op(vk::AttachmentStoreOp::STORE);
        let device = &self.renderer.device;
        let commands = self.renderer.command_buffer;
        // SAFETY: the frame holds the renderer mutably while `command_buffer` records, outside any
        // rendering; `view` is a live view in `COLOR_ATTACHMENT_OPTIMAL` and `attachment` outlives
        // the call.
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
