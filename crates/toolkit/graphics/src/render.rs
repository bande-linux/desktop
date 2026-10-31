use std::fmt;

use crate::scene::Scene;

#[derive(Debug)]
pub enum RenderError {
    TooLarge { width: u32, height: u32 },
    BufferSize,
    Adapter(wgpu::RequestAdapterError),
    Device(wgpu::RequestDeviceError),
    Gpu(String),
    Software(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { width, height } => {
                write!(f, "a {width}x{height} scene exceeds 65535 pixels per side")
            }
            Self::BufferSize => write!(f, "the target buffer does not match the scene's size"),
            Self::Adapter(error) => write!(f, "no usable Vulkan adapter: {error}"),
            Self::Device(error) => write!(f, "cannot open the Vulkan device: {error}"),
            Self::Gpu(error) => write!(f, "GPU rendering failed: {error}"),
            Self::Software(adapter) => {
                write!(f, "the only Vulkan device, {adapter}, renders on the CPU")
            }
        }
    }
}

impl std::error::Error for RenderError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub(crate) fn extent(scene: &Scene, scale: f64) -> Result<(u16, u16), RenderError> {
    let (width, height) = (pixels(scene.width, scale), pixels(scene.height, scale));
    let too_large = || RenderError::TooLarge { width, height };
    Ok((
        u16::try_from(width).map_err(|_| too_large())?,
        u16::try_from(height).map_err(|_| too_large())?,
    ))
}

pub fn pixels(points: u32, scale: f64) -> u32 {
    (f64::from(points) * scale).round().max(1.0) as u32
}
