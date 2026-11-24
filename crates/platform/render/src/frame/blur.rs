use std::slice;
use std::time::{Duration, Instant};

use ash::vk;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Physical, Rectangle};

use super::{Quad, RoundedRect, Sampling, VulkanFrame, scissor};
use crate::{
    ATTACHMENT, COLOR_LAYERS, DRAWING, Image, Program, Rect, RenderError, SAMPLED, TRANSFER_SOURCE,
    Usage, VulkanRenderer, barrier, disjoint,
};

const MAX_PASSES: u32 = 6;
const MAX_PARTS: usize = 64;
const SMALLEST_PARTLY_REDRAWN_LEVEL: i32 = 8;
pub(crate) const KEEP_UNUSED: Duration = Duration::from_secs(2);

type Level = (vk::Image, vk::ImageView, i32, i32);

struct Levels {
    levels: Vec<Level>,
    raised: Vec<Level>,
    prior: Usage,
    raised_prior: Usage,
    drawing: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blur {
    pub region: RoundedRect,
    pub passes: u32,
    pub offset: f32,
    pub alpha: f32,
    pub tone: Option<[f32; 4]>,
}

/// Something drawn beneath a blur: `look` is everything besides its content and place that
/// decides how it is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: Id,
    pub commit: CommitCounter,
    pub area: Rectangle<i32, Physical>,
    pub look: u64,
}

/// What a blur samples, from [`VulkanRenderer::backdrop`]. While `drawing` is the same, what was
/// drawn for it is kept and any part may be redrawn. A new `drawing` is redrawn whole, unless
/// `change` says in which parts it differs from what is kept.
#[derive(Debug, Clone)]
pub struct Backdrop {
    owner: u64,
    below: u64,
    layers: Vec<Layer>,
    stale: Vec<Rectangle<i32, Physical>>,
    pub drawing: u64,
    pub change: Change,
}

/// How a backdrop differs from what is kept of it. The parts are relative to the blur's bounds and
/// must be drawn again beneath it before it is.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Nothing,
    Parts {
        since: u64,
        parts: Vec<Rectangle<i32, Physical>>,
    },
    Everything {
        kept_for_parts: bool,
    },
}

pub(crate) struct BlurChain {
    levels: Vec<Image>,
    raised: Vec<Image>,
    raised_drawn: bool,
    pub(crate) last_used: Instant,
    source: Rect,
    format: vk::Format,
    drawn: bool,
    backdrop: Option<Backdrop>,
}

impl BlurChain {
    pub(crate) fn images(&self) -> impl Iterator<Item = &Image> + Clone {
        self.levels.iter().chain(&self.raised)
    }

    fn takes_parts(&self) -> bool {
        self.levels.last().is_some_and(|smallest| {
            smallest.width.min(smallest.height) >= SMALLEST_PARTLY_REDRAWN_LEVEL
        })
    }
}

impl VulkanRenderer {
    /// The backdrop of `blur`, with `layers` drawn beneath it except where they are `covered`.
    /// `damage` says which parts of the layer at an index changed since a commit of it, and `below`
    /// stands for everything else the backdrop depends on.
    pub fn backdrop(
        &mut self,
        (owner, blur): (u64, &Blur),
        below: u64,
        (layers, covered): (Vec<Layer>, &[Rectangle<i32, Physical>]),
        damage: impl Fn(usize, CommitCounter) -> Vec<Rectangle<i32, Physical>>,
    ) -> Backdrop {
        let drawing = self.next_id();
        let bounds = blur.area();
        let covered: Vec<Rectangle<i32, Physical>> = covered
            .iter()
            .filter_map(|rect| rect.intersection(bounds))
            .map(|rect| Rectangle::new(rect.loc - bounds.loc, rect.size))
            .collect();
        let whole = Backdrop {
            owner,
            below,
            layers,
            stale: covered.clone(),
            drawing,
            change: Change::Everything {
                kept_for_parts: false,
            },
        };
        let Some(chain) = self.blur_chains.get_mut(&owner) else {
            return whole;
        };
        chain.last_used = Instant::now();
        let divisible = chain.takes_parts();
        let separate = !chain.raised.is_empty();
        let Some(kept) = chain.backdrop.as_mut().filter(|kept| kept.below == below) else {
            return whole;
        };
        let Some(mut changed) = changed_layers(&kept.layers, &whole.layers, damage) else {
            return whole;
        };
        changed = changed
            .into_iter()
            .filter_map(|part| part.intersection(bounds))
            .map(|part| Rectangle::new(part.loc - bounds.loc, part.size))
            .chain(kept.stale.iter().copied())
            .collect();
        let seen = Rectangle::subtract_rects_many(changed.iter().copied(), covered.iter().copied());
        if changed.len().max(covered.len()).max(seen.len()) > MAX_PARTS {
            return whole;
        }
        let parts = gathered(seen, blur.spread());
        let stale = overlaps(changed.iter().chain(&parts), &covered);
        if parts.is_empty() {
            kept.layers.clone_from(&whole.layers);
            kept.stale.clone_from(&stale);
            return Backdrop {
                stale,
                drawing: kept.drawing,
                change: Change::Nothing,
                ..whole
            };
        }
        if !divisible || !blur.mostly_unchanged(&parts) {
            return whole;
        }
        if !separate {
            return Backdrop {
                change: Change::Everything {
                    kept_for_parts: true,
                },
                ..whole
            };
        }
        Backdrop {
            stale,
            change: Change::Parts {
                since: kept.drawing,
                parts,
            },
            ..whole
        }
    }
}

