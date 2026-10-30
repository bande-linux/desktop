mod frame;
mod import;
mod rect;

use std::collections::HashMap;
use std::ffi::CStr;
use std::fmt;
use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::marker::PhantomData;
use std::os::fd::{AsRawFd, BorrowedFd, IntoRawFd, OwnedFd};
use std::slice;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use ash::vk;
use smithay::backend::allocator::dmabuf::{Dmabuf, WeakDmabuf};
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Buffer, Format, Fourcc, Modifier};
use smithay::backend::renderer::sync::SyncPoint;
use smithay::backend::renderer::{
    Bind, ContextId, DebugFlags, Offscreen, Renderer, RendererSuper, Texture, TextureFilter,
};
use smithay::reexports::rustix::mm;
use smithay::utils::{Buffer as BufferCoords, Physical, Size, Transform};

pub use frame::{Blur, RoundedRect, Shadow, VulkanFrame};
pub use rect::{Rect, disjoint};

const SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/compose.spv"));
const BYTES_PER_PIXEL: usize = 4;
const MIN_STAGING_BYTES: usize = 4 << 20;
const REQUIRED_EXTENSIONS: [&CStr; 3] = [
    ash::khr::push_descriptor::NAME,
    ash::khr::external_memory_fd::NAME,
    ash::ext::external_memory_dma_buf::NAME,
];
const TARGET_FORMATS: [vk::Format; 2] = [vk::Format::B8G8R8A8_UNORM, vk::Format::R8G8B8A8_UNORM];
const TEXTURE_FOURCCS: [Fourcc; 2] = [Fourcc::Argb8888, Fourcc::Xrgb8888];

static NEXT_RENDERER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub enum RenderError {
    Loading(ash::LoadingError),
    Vulkan(vk::Result),
    NoDevice,
    NoMemoryType,
    BufferOutOfBounds,
    UnalignedDmabuf,
    UnsupportedFormat,
    UnsupportedTransform,
    UnknownBuffer,
    ForeignResource,
    Interrupted,
    NothingRendered,
    Io(std::io::Error),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loading(error) => write!(f, "cannot load the Vulkan library: {error}"),
            Self::Vulkan(result) => write!(f, "Vulkan call failed: {result}"),
            Self::NoDevice => {
                f.write_str("no Vulkan 1.3 device with push descriptors and dma-buf import")
            }
            Self::NoMemoryType => f.write_str("no suitable Vulkan memory type"),
            Self::BufferOutOfBounds => f.write_str("buffer rows lie outside its pool"),
            Self::UnalignedDmabuf => {
                f.write_str("dma-buf offset or stride is not a multiple of the pixel size")
            }
            Self::UnsupportedFormat => f.write_str("pixel format or layout is not supported"),
            Self::UnsupportedTransform => f.write_str("only untransformed rendering is supported"),
            Self::UnknownBuffer => f.write_str("buffer is neither wl_shm nor a dma-buf"),
            Self::ForeignResource => {
                f.write_str("texture or render target belongs to another renderer")
            }
            Self::Interrupted => f.write_str("waiting for a sync point was interrupted"),
            Self::NothingRendered => f.write_str("no frame has been rendered yet"),
            Self::Io(error) => write!(f, "cannot size the dma-buf: {error}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<vk::Result> for RenderError {
    fn from(result: vk::Result) -> Self {
        Self::Vulkan(result)
    }
}

impl From<std::io::Error> for RenderError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
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
    pub software: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counters {
    pub uploaded_bytes: u64,
    pub shaded_pixels: u64,
}

#[derive(Debug, Clone, Copy)]
enum Resource {
    Texture(u64),
    Target(u64),
}

type Graveyard = Arc<Mutex<Vec<Resource>>>;

#[derive(Debug)]
struct Owned {
    resource: Resource,
    graveyard: Graveyard,
}

impl Drop for Owned {
    fn drop(&mut self) {
        self.graveyard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(self.resource);
    }
}

#[derive(Debug, Clone)]
pub struct VulkanTexture {
    _owned: Arc<Owned>,
    id: u64,
    renderer: u64,
    width: i32,
    height: i32,
    format: Fourcc,
}

impl Texture for VulkanTexture {
    fn width(&self) -> u32 {
        self.width.unsigned_abs()
    }

    fn height(&self) -> u32 {
        self.height.unsigned_abs()
    }

    fn format(&self) -> Option<Fourcc> {
        Some(self.format)
    }
}

#[derive(Debug)]
pub struct OffscreenTarget {
    _owned: Owned,
    id: u64,
    renderer: u64,
    width: i32,
    height: i32,
    format: Fourcc,
}

#[derive(Debug)]
pub struct VulkanFramebuffer<'buffer> {
    id: u64,
    width: i32,
    height: i32,
    format: Fourcc,
    _target: PhantomData<&'buffer mut ()>,
}

