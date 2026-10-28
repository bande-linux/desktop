mod rect;

use std::collections::HashMap;
use std::ffi::CStr;
use std::fmt;
use std::slice;

use ash::vk;

pub use rect::{Rect, disjoint};

const SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/compose.spv"));
const TARGET_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
const TEXTURE_FORMAT: vk::Format = vk::Format::B8G8R8A8_UNORM;
const BYTES_PER_PIXEL: usize = 4;
const MIN_STAGING_BYTES: usize = 4 << 20;

#[derive(Debug)]
pub enum RenderError {
    Loading(ash::LoadingError),
    Vulkan(vk::Result),
    NoDevice,
    NoMemoryType,
    BufferOutOfBounds,
    NothingRendered,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loading(error) => write!(f, "cannot load the Vulkan library: {error}"),
            Self::Vulkan(result) => write!(f, "Vulkan call failed: {result}"),
            Self::NoDevice => f.write_str("no Vulkan 1.3 device with push descriptors"),
            Self::NoMemoryType => f.write_str("no suitable Vulkan memory type"),
            Self::BufferOutOfBounds => f.write_str("buffer rows lie outside its pool"),
            Self::NothingRendered => f.write_str("no frame has been rendered yet"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<vk::Result> for RenderError {
    fn from(result: vk::Result) -> Self {
        Self::Vulkan(result)
    }
}

impl From<ash::LoadingError> for RenderError {
    fn from(error: ash::LoadingError) -> Self {
        Self::Loading(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    pub driver: String,
}

pub struct ShmView<'a> {
    pub pool: &'a [u8],
    pub offset: usize,
    pub stride: usize,
    pub width: i32,
    pub height: i32,
    pub opaque: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub texture: u64,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FrameStats {
    pub uploaded_bytes: u64,
    pub repainted_pixels: u64,
}

struct Image {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    width: i32,
    height: i32,
}

struct Texture {
    image: Image,
    opaque: bool,
    readable: bool,
}

struct Staging {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: *mut u8,
    capacity: usize,
    used: usize,
}

struct PendingCopy {
    texture: u64,
    staging: usize,
    offset: usize,
    rect: Rect,
}

pub struct Renderer {
    instance: ash::Instance,
    device: ash::Device,
    push_descriptor: ash::khr::push_descriptor::Device,
    queue: vk::Queue,
    memory: vk::PhysicalDeviceMemoryProperties,
    info: DeviceInfo,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,
    set_layout: vk::DescriptorSetLayout,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    target: Option<Image>,
    textures: HashMap<u64, Texture>,
    staging: Vec<Staging>,
    copies: Vec<PendingCopy>,
    uploaded_bytes: u64,
    _entry: ash::Entry,
}

struct Chosen {
    physical: vk::PhysicalDevice,
    family: u32,
    info: DeviceInfo,
}

fn device_rank(kind: vk::PhysicalDeviceType) -> u8 {
    match kind {
        vk::PhysicalDeviceType::DISCRETE_GPU | vk::PhysicalDeviceType::INTEGRATED_GPU => 0,
        vk::PhysicalDeviceType::VIRTUAL_GPU => 1,
        vk::PhysicalDeviceType::CPU => 2,
        _ => 3,
    }
}

fn text(name: Result<&CStr, std::ffi::FromBytesUntilNulError>) -> String {
    name.map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn choose_device(instance: &ash::Instance) -> Result<Chosen, RenderError> {
    let physicals = unsafe { instance.enumerate_physical_devices() }?;
    physicals
        .into_iter()
        .filter_map(|physical| {
            let properties = unsafe { instance.get_physical_device_properties(physical) };
            if properties.api_version < vk::API_VERSION_1_3 {
                return None;
            }
            let extensions =
                unsafe { instance.enumerate_device_extension_properties(physical) }.ok()?;
            extensions
                .iter()
                .any(|extension| {
                    extension.extension_name_as_c_str() == Ok(ash::khr::push_descriptor::NAME)
                })
                .then_some(())?;
            let family = unsafe { instance.get_physical_device_queue_family_properties(physical) }
                .iter()
                .position(|family| family.queue_flags.contains(vk::QueueFlags::GRAPHICS))?;
            let mut driver = vk::PhysicalDeviceDriverProperties::default();
            let mut properties2 = vk::PhysicalDeviceProperties2::default().push_next(&mut driver);
            unsafe { instance.get_physical_device_properties2(physical, &mut properties2) };
            let info = DeviceInfo {
                name: text(properties.device_name_as_c_str()),
                driver: text(driver.driver_name_as_c_str()),
            };
            Some((
                device_rank(properties.device_type),
                Chosen {
                    physical,
                    family: u32::try_from(family).ok()?,
                    info,
                },
            ))
        })
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, chosen)| chosen)
        .ok_or(RenderError::NoDevice)
}

impl Renderer {
    pub fn new() -> Result<Self, RenderError> {
        let entry = unsafe { ash::Entry::load() }?;
        let application = vk::ApplicationInfo::default()
            .application_name(c"unbound-compositor")
            .api_version(vk::API_VERSION_1_3);
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default().application_info(&application),
                None,
            )
        }?;
        let chosen = match choose_device(&instance) {
            Ok(chosen) => chosen,
            Err(error) => {
                unsafe { instance.destroy_instance(None) };
                return Err(error);
            }
        };
        let priorities = [1.0];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(chosen.family)
            .queue_priorities(&priorities);
        let mut vulkan13 = vk::PhysicalDeviceVulkan13Features::default()
            .dynamic_rendering(true)
            .synchronization2(true);
        let extensions = [ash::khr::push_descriptor::NAME.as_ptr()];
        let device = match unsafe {
            instance.create_device(
                chosen.physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(slice::from_ref(&queue_info))
                    .enabled_extension_names(&extensions)
                    .push_next(&mut vulkan13),
                None,
            )
        } {
            Ok(device) => device,
            Err(error) => {
                unsafe { instance.destroy_instance(None) };
                return Err(error.into());
            }
        };
        let mut renderer = Self {
            push_descriptor: ash::khr::push_descriptor::Device::new(&instance, &device),
            queue: unsafe { device.get_device_queue(chosen.family, 0) },
            memory: unsafe { instance.get_physical_device_memory_properties(chosen.physical) },
            info: chosen.info,
            command_pool: vk::CommandPool::null(),
            command_buffer: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            set_layout: vk::DescriptorSetLayout::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            target: None,
            textures: HashMap::new(),
            staging: Vec::new(),
            copies: Vec::new(),
            uploaded_bytes: 0,
            instance,
            device,
            _entry: entry,
        };
        renderer.create_objects(chosen.family)?;
        Ok(renderer)
    }

    fn create_objects(&mut self, family: u32) -> Result<(), RenderError> {
        let device = &self.device;
        self.command_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
        }?;
        self.command_buffer = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }?
        .into_iter()
        .next()
        .ok_or(RenderError::Vulkan(vk::Result::ERROR_UNKNOWN))?;
        self.fence = unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }?;

        let binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        self.set_layout = unsafe {
            device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default()
                    .flags(vk::DescriptorSetLayoutCreateFlags::PUSH_DESCRIPTOR_KHR)
                    .bindings(slice::from_ref(&binding)),
                None,
            )
        }?;
        let quad = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .size(QUAD_BYTES);
        self.pipeline_layout = unsafe {
            device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(slice::from_ref(&self.set_layout))
                    .push_constant_ranges(slice::from_ref(&quad)),
                None,
            )
        }?;
        self.pipeline = self.create_pipeline()?;
        Ok(())
    }

    fn create_pipeline(&self) -> Result<vk::Pipeline, RenderError> {
        let words: Vec<u32> = SPIRV
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word))
            .collect();
        let module = unsafe {
            self.device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)
        }?;
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(module)
                .name(c"vertex"),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(module)
                .name(c"fragment"),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let premultiplied = vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA);
        let blend = vk::PipelineColorBlendStateCreateInfo::default()
            .attachments(slice::from_ref(&premultiplied));
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let formats = [TARGET_FORMAT];
        let mut rendering =
            vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&formats);
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&assembly)
            .viewport_state(&viewport)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(self.pipeline_layout)
            .push_next(&mut rendering);
        let pipelines = unsafe {
            self.device.create_graphics_pipelines(
                vk::PipelineCache::null(),
                slice::from_ref(&info),
                None,
            )
        };
        unsafe { self.device.destroy_shader_module(module, None) };
        pipelines
            .map_err(|(_, result)| RenderError::Vulkan(result))?
            .into_iter()
            .next()
            .ok_or(RenderError::Vulkan(vk::Result::ERROR_UNKNOWN))
    }

    pub fn device_info(&self) -> &DeviceInfo {
        &self.info
    }

    fn memory_type(
        &self,
        allowed: u32,
        wanted: vk::MemoryPropertyFlags,
    ) -> Result<u32, RenderError> {
        let types = &self.memory.memory_types[..self.memory.memory_type_count as usize];
        types
            .iter()
            .zip(0u32..)
            .find(|(kind, index)| {
                allowed & (1 << index) != 0 && kind.property_flags.contains(wanted)
            })
            .map(|(_, index)| index)
            .ok_or(RenderError::NoMemoryType)
    }

    fn create_image(
        &self,
        width: i32,
        height: i32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        alpha: vk::ComponentSwizzle,
    ) -> Result<Image, RenderError> {
        let image = unsafe {
            self.device.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(format)
                    .extent(vk::Extent3D {
                        width: width.unsigned_abs(),
                        height: height.unsigned_abs(),
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )
        }?;
        let mut created = Image {
            image,
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
            width,
            height,
        };
        let bound = self.bind_image_memory(&mut created).and_then(|()| {
            created.view = self.create_view(created.image, format, alpha)?;
            Ok(())
        });
        match bound {
            Ok(()) => Ok(created),
            Err(error) => {
                self.destroy_image(&created);
                Err(error)
            }
        }
    }

    fn bind_image_memory(&self, image: &mut Image) -> Result<(), RenderError> {
        let requirements = unsafe { self.device.get_image_memory_requirements(image.image) };
        let kind = self
            .memory_type(
                requirements.memory_type_bits,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            )
            .or_else(|_| {
                self.memory_type(
                    requirements.memory_type_bits,
                    vk::MemoryPropertyFlags::empty(),
                )
            })?;
        image.memory = unsafe {
            self.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(kind),
                None,
            )
        }?;
        unsafe { self.device.bind_image_memory(image.image, image.memory, 0) }?;
        Ok(())
    }

    fn create_view(
        &self,
        image: vk::Image,
        format: vk::Format,
        alpha: vk::ComponentSwizzle,
    ) -> Result<vk::ImageView, RenderError> {
        let view = unsafe {
            self.device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .components(vk::ComponentMapping {
                        a: alpha,
                        ..vk::ComponentMapping::default()
                    })
                    .subresource_range(COLOR_RANGE),
                None,
            )
        }?;
        Ok(view)
    }

    fn destroy_image(&self, image: &Image) {
        unsafe {
            self.device.destroy_image_view(image.view, None);
            self.device.destroy_image(image.image, None);
            self.device.free_memory(image.memory, None);
        }
    }

    fn create_staging(&self, capacity: usize) -> Result<Staging, RenderError> {
        let buffer = unsafe {
            self.device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(capacity as u64)
                    .usage(vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
        }?;
        let mut staging = Staging {
            buffer,
            memory: vk::DeviceMemory::null(),
            mapped: std::ptr::null_mut(),
            capacity,
            used: 0,
        };
        match self.map_staging(&mut staging) {
            Ok(()) => Ok(staging),
            Err(error) => {
                self.destroy_staging(&staging);
                Err(error)
            }
        }
    }

    fn map_staging(&self, staging: &mut Staging) -> Result<(), RenderError> {
        let requirements = unsafe { self.device.get_buffer_memory_requirements(staging.buffer) };
        let kind = self.memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        staging.memory = unsafe {
            self.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(kind),
                None,
            )
        }?;
        unsafe {
            self.device
                .bind_buffer_memory(staging.buffer, staging.memory, 0)?;
            staging.mapped = self
                .device
                .map_memory(
                    staging.memory,
                    0,
                    vk::WHOLE_SIZE,
                    vk::MemoryMapFlags::empty(),
                )?
                .cast();
        }
        Ok(())
    }

    fn destroy_staging(&self, staging: &Staging) {
        unsafe {
            self.device.destroy_buffer(staging.buffer, None);
            self.device.free_memory(staging.memory, None);
        }
    }

    fn reserve(&mut self, bytes: usize) -> Result<(usize, usize), RenderError> {
        let fits = self
            .staging
            .last()
            .is_some_and(|staging| staging.capacity - staging.used >= bytes);
        if !fits {
            let staging = self.create_staging(bytes.max(MIN_STAGING_BYTES))?;
            self.staging.push(staging);
        }
        let index = self.staging.len() - 1;
        let staging = &mut self.staging[index];
        let offset = staging.used;
        staging.used += bytes;
        Ok((index, offset))
    }

    pub fn upload(
        &mut self,
        texture: u64,
        view: &ShmView,
        damage: &[Rect],
    ) -> Result<(), RenderError> {
        let bounds = Rect::new(0, 0, view.width, view.height);
        let reusable = self.textures.get(&texture).is_some_and(|existing| {
            existing.image.width == view.width && existing.image.height == view.height
        });
        if !reusable {
            self.forget(texture);
            let image = self.create_image(
                view.width,
                view.height,
                TEXTURE_FORMAT,
                vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
                alpha_swizzle(view.opaque),
            )?;
            self.textures.insert(
                texture,
                Texture {
                    image,
                    opaque: view.opaque,
                    readable: false,
                },
            );
        }
        if let Some(existing) = self.textures.get(&texture)
            && existing.opaque != view.opaque
        {
            let replacement = self.create_view(
                existing.image.image,
                TEXTURE_FORMAT,
                alpha_swizzle(view.opaque),
            )?;
            if let Some(existing) = self.textures.get_mut(&texture) {
                let replaced = std::mem::replace(&mut existing.image.view, replacement);
                existing.opaque = view.opaque;
                unsafe { self.device.destroy_image_view(replaced, None) };
            }
        }

        let rects = if reusable {
            disjoint(damage.iter().copied(), bounds)
        } else {
            vec![bounds]
        };
        for rect in rects {
            self.stage(texture, view, rect)?;
        }
        Ok(())
    }

    fn stage(&mut self, texture: u64, view: &ShmView, rect: Rect) -> Result<(), RenderError> {
        let row_bytes = rect.width.unsigned_abs() as usize * BYTES_PER_PIXEL;
        let rows = rect.height.unsigned_abs() as usize;
        let (index, offset) = self.reserve(row_bytes * rows)?;
        let left = rect.x.unsigned_abs() as usize * BYTES_PER_PIXEL;
        let top = rect.y.unsigned_abs() as usize;
        let staging = &self.staging[index];
        for row in 0..rows {
            let start = view.offset + (top + row) * view.stride + left;
            let source = view
                .pool
                .get(start..start + row_bytes)
                .ok_or(RenderError::BufferOutOfBounds)?;
            let destination = offset + row * row_bytes;
            debug_assert!(destination + row_bytes <= staging.capacity);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    source.as_ptr(),
                    staging.mapped.add(destination),
                    row_bytes,
                );
            }
        }
        self.copies.push(PendingCopy {
            texture,
            staging: index,
            offset,
            rect,
        });
        self.uploaded_bytes += (row_bytes * rows) as u64;
        Ok(())
    }

    pub fn forget(&mut self, texture: u64) {
        if let Some(removed) = self.textures.remove(&texture) {
            self.destroy_image(&removed.image);
        }
        self.copies.retain(|copy| copy.texture != texture);
    }

    pub fn retain_textures(&mut self, alive: impl Fn(u64) -> bool) {
        let dead: Vec<u64> = self
            .textures
            .keys()
            .copied()
            .filter(|texture| !alive(*texture))
            .collect();
        dead.into_iter().for_each(|texture| self.forget(texture));
    }

    pub fn render(
        &mut self,
        width: i32,
        height: i32,
        background: [u8; 3],
        placements: &[Placement],
        damage: &[Rect],
    ) -> Result<FrameStats, RenderError> {
        let bounds = Rect::new(0, 0, width, height);
        let fresh = self
            .target
            .as_ref()
            .is_none_or(|target| target.width != width || target.height != height);
        if fresh {
            if let Some(old) = self.target.take() {
                self.destroy_image(&old);
            }
            self.target = Some(self.create_image(
                width,
                height,
                TARGET_FORMAT,
                vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
                vk::ComponentSwizzle::IDENTITY,
            )?);
        }
        let repaint = if fresh {
            vec![bounds]
        } else {
            disjoint(damage.iter().copied(), bounds)
        };

        let commands = self.command_buffer;
        unsafe {
            self.device
                .reset_command_buffer(commands, vk::CommandBufferResetFlags::empty())?;
            self.device.begin_command_buffer(
                commands,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }
        self.record_copies();
        self.record_repaint(fresh, &repaint, background, placements);
        unsafe { self.device.end_command_buffer(commands) }?;
        self.submit_and_wait()?;

        self.copies.clear();
        self.recycle_staging();
        Ok(FrameStats {
            uploaded_bytes: std::mem::take(&mut self.uploaded_bytes),
            repainted_pixels: repaint.iter().map(Rect::area).sum(),
        })
    }

    fn record_copies(&mut self) {
        let mut by_texture: HashMap<u64, Vec<&PendingCopy>> = HashMap::new();
        for copy in &self.copies {
            by_texture.entry(copy.texture).or_default().push(copy);
        }
        for (key, copies) in by_texture {
            let Some(texture) = self.textures.get_mut(&key) else {
                continue;
            };
            let previous = if texture.readable {
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
            } else {
                vk::ImageLayout::UNDEFINED
            };
            let image = texture.image.image;
            texture.readable = true;
            let device = &self.device;
            let commands = self.command_buffer;
            barrier(
                device,
                commands,
                image,
                (
                    previous,
                    vk::PipelineStageFlags2::FRAGMENT_SHADER,
                    vk::AccessFlags2::NONE,
                ),
                (
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::PipelineStageFlags2::COPY,
                    vk::AccessFlags2::TRANSFER_WRITE,
                ),
            );
            for copy in copies {
                let region = vk::BufferImageCopy::default()
                    .buffer_offset(copy.offset as u64)
                    .image_subresource(COLOR_LAYERS)
                    .image_offset(vk::Offset3D {
                        x: copy.rect.x,
                        y: copy.rect.y,
                        z: 0,
                    })
                    .image_extent(vk::Extent3D {
                        width: copy.rect.width.unsigned_abs(),
                        height: copy.rect.height.unsigned_abs(),
                        depth: 1,
                    });
                unsafe {
                    device.cmd_copy_buffer_to_image(
                        commands,
                        self.staging[copy.staging].buffer,
                        image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        slice::from_ref(&region),
                    );
                }
            }
            barrier(
                device,
                commands,
                image,
                (
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::PipelineStageFlags2::COPY,
                    vk::AccessFlags2::TRANSFER_WRITE,
                ),
                (
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    vk::PipelineStageFlags2::FRAGMENT_SHADER,
                    vk::AccessFlags2::SHADER_SAMPLED_READ,
                ),
            );
        }
    }

    fn record_repaint(
        &self,
        fresh: bool,
        repaint: &[Rect],
        background: [u8; 3],
        placements: &[Placement],
    ) {
        let Some(target) = &self.target else {
            return;
        };
        let device = &self.device;
        let commands = self.command_buffer;
        let attachment_access =
            vk::AccessFlags2::COLOR_ATTACHMENT_READ | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE;
        barrier(
            device,
            commands,
            target.image,
            (
                if fresh {
                    vk::ImageLayout::UNDEFINED
                } else {
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
                },
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            ),
            (
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                attachment_access,
            ),
        );
        if repaint.is_empty() {
            return;
        }

        let extent = vk::Extent2D {
            width: target.width.unsigned_abs(),
            height: target.height.unsigned_abs(),
        };
        let attachment = vk::RenderingAttachmentInfo::default()
            .image_view(target.view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::LOAD)
            .store_op(vk::AttachmentStoreOp::STORE);
        let [red, green, blue] = background.map(|channel| f32::from(channel) / 255.0);
        let clear = vk::ClearAttachment {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            color_attachment: 0,
            clear_value: vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [red, green, blue, 1.0],
                },
            },
        };
        unsafe {
            device.cmd_begin_rendering(
                commands,
                &vk::RenderingInfo::default()
                    .render_area(extent.into())
                    .layer_count(1)
                    .color_attachments(slice::from_ref(&attachment)),
            );
            device.cmd_bind_pipeline(commands, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            device.cmd_set_viewport(
                commands,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: extent.width as f32,
                    height: extent.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
        }
        for area in repaint {
            let scissor = vk::Rect2D {
                offset: vk::Offset2D {
                    x: area.x,
                    y: area.y,
                },
                extent: vk::Extent2D {
                    width: area.width.unsigned_abs(),
                    height: area.height.unsigned_abs(),
                },
            };
            unsafe {
                device.cmd_set_scissor(commands, 0, slice::from_ref(&scissor));
                device.cmd_clear_attachments(
                    commands,
                    slice::from_ref(&clear),
                    &[vk::ClearRect {
                        rect: scissor,
                        base_array_layer: 0,
                        layer_count: 1,
                    }],
                );
            }
            for placement in placements {
                let Some(texture) = self.textures.get(&placement.texture) else {
                    continue;
                };
                let quad = Rect::new(
                    placement.x,
                    placement.y,
                    texture.image.width,
                    texture.image.height,
                );
                if !texture.readable || quad.intersection(area).is_none() {
                    continue;
                }
                self.record_quad(texture, quad, extent);
            }
        }
        unsafe { device.cmd_end_rendering(commands) };
    }

    fn record_quad(&self, texture: &Texture, quad: Rect, target: vk::Extent2D) {
        let image_info = vk::DescriptorImageInfo::default()
            .image_view(texture.image.view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let write = vk::WriteDescriptorSet::default()
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .image_info(slice::from_ref(&image_info));
        let constants: Vec<u8> = [
            target.width as f32,
            target.height as f32,
            quad.x as f32,
            quad.y as f32,
            quad.width as f32,
            quad.height as f32,
        ]
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect();
        unsafe {
            self.push_descriptor.cmd_push_descriptor_set(
                self.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline_layout,
                0,
                slice::from_ref(&write),
            );
            self.device.cmd_push_constants(
                self.command_buffer,
                self.pipeline_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                &constants,
            );
            self.device.cmd_draw(self.command_buffer, 4, 1, 0, 0);
        }
    }

    fn submit_and_wait(&self) -> Result<(), RenderError> {
        let submit =
            vk::SubmitInfo::default().command_buffers(slice::from_ref(&self.command_buffer));
        unsafe {
            self.device
                .queue_submit(self.queue, slice::from_ref(&submit), self.fence)?;
            self.device
                .wait_for_fences(slice::from_ref(&self.fence), true, u64::MAX)?;
            self.device.reset_fences(slice::from_ref(&self.fence))?;
        }
        Ok(())
    }

    fn recycle_staging(&mut self) {
        let Some(largest) = self
            .staging
            .iter()
            .enumerate()
            .max_by_key(|(_, staging)| staging.capacity)
            .map(|(index, _)| index)
        else {
            return;
        };
        let mut kept = self.staging.swap_remove(largest);
        kept.used = 0;
        for staging in std::mem::take(&mut self.staging) {
            self.destroy_staging(&staging);
        }
        self.staging.push(kept);
    }

    pub fn read_back(&mut self) -> Result<Vec<u8>, RenderError> {
        let (image, width, height) = match &self.target {
            Some(target) => (target.image, target.width, target.height),
            None => return Err(RenderError::NothingRendered),
        };
        let row_bytes = width.unsigned_abs() as usize * BYTES_PER_PIXEL;
        let bytes = row_bytes * height.unsigned_abs() as usize;
        let readback = self.create_staging(bytes)?;
        let result = self
            .copy_target_to(&readback, image, width, height)
            .map(|()| {
                let mut rgba = vec![0u8; bytes];
                unsafe { std::ptr::copy_nonoverlapping(readback.mapped, rgba.as_mut_ptr(), bytes) };
                rgba
            });
        self.destroy_staging(&readback);
        result
    }

    fn copy_target_to(
        &self,
        readback: &Staging,
        image: vk::Image,
        width: i32,
        height: i32,
    ) -> Result<(), RenderError> {
        let device = &self.device;
        let commands = self.command_buffer;
        let attachment = (
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
        );
        let source = (
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            vk::PipelineStageFlags2::COPY,
            vk::AccessFlags2::TRANSFER_READ,
        );
        let region = vk::BufferImageCopy::default()
            .image_subresource(COLOR_LAYERS)
            .image_extent(vk::Extent3D {
                width: width.unsigned_abs(),
                height: height.unsigned_abs(),
                depth: 1,
            });
        unsafe {
            device.reset_command_buffer(commands, vk::CommandBufferResetFlags::empty())?;
            device.begin_command_buffer(
                commands,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }
        barrier(device, commands, image, attachment, source);
        unsafe {
            device.cmd_copy_image_to_buffer(
                commands,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                readback.buffer,
                slice::from_ref(&region),
            );
        }
        barrier(device, commands, image, source, attachment);
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
            device.end_command_buffer(commands)?;
        }
        self.submit_and_wait()
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
        }
        for (_, texture) in std::mem::take(&mut self.textures) {
            self.destroy_image(&texture.image);
        }
        if let Some(target) = self.target.take() {
            self.destroy_image(&target);
        }
        for staging in std::mem::take(&mut self.staging) {
            self.destroy_staging(&staging);
        }
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device
                .destroy_descriptor_set_layout(self.set_layout, None);
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

const QUAD_BYTES: u32 = 6 * 4;

const COLOR_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

const COLOR_LAYERS: vk::ImageSubresourceLayers = vk::ImageSubresourceLayers {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    mip_level: 0,
    base_array_layer: 0,
    layer_count: 1,
};

fn alpha_swizzle(opaque: bool) -> vk::ComponentSwizzle {
    if opaque {
        vk::ComponentSwizzle::ONE
    } else {
        vk::ComponentSwizzle::IDENTITY
    }
}

type Usage = (vk::ImageLayout, vk::PipelineStageFlags2, vk::AccessFlags2);

fn barrier(
    device: &ash::Device,
    commands: vk::CommandBuffer,
    image: vk::Image,
    from: Usage,
    to: Usage,
) {
    let barrier = vk::ImageMemoryBarrier2::default()
        .old_layout(from.0)
        .src_stage_mask(from.1)
        .src_access_mask(from.2)
        .new_layout(to.0)
        .dst_stage_mask(to.1)
        .dst_access_mask(to.2)
        .image(image)
        .subresource_range(COLOR_RANGE);
    unsafe {
        device.cmd_pipeline_barrier2(
            commands,
            &vk::DependencyInfo::default().image_memory_barriers(slice::from_ref(&barrier)),
        );
    }
}
