use std::collections::HashMap;
use std::io;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::{Error as DamageError, OutputDamageTracker};
use smithay::backend::renderer::{Bind, Color32F, Offscreen};
use smithay::utils::{Physical, Size, Transform};
use unbound_render::{DeviceInfo, OffscreenTarget, RenderError, VulkanRenderer};

use crate::clock::{FrameOutput, Submission};
use crate::state::Headless;

pub const BACKGROUND: [u8; 3] = [0x20, 0x28, 0x30];

pub fn background() -> Color32F {
    let [red, green, blue] = BACKGROUND.map(|channel| f32::from(channel) / 255.0);
    Color32F::new(red, green, blue, 1.0)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FrameStats {
    pub uploaded_bytes: u64,
    pub repainted_pixels: u64,
    pub shaded_pixels: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub size: Size<i32, Physical>,
    pub rgba: Vec<u8>,
}

struct Output {
    size: Size<i32, Physical>,
    scale: f64,
    transform: Transform,
    target: OffscreenTarget,
    damage: OutputDamageTracker,
}

pub struct Renderer {
    vulkan: VulkanRenderer,
    outputs: HashMap<String, Output>,
}

pub(crate) fn rendering_error(error: DamageError<RenderError>) -> RenderError {
    match error {
        DamageError::Rendering(error) => error,
        DamageError::OutputNoMode(error) => RenderError::Io(io::Error::other(error)),
    }
}

impl Renderer {
    pub fn new() -> Result<Self, RenderError> {
        Ok(Self {
            vulkan: VulkanRenderer::new()?,
            outputs: HashMap::new(),
        })
    }

    pub fn device_info(&self) -> &DeviceInfo {
        self.vulkan.device_info()
    }

    pub fn render(&mut self, compositor: &mut Headless) -> Result<FrameStats, RenderError> {
        let main = compositor.main_output();
        self.render_output(compositor, &main)
    }

    pub fn render_output(
        &mut self,
        compositor: &mut Headless,
        name: &str,
    ) -> Result<FrameStats, RenderError> {
        let Some(settings) = compositor.output_settings(name) else {
            return Err(RenderError::NothingRendered);
        };
        let scale = compositor.render_scale(name).unwrap_or(settings.scale);
        let (size, transform) = (settings.mode.size, settings.transform);
        self.outputs
            .retain(|known, _| compositor.outputs().iter().any(|output| output == known));
        let output = match self.outputs.remove(name) {
            Some(output)
                if output.size == size
                    && output.scale == scale
                    && output.transform == transform =>
            {
                Some(output)
            }
            _ => None,
        };
        let age = usize::from(output.is_some());
        let mut output = match output {
            Some(output) => output,
            None => Output {
                size,
                scale,
                transform,
                target: self
                    .vulkan
                    .create_buffer(Fourcc::Abgr8888, Size::from((size.w, size.h)))?,
                damage: OutputDamageTracker::new(size, scale, transform),
            },
        };
        let elements = compositor.render_elements_on(&mut self.vulkan, name);
        let rendered = self
            .vulkan
            .bind(&mut output.target)
            .and_then(|mut framebuffer| {
                output
                    .damage
                    .render_output(
                        &mut self.vulkan,
                        &mut framebuffer,
                        age,
                        &elements,
                        background(),
                    )
                    .map(|result| {
                        result.damage.map_or(0, |damage| {
                            damage
                                .iter()
                                .map(|rect| {
                                    u64::from(rect.size.w.unsigned_abs())
                                        * u64::from(rect.size.h.unsigned_abs())
                                })
                                .sum()
                        })
                    })
                    .map_err(rendering_error)
            });
        self.outputs.insert(name.to_owned(), output);
        let counters = self.vulkan.take_counters();
        compositor.copy_captures(&mut self.vulkan, name);
        self.vulkan.take_counters();
        Ok(FrameStats {
            uploaded_bytes: counters.uploaded_bytes,
            repainted_pixels: rendered?,
            shaded_pixels: counters.shaded_pixels,
        })
    }

    pub fn snapshot(&mut self, compositor: &Headless) -> Result<Snapshot, RenderError> {
        self.snapshot_output(&compositor.main_output())
    }

    pub fn snapshot_output(&mut self, name: &str) -> Result<Snapshot, RenderError> {
        let output = self.outputs.get(name).ok_or(RenderError::NothingRendered)?;
        Ok(Snapshot {
            size: output.size,
            rgba: self.vulkan.read_target(&output.target)?,
        })
    }
}

impl FrameOutput for Renderer {
    type Error = RenderError;

    fn submit(
        &mut self,
        compositor: &mut Headless,
        output: &str,
    ) -> Result<Submission, RenderError> {
        self.render_output(compositor, output)?;
        Ok(Submission::AtVblank)
    }
}