impl Texture for VulkanFramebuffer<'_> {
    fn width(&self) -> u32 {
        self.width.unsigned_abs()
    }

    fn height(&self) -> u32 {
        self.height.unsigned_abs()
    }

    fn format(&self) -> Option<Fourcc> {
        Some(self.format)
    }
}

struct Image {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    width: i32,
    height: i32,
}

struct TextureImage {
    image: Image,
    readable: bool,
}

struct Imported {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: u64,
}

struct Export {
    fd: OwnedFd,
    mapping: *mut u8,
    length: usize,
    offset: usize,
    stride: usize,
}

impl Drop for Export {
    fn drop(&mut self) {
        let _ = unsafe { mm::munmap(self.mapping.cast(), self.length) };
    }
}

struct Target {
    image: Image,
    format: vk::Format,
    rendered: bool,
    export: Option<Export>,
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
    source: vk::Buffer,
    offset: u64,
    row_texels: u32,
    rect: Rect,
}

pub struct VulkanRenderer {
    instance: ash::Instance,
    device: ash::Device,
    push_descriptor: ash::khr::push_descriptor::Device,
    external_memory_fd: ash::khr::external_memory_fd::Device,
    queue: vk::Queue,
    memory: vk::PhysicalDeviceMemoryProperties,
    info: DeviceInfo,
    reduced_effects: bool,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,
    sampler: vk::Sampler,
    set_layout: vk::DescriptorSetLayout,
    pipeline_layout: vk::PipelineLayout,
    pipelines: HashMap<(vk::Format, Program), vk::Pipeline>,
    id: u64,
    context: ContextId<VulkanTexture>,
    debug_flags: DebugFlags,
    graveyard: Graveyard,
    next_resource: u64,
    textures: HashMap<u64, TextureImage>,
    targets: HashMap<u64, Target>,
    dmabuf_targets: HashMap<WeakDmabuf, u64>,
    dmabuf_textures: HashMap<WeakDmabuf, VulkanTexture>,
    imports: HashMap<WeakDmabuf, Imported>,
    staging: Vec<Staging>,
    copies: Vec<PendingCopy>,
    counters: Counters,
    blur_chains: HashMap<(i32, i32, vk::Format, u32), frame::BlurChain>,
    _entry: ash::Entry,
}