fn changed_layers(
    kept: &[Layer],
    layers: &[Layer],
    damage: impl Fn(usize, CommitCounter) -> Vec<Rectangle<i32, Physical>>,
) -> Option<Vec<Rectangle<i32, Physical>>> {
    let same = kept.len() == layers.len()
        && kept
            .iter()
            .zip(layers)
            .all(|(kept, layer)| kept.id == layer.id);
    same.then(|| {
        kept.iter()
            .zip(layers)
            .enumerate()
            .flat_map(|(index, (kept, layer))| {
                if (kept.area, kept.look) != (layer.area, layer.look) {
                    vec![kept.area, layer.area]
                } else if kept.commit != layer.commit {
                    damage(index, kept.commit)
                } else {
                    Vec::new()
                }
            })
            .collect()
    })
}

fn gathered(parts: Vec<Rectangle<i32, Physical>>, spread: i32) -> Vec<Rectangle<i32, Physical>> {
    let redrawn = |part: &Rectangle<i32, Physical>| {
        i64::from(part.size.w + 2 * spread) * i64::from(part.size.h + 2 * spread)
    };
    let mut gathered: Vec<Rectangle<i32, Physical>> = Vec::new();
    for mut part in parts {
        while let Some(index) = gathered
            .iter()
            .position(|other| redrawn(&other.merge(part)) <= redrawn(other) + redrawn(&part))
        {
            part = part.merge(gathered.swap_remove(index));
        }
        gathered.push(part);
    }
    gathered
}

