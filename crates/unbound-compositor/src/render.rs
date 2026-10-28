use std::io;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::{Error as DamageError, OutputDamageTracker};
use smithay::backend::renderer::{Bind, Color32F, Offscreen};
use smithay::utils::{Physical, Size, Transform};
use unbound_render::{DeviceInfo, OffscreenTarget, RenderError, VulkanRenderer};

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
    target: OffscreenTarget,
    damage: OutputDamageTracker,
}

pub struct Renderer {
    vulkan: VulkanRenderer,
    output: Option<Output>,
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
            output: None,
        })
    }

    pub fn device_info(&self) -> &DeviceInfo {
        self.vulkan.device_info()
    }

    pub fn render(&mut self, compositor: &mut Headless) -> Result<FrameStats, RenderError> {
        let size = compositor.output_size();
        let output = match self.output.take() {
            Some(output) if output.size == size => Some(output),
            _ => None,
        };
        let age = usize::from(output.is_some());
        let mut output = match output {
            Some(output) => output,
            None => Output {
                size,
                target: self
                    .vulkan
                    .create_buffer(Fourcc::Abgr8888, Size::from((size.w, size.h)))?,
                damage: OutputDamageTracker::new(size, 1.0, Transform::Normal),
            },
        };
        let elements = compositor.render_elements(&mut self.vulkan);
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
        self.output = Some(output);
        let counters = self.vulkan.take_counters();
        Ok(FrameStats {
            uploaded_bytes: counters.uploaded_bytes,
            repainted_pixels: rendered?,
            shaded_pixels: counters.shaded_pixels,
        })
    }

    pub fn snapshot(&mut self, compositor: &Headless) -> Result<Snapshot, RenderError> {
        let output = self.output.as_ref().ok_or(RenderError::NothingRendered)?;
        Ok(Snapshot {
            size: compositor.output_size(),
            rgba: self.vulkan.read_target(&output.target)?,
        })
    }
}