impl fmt::Debug for VulkanRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VulkanRenderer")
            .field("device", &self.info)
            .field("textures", &self.textures.len())
            .field("targets", &self.targets.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Program {
    Texture,
    Solid,
    Shadow,
    BlurDown,
    BlurUp,
}

impl Program {
    fn entry_point(self) -> &'static CStr {
        match self {
            Self::Texture => c"fragment",
            Self::Solid => c"solid",
            Self::Shadow => c"shadow",
            Self::BlurDown => c"blur_down",
            Self::BlurUp => c"blur_up",
        }
    }
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
            REQUIRED_EXTENSIONS
                .iter()
                .all(|required| {
                    extensions
                        .iter()
                        .any(|extension| extension.extension_name_as_c_str() == Ok(*required))
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
                software: properties.device_type == vk::PhysicalDeviceType::CPU,
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

fn target_format(format: Fourcc) -> Option<vk::Format> {
    match format {
        Fourcc::Argb8888 | Fourcc::Xrgb8888 => Some(vk::Format::B8G8R8A8_UNORM),
        Fourcc::Abgr8888 | Fourcc::Xbgr8888 => Some(vk::Format::R8G8B8A8_UNORM),
        _ => None,
    }
}

fn is_opaque(format: Fourcc) -> Option<bool> {
    match format {
        Fourcc::Argb8888 => Some(false),
        Fourcc::Xrgb8888 => Some(true),
        _ => None,
    }
}

fn linear_formats() -> FormatSet {
    TEXTURE_FOURCCS
        .iter()
        .map(|&code| Format {
            code,
            modifier: Modifier::Linear,
        })
        .collect()
}

impl VulkanRenderer {
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
        let extensions = REQUIRED_EXTENSIONS.map(CStr::as_ptr);
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
            external_memory_fd: ash::khr::external_memory_fd::Device::new(&instance, &device),
            queue: unsafe { device.get_device_queue(chosen.family, 0) },
            memory: unsafe { instance.get_physical_device_memory_properties(chosen.physical) },
            reduced_effects: chosen.info.software,
            info: chosen.info,
            command_pool: vk::CommandPool::null(),
            command_buffer: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            sampler: vk::Sampler::null(),
            set_layout: vk::DescriptorSetLayout::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            pipelines: HashMap::new(),
            id: NEXT_RENDERER.fetch_add(1, Ordering::Relaxed),
            context: ContextId::new(),
            debug_flags: DebugFlags::empty(),
            graveyard: Graveyard::default(),
            next_resource: 0,
            textures: HashMap::new(),
            targets: HashMap::new(),
            dmabuf_targets: HashMap::new(),
            dmabuf_textures: HashMap::new(),
            imports: HashMap::new(),
            staging: Vec::new(),
            copies: Vec::new(),
            counters: Counters::default(),
            blur_chains: HashMap::new(),
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

        self.sampler = unsafe {
            device.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                None,
            )
        }?;
        let samplers = [self.sampler];
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
                .immutable_samplers(&samplers),
        ];
        self.set_layout = unsafe {
            device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default()
                    .flags(vk::DescriptorSetLayoutCreateFlags::PUSH_DESCRIPTOR_KHR)
                    .bindings(&bindings),
                None,
            )
        }?;
        let quad = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .size(frame::QUAD_BYTES);
        self.pipeline_layout = unsafe {
            device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(slice::from_ref(&self.set_layout))
                    .push_constant_ranges(slice::from_ref(&quad)),
                None,
            )
        }?;
        for format in TARGET_FORMATS {
            for program in [
                Program::Texture,
                Program::Solid,
                Program::Shadow,
                Program::BlurDown,
                Program::BlurUp,
            ] {
                let pipeline = self.create_pipeline(format, program)?;
                self.pipelines.insert((format, program), pipeline);
            }
        }
        Ok(())
    }

    fn create_pipeline(
        &self,
        format: vk::Format,
        program: Program,
    ) -> Result<vk::Pipeline, RenderError> {
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
                .name(program.entry_point()),
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
        let formats = [format];
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

    fn retire_unused_blur_chains(&mut self) {
        let (kept, retired): (HashMap<_, _>, HashMap<_, _>) = std::mem::take(&mut self.blur_chains)
            .into_iter()
            .partition(|(_, chain)| chain.used);
        for chain in retired.values() {
            chain
                .levels
                .iter()
                .for_each(|image| self.destroy_image(image));
        }
        self.blur_chains = kept;
        for chain in self.blur_chains.values_mut() {
            chain.used = false;
        }
    }

    pub fn device_info(&self) -> &DeviceInfo {
        &self.info
    }

    pub fn reduced_effects(&self) -> bool {
        self.reduced_effects
    }

    pub fn set_reduced_effects(&mut self, reduced: bool) {
        self.reduced_effects = reduced;
    }

    pub fn take_counters(&mut self) -> Counters {
        std::mem::take(&mut self.counters)
    }

    pub fn read_target(&mut self, target: &OffscreenTarget) -> Result<Vec<u8>, RenderError> {
        if target.renderer != self.id {
            return Err(RenderError::ForeignResource);
        }
        let (image, width, height, rendered) = match self.targets.get(&target.id) {
            Some(found) => (
                found.image.image,
                found.image.width,
                found.image.height,
                found.rendered,
            ),
            None => return Err(RenderError::ForeignResource),
        };
        let row_bytes = width.unsigned_abs() as usize * BYTES_PER_PIXEL;
        let bytes = row_bytes * height.unsigned_abs() as usize;
        if !rendered {
            return Ok(vec![0; bytes]);
        }
        let readback = self.create_staging(bytes)?;
        let result = self
            .copy_target_to(&readback, image, width, height)
            .map(|()| {
                let mut pixels = vec![0u8; bytes];
                unsafe {
                    std::ptr::copy_nonoverlapping(readback.mapped, pixels.as_mut_ptr(), bytes)
                };
                pixels
            });
        self.destroy_staging(&readback);
        result
    }

    fn next_id(&mut self) -> u64 {
        self.next_resource += 1;
        self.next_resource
    }

    fn owned(&self, resource: Resource) -> Owned {
        Owned {
            resource,
            graveyard: self.graveyard.clone(),
        }
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

    fn destroy_imported(&self, imported: &Imported) {
        unsafe {
            self.device.destroy_buffer(imported.buffer, None);
            self.device.free_memory(imported.memory, None);
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
        for (_, chain) in std::mem::take(&mut self.blur_chains) {
            chain
                .levels
                .iter()
                .for_each(|image| self.destroy_image(image));
        }
        for staging in std::mem::take(&mut self.staging) {
            self.destroy_staging(&staging);
        }
        self.staging.push(kept);
    }

    fn import_dmabuf_memory(
        &self,
        fd: BorrowedFd,
        usage: vk::BufferUsageFlags,
    ) -> Result<Imported, RenderError> {
        let mut file = File::from(fd.try_clone_to_owned()?);
        let size = file.seek(SeekFrom::End(0))?;
        let mut external = vk::ExternalMemoryBufferCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let buffer = unsafe {
            self.device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .push_next(&mut external),
                None,
            )
        }?;
        match self.bind_dmabuf(buffer, file, size) {
            Ok(memory) => Ok(Imported {
                buffer,
                memory,
                size,
            }),
            Err(error) => {
                unsafe { self.device.destroy_buffer(buffer, None) };
                Err(error)
            }
        }
    }

    fn bind_dmabuf(
        &self,
        buffer: vk::Buffer,
        file: File,
        size: u64,
    ) -> Result<vk::DeviceMemory, RenderError> {
        let requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        if requirements.size > size {
            return Err(RenderError::BufferOutOfBounds);
        }
        let mut properties = vk::MemoryFdPropertiesKHR::default();
        unsafe {
            self.external_memory_fd.get_memory_fd_properties(
                vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
                file.as_raw_fd(),
                &mut properties,
            )
        }?;
        let kind = self.memory_type(
            requirements.memory_type_bits & properties.memory_type_bits,
            vk::MemoryPropertyFlags::empty(),
        )?;
        let fd = file.into_raw_fd();
        let mut import = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
            .fd(fd);
        let memory = unsafe {
            self.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(size)
                    .memory_type_index(kind)
                    .push_next(&mut import),
                None,
            )
        };
        let memory = match memory {
            Ok(memory) => memory,
            Err(error) => {
                unsafe { smithay::reexports::rustix::io::close(fd) };
                return Err(error.into());
            }
        };
        if let Err(error) = unsafe { self.device.bind_buffer_memory(buffer, memory, 0) } {
            unsafe { self.device.free_memory(memory, None) };
            return Err(error.into());
        }
        Ok(memory)
    }

    fn create_target(
        &mut self,
        width: i32,
        height: i32,
        format: Fourcc,
        export: Option<Export>,
    ) -> Result<u64, RenderError> {
        let image = target_format(format)
            .ok_or(RenderError::UnsupportedFormat)
            .and_then(|vk_format| {
                let image = self.create_image(
                    width,
                    height,
                    vk_format,
                    vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
                    vk::ComponentSwizzle::IDENTITY,
                )?;
                Ok((image, vk_format))
            });
        let (image, vk_format) = match image {
            Ok(created) => created,
            Err(error) => {
                return Err(error);
            }
        };
        let id = self.next_id();
        self.targets.insert(
            id,
            Target {
                image,
                format: vk_format,
                rendered: false,
                export,
            },
        );
        Ok(id)
    }

    fn forget_texture(&mut self, id: u64) {
        if let Some(removed) = self.textures.remove(&id) {
            self.destroy_image(&removed.image);
        }
        self.copies.retain(|copy| copy.texture != id);
    }

    fn forget_target(&mut self, id: u64) {
        if let Some(removed) = self.targets.remove(&id) {
            self.destroy_image(&removed.image);
        }
    }

    fn collect_garbage(&mut self) {
        self.dmabuf_textures.retain(|weak, _| !weak.is_gone());
        let gone_targets: Vec<u64> = self
            .dmabuf_targets
            .iter()
            .filter(|(weak, _)| weak.is_gone())
            .map(|(_, &id)| id)
            .collect();
        self.dmabuf_targets.retain(|weak, _| !weak.is_gone());
        for id in gone_targets {
            self.forget_target(id);
        }
        let gone_imports: Vec<WeakDmabuf> = self
            .imports
            .keys()
            .filter(|weak| weak.is_gone())
            .cloned()
            .collect();
        for weak in gone_imports {
            if let Some(imported) = self.imports.remove(&weak) {
                self.copies.retain(|copy| copy.source != imported.buffer);
                self.destroy_imported(&imported);
            }
        }
        let dead = std::mem::take(
            &mut *self
                .graveyard
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for resource in dead {
            match resource {
                Resource::Texture(id) => self.forget_texture(id),
                Resource::Target(id) => self.forget_target(id),
            }
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

    fn begin_commands(&self) -> Result<(), RenderError> {
        unsafe {
            self.device
                .reset_command_buffer(self.command_buffer, vk::CommandBufferResetFlags::empty())?;
            self.device.begin_command_buffer(
                self.command_buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }
        Ok(())
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
                    .buffer_offset(copy.offset)
                    .buffer_row_length(copy.row_texels)
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
                        copy.source,
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
        self.copies.clear();
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
        let region = vk::BufferImageCopy::default()
            .image_subresource(COLOR_LAYERS)
            .image_extent(vk::Extent3D {
                width: width.unsigned_abs(),
                height: height.unsigned_abs(),
                depth: 1,
            });
        self.begin_commands()?;
        barrier(device, commands, image, ATTACHMENT, TRANSFER_SOURCE);
        unsafe {
            device.cmd_copy_image_to_buffer(
                commands,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                readback.buffer,
                slice::from_ref(&region),
            );
        }
        barrier(device, commands, image, TRANSFER_SOURCE, ATTACHMENT);
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

    fn dmabuf_export(&self, dmabuf: &Dmabuf) -> Result<Export, RenderError> {
        let layout = import::single_plane_layout(dmabuf)?;
        target_format(dmabuf.format().code).ok_or(RenderError::UnsupportedFormat)?;
        let fd = layout.fd.try_clone_to_owned()?;
        let length = usize::try_from(File::from(fd.try_clone()?).seek(SeekFrom::End(0))?)
            .map_err(|_| RenderError::BufferOutOfBounds)?;
        let end = usize::try_from(layout.end(dmabuf.size()))
            .map_err(|_| RenderError::BufferOutOfBounds)?;
        if end > length {
            return Err(RenderError::BufferOutOfBounds);
        }
        let mapping = unsafe {
            mm::mmap(
                std::ptr::null_mut(),
                length,
                mm::ProtFlags::READ | mm::ProtFlags::WRITE,
                mm::MapFlags::SHARED,
                &fd,
                0,
            )
        }
        .map_err(std::io::Error::from)?;
        Ok(Export {
            fd,
            mapping: mapping.cast(),
            length,
            offset: layout.offset as usize,
            stride: layout.stride as usize,
        })
    }
}

impl RendererSuper for VulkanRenderer {
    type Error = RenderError;
    type TextureId = VulkanTexture;
    type Framebuffer<'buffer> = VulkanFramebuffer<'buffer>;
    type Frame<'frame, 'buffer>
        = VulkanFrame<'frame, 'buffer>
    where
        'buffer: 'frame,
        Self: 'frame;
}

impl Renderer for VulkanRenderer {
    fn context_id(&self) -> ContextId<VulkanTexture> {
        self.context.clone()
    }

    fn downscale_filter(&mut self, _filter: TextureFilter) -> Result<(), RenderError> {
        Ok(())
    }

    fn upscale_filter(&mut self, _filter: TextureFilter) -> Result<(), RenderError> {
        Ok(())
    }

    fn set_debug_flags(&mut self, flags: DebugFlags) {
        self.debug_flags = flags;
    }

    fn debug_flags(&self) -> DebugFlags {
        self.debug_flags
    }

    fn render<'frame, 'buffer>(
        &'frame mut self,
        framebuffer: &'frame mut VulkanFramebuffer<'buffer>,
        _output_size: Size<i32, Physical>,
        dst_transform: Transform,
    ) -> Result<VulkanFrame<'frame, 'buffer>, RenderError>
    where
        'buffer: 'frame,
    {
        self.collect_garbage();
        VulkanFrame::begin(self, framebuffer, dst_transform)
    }

    fn wait(&mut self, sync: &SyncPoint) -> Result<(), RenderError> {
        sync.wait().map_err(|_| RenderError::Interrupted)
    }

    fn cleanup_texture_cache(&mut self) -> Result<(), RenderError> {
        self.collect_garbage();
        Ok(())
    }
}

impl Offscreen<OffscreenTarget> for VulkanRenderer {
    fn create_buffer(
        &mut self,
        format: Fourcc,
        size: Size<i32, BufferCoords>,
    ) -> Result<OffscreenTarget, RenderError> {
        self.collect_garbage();
        let id = self.create_target(size.w, size.h, format, None)?;
        Ok(OffscreenTarget {
            _owned: self.owned(Resource::Target(id)),
            id,
            renderer: self.id,
            width: size.w,
            height: size.h,
            format,
        })
    }
}

impl Bind<OffscreenTarget> for VulkanRenderer {
    fn bind<'a>(
        &mut self,
        target: &'a mut OffscreenTarget,
    ) -> Result<VulkanFramebuffer<'a>, RenderError> {
        if target.renderer != self.id || !self.targets.contains_key(&target.id) {
            return Err(RenderError::ForeignResource);
        }
        Ok(VulkanFramebuffer {
            id: target.id,
            width: target.width,
            height: target.height,
            format: target.format,
            _target: PhantomData,
        })
    }
}

impl Bind<Dmabuf> for VulkanRenderer {
    fn bind<'a>(&mut self, dmabuf: &'a mut Dmabuf) -> Result<VulkanFramebuffer<'a>, RenderError> {
        let format = dmabuf.format().code;
        let size = dmabuf.size();
        let id = match self.dmabuf_targets.get(&dmabuf.weak()) {
            Some(&id) => id,
            None => {
                let export = self.dmabuf_export(dmabuf)?;
                let id = self.create_target(size.w, size.h, format, Some(export))?;
                self.dmabuf_targets.insert(dmabuf.weak(), id);
                id
            }
        };
        Ok(VulkanFramebuffer {
            id,
            width: size.w,
            height: size.h,
            format,
            _target: PhantomData,
        })
    }

    fn supported_formats(&self) -> Option<FormatSet> {
        Some(linear_formats())
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
        }
        self.dmabuf_textures.clear();
        for (_, texture) in std::mem::take(&mut self.textures) {
            self.destroy_image(&texture.image);
        }
        for (_, target) in std::mem::take(&mut self.targets) {
            self.destroy_image(&target.image);
        }
        for staging in std::mem::take(&mut self.staging) {
            self.destroy_staging(&staging);
        }
        for (_, imported) in std::mem::take(&mut self.imports) {
            self.destroy_imported(&imported);
        }
        unsafe {
            for (_, pipeline) in self.pipelines.drain() {
                self.device.destroy_pipeline(pipeline, None);
            }
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device
                .destroy_descriptor_set_layout(self.set_layout, None);
            self.device.destroy_sampler(self.sampler, None);
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

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

type Usage = (vk::ImageLayout, vk::PipelineStageFlags2, vk::AccessFlags2);

const ATTACHMENT: Usage = (
    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
    vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
);

const TRANSFER_SOURCE: Usage = (
    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
    vk::PipelineStageFlags2::COPY,
    vk::AccessFlags2::TRANSFER_READ,
);

fn alpha_swizzle(opaque: bool) -> vk::ComponentSwizzle {
    if opaque {
        vk::ComponentSwizzle::ONE
    } else {
        vk::ComponentSwizzle::IDENTITY
    }
}

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
