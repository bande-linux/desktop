use std::fmt;

use crate::scene::Scene;

#[derive(Debug)]
pub enum RenderError {
    TooLarge { width: u32, height: u32 },
    BufferSize,
    Adapter(wgpu::RequestAdapterError),
    Device(wgpu::RequestDeviceError),
    Gpu(String),
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
        }
    }
}

impl std::error::Error for RenderError {}

pub struct Snapshot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub(crate) fn extent(scene: &Scene) -> Result<(u16, u16), RenderError> {
    let too_large = || RenderError::TooLarge {
        width: scene.width,
        height: scene.height,
    };
    Ok((
        u16::try_from(scene.width).map_err(|_| too_large())?,
        u16::try_from(scene.height).map_err(|_| too_large())?,
    ))
}
