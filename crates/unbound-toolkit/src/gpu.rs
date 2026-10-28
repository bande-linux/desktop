use std::fmt;
use std::sync::mpsc;

#[derive(Debug)]
pub enum GpuError {
    Adapter(wgpu::RequestAdapterError),
    Device(wgpu::RequestDeviceError),
    Readback(String),
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adapter(error) => write!(f, "no usable Vulkan adapter: {error}"),
            Self::Device(error) => write!(f, "cannot open the Vulkan device: {error}"),
            Self::Readback(error) => write!(f, "cannot read the rendered frame back: {error}"),
        }
    }
}

impl std::error::Error for GpuError {}

pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    pub fn new() -> Result<Self, GpuError> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(descriptor.with_env());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .map_err(GpuError::Adapter)?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .map_err(GpuError::Device)?;
        Ok(Self {
            adapter,
            device,
            queue,
        })
    }

    pub fn render_target(&self, width: u32, height: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    pub fn finish_and_read(
        &self,
        mut encoder: wgpu::CommandEncoder,
        target: &wgpu::Texture,
    ) -> Result<Vec<u8>, GpuError> {
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
        let readback_error = |error: &dyn fmt::Display| GpuError::Readback(error.to_string());
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| readback_error(&error))?;
        receiver
            .recv()
            .map_err(|error| readback_error(&error))?
            .map_err(|error| readback_error(&error))?;
        let mapped = slice
            .get_mapped_range()
            .map_err(|error| readback_error(&error))?;
        Ok(mapped
            .chunks(padded_row_bytes as usize)
            .flat_map(|row| &row[..row_bytes as usize])
            .copied()
            .collect())
    }
}