fn overlaps<'a>(
    parts: impl Iterator<Item = &'a Rectangle<i32, Physical>>,
    covered: &[Rectangle<i32, Physical>],
) -> Vec<Rectangle<i32, Physical>> {
    let mut overlaps: Vec<Rectangle<i32, Physical>> = Vec::new();
    for overlap in parts.flat_map(|part| covered.iter().filter_map(|rect| rect.intersection(*part)))
    {
        let new = overlap.subtract_rects(overlaps.iter().copied());
        overlaps.extend(new);
    }
    overlaps
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

    /// How far from a change of the backdrop the blurred picture may differ.
    pub fn spread(&self) -> i32 {
        let texels = 1.5 * self.offset.max(0.0) + 3.0;
        (texels * (1u32 << self.passes()) as f32).ceil() as i32
    }

    fn area(&self) -> Rectangle<i32, Physical> {
        let bounds = self.bounds();
        Rectangle::new(
            (bounds.x, bounds.y).into(),
            (bounds.width, bounds.height).into(),
        )
    }

    fn mostly_unchanged(&self, parts: &[Rectangle<i32, Physical>]) -> bool {
        let spread = self.spread();
        let redrawn: u64 = parts
            .iter()
            .map(|part| Rect::new(0, 0, part.size.w + 2 * spread, part.size.h + 2 * spread).area())
            .sum();
        redrawn * 2 <= self.bounds().area()
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

fn widened(part: Rect, spread: i32, source: Rect, size: (i32, i32)) -> Rect {
    let along = |from: i32, length: i32, whole: i32, level: i32| {
        let shrunk = |at: i32| f64::from(at) * f64::from(level) / f64::from(whole);
        let start = shrunk(from - spread).floor() as i32;
        let end = shrunk(from + length + spread).ceil() as i32;
        (start, end - start)
    };
    let (x, width) = along(part.x - source.x, part.width, source.width, size.0);
    let (y, height) = along(part.y - source.y, part.height, source.height, size.1);
    Rect::new(x, y, width, height)
}

impl VulkanFrame<'_, '_> {
    pub fn draw_blur(
        &mut self,
        blur: &Blur,
        backdrop: &Backdrop,
        damage: &[Rectangle<i32, Physical>],
    ) -> Result<(), RenderError> {
        let blur = &Blur {
            region: blur.region.translated(self.origin()),
            ..*blur
        };
        let (_, rects) = self.placed(blur.bounds(), damage);
        let parts: Vec<Rect> = match &backdrop.change {
            Change::Parts { parts, .. } => parts
                .iter()
                .flat_map(|part| self.placed(blur.bounds(), slice::from_ref(part)).1)
                .collect(),
            Change::Nothing | Change::Everything { .. } => Vec::new(),
        };
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
        let chain = self.blur_levels(backdrop, source, passes)?;
        let kept = chain.drawing.is_some_and(|drawing| match backdrop.change {
            Change::Parts { since, .. } => drawing == since,
            _ => drawing == backdrop.drawing,
        });
        let redrawn = match (&backdrop.change, kept) {
            (Change::Parts { .. }, true) => Some(parts),
            (_, true) => None,
            (_, false) => Some(vec![source]),
        };
        if let Some(changed) = redrawn {
            let complete = kept || matches!(backdrop.change, Change::Everything { .. });
            if !changed.is_empty() {
                self.blur(blur, &chain, source, &changed, kept)?;
            }
            if let Some(chain) = self.renderer.blur_chains.get_mut(&backdrop.owner) {
                chain.drawn = true;
                chain.raised_drawn = !chain.raised.is_empty();
                chain.source = source;
                chain.backdrop = complete.then(|| backdrop.clone());
            }
        }
        let blurred = chain.raised[0];
        self.bind(match blur.tone {
            Some(_) => Program::BlurTone,
            None => Program::Texture,
        })?;
        self.sample(blurred.1, blurred.1)?;
        self.draw(
            &Quad {
                color: blur.tone.unwrap_or_default(),
                destination: source,
                sampling: Sampling::upright(Rectangle::from_size(
                    (f64::from(source.width), f64::from(source.height)).into(),
                )),
                alpha: blur.alpha,
                clip: blur.region,
                dither: self.smallest_step(),
                ..Quad::default()
            },
            &rects,
        );
        Ok(())
    }

    fn blur(
        &mut self,
        blur: &Blur,
        chain: &Levels,
        source: Rect,
        changed: &[Rect],
        kept: bool,
    ) -> Result<(), RenderError> {
        for program in [Program::BlurDown, Program::BlurUp] {
            self.bind(program)?;
        }
        for level in chain.levels.iter().chain(&chain.raised) {
            self.renderer.descriptor_set(level.1, level.1)?;
        }
        let passes = blur.passes() as usize;
        let spread = blur.spread();
        let load = match kept {
            true => vk::AttachmentLoadOp::LOAD,
            false => vk::AttachmentLoadOp::DONT_CARE,
        };
        let part_of = |level: Level| -> Vec<Rect> {
            let whole = Rect::new(0, 0, level.2, level.3);
            match kept {
                true => disjoint(
                    changed
                        .iter()
                        .map(|part| widened(*part, spread, source, (level.2, level.3))),
                    whole,
                ),
                false => vec![whole],
            }
        };
        self.pause();
        self.copy_into(source, changed, chain.levels[0], chain.prior);
        for pass in 0..passes {
            let to = chain.levels[pass + 1];
            self.filter(
                Program::BlurDown,
                chain.levels[pass],
                (to, chain.prior),
                blur.offset,
                (&part_of(to), load),
            )?;
        }
        for pass in (0..passes).rev() {
            let from = match pass + 1 == passes {
                true => chain.levels[passes],
                false => chain.raised[pass + 1],
            };
            let to = chain.raised[pass];
            self.filter(
                Program::BlurUp,
                from,
                (to, chain.raised_prior),
                blur.offset,
                (&part_of(to), load),
            )?;
        }
        self.resume();
        Ok(())
    }

    fn blur_levels(
        &mut self,
        backdrop: &Backdrop,
        source: Rect,
        passes: u32,
    ) -> Result<Levels, RenderError> {
        let key = backdrop.owner;
        let fits = self.renderer.blur_chains.get(&key).is_some_and(|chain| {
            (chain.source.width, chain.source.height) == (source.width, source.height)
                && chain.format == self.format
                && chain.levels.len() == passes as usize + 1
        });
        if !fits {
            if let Some(chain) = self.renderer.blur_chains.remove(&key) {
                chain
                    .images()
                    .for_each(|image| self.renderer.destroy_image(image));
            }
            let levels = self.create_levels(source, passes + 1)?;
            self.renderer.blur_chains.insert(
                key,
                BlurChain {
                    levels,
                    raised: Vec::new(),
                    raised_drawn: false,
                    last_used: Instant::now(),
                    source,
                    format: self.format,
                    drawn: false,
                    backdrop: None,
                },
            );
        }
        let separate = matches!(
            backdrop.change,
            Change::Everything {
                kept_for_parts: true
            }
        );
        let raised = match self.renderer.blur_chains.get(&key) {
            Some(chain) if separate && chain.raised.is_empty() => {
                self.create_levels(source, passes)?
            }
            _ => Vec::new(),
        };
        let chain = self
            .renderer
            .blur_chains
            .get_mut(&key)
            .ok_or(RenderError::ForeignResource)?;
        chain.raised.extend(raised);
        chain.last_used = Instant::now();
        let prior = |drawn: bool| match drawn {
            true => SAMPLED,
            false => DISCARDED,
        };
        let handles = |images: &[Image]| -> Vec<Level> {
            images
                .iter()
                .map(|image| (image.image, image.view, image.width, image.height))
                .collect()
        };
        let levels = handles(&chain.levels);
        Ok(Levels {
            raised: match chain.raised.is_empty() {
                true => levels.clone(),
                false => handles(&chain.raised),
            },
            levels,
            prior: prior(chain.drawn),
            raised_prior: match chain.raised.is_empty() {
                true => SAMPLED,
                false => prior(chain.raised_drawn),
            },
            drawing: chain
                .backdrop
                .as_ref()
                .filter(|_| chain.source == source)
                .map(|kept| kept.drawing),
        })
    }

    fn create_levels(&mut self, source: Rect, count: u32) -> Result<Vec<Image>, RenderError> {
        let mut levels = Vec::new();
        for level in 0..count {
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
        Ok(levels)
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

    fn copy_into(&mut self, source: Rect, parts: &[Rect], level: Level, prior: Usage) {
        let device = &self.renderer.device;
        let commands = self.renderer.command_buffer;
        barrier(device, commands, self.image, ATTACHMENT, TRANSFER_SOURCE);
        barrier(device, commands, level.0, prior, COPY_DESTINATION);
        let regions: Vec<vk::ImageCopy> = parts
            .iter()
            .map(|part| {
                vk::ImageCopy::default()
                    .src_subresource(COLOR_LAYERS)
                    .src_offset(vk::Offset3D {
                        x: part.x,
                        y: part.y,
                        z: 0,
                    })
                    .dst_subresource(COLOR_LAYERS)
                    .dst_offset(vk::Offset3D {
                        x: part.x - source.x,
                        y: part.y - source.y,
                        z: 0,
                    })
                    .extent(vk::Extent3D {
                        width: part.width.unsigned_abs(),
                        height: part.height.unsigned_abs(),
                        depth: 1,
                    })
            })
            .collect();
        // SAFETY: the frame holds the renderer mutably while `command_buffer` records; the barriers
        // above moved both live images to the copy layouts, and every part lies within `source`,
        // which lies within the target while level 0 has exactly its size.
        unsafe {
            device.cmd_copy_image(
                commands,
                self.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                level.0,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &regions,
            );
        }
        barrier(device, commands, self.image, TRANSFER_SOURCE, DRAWING);
        barrier(device, commands, level.0, COPY_DESTINATION, SAMPLED);
        self.renderer.counters.shaded_pixels += parts.iter().map(Rect::area).sum::<u64>();
    }

    fn filter(
        &mut self,
        program: Program,
        from: Level,
        (to, prior): (Level, Usage),
        offset: f32,
        (parts, load): (&[Rect], vk::AttachmentLoadOp),
    ) -> Result<(), RenderError> {
        let bounds = Rect::new(0, 0, to.2, to.3);
        let drawn = match load {
            vk::AttachmentLoadOp::LOAD => DRAWING,
            _ => DRAWN,
        };
        barrier(
            &self.renderer.device,
            self.renderer.command_buffer,
            to.0,
            prior,
            drawn,
        );
        self.begin_rendering(to.1, bounds, load);
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
            parts,
        );
        self.bounds = target;
        self.renderer.counters.shaded_pixels += parts.iter().map(Rect::area).sum::<u64>();
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
            drawn,
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
