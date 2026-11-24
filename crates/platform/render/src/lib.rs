mod formats;
mod frame;
mod import;
mod rect;
mod submit;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::fmt;
use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::marker::PhantomData;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ash::vk;
use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags, WeakDmabuf};
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Buffer, Format, Fourcc, Modifier};
use smithay::backend::renderer::sync::SyncPoint;
use smithay::backend::renderer::{
    Bind, ContextId, DebugFlags, ImportMem, Offscreen, Renderer, RendererSuper, Texture,
    TextureFilter,
};
use smithay::reexports::rustix::mm;
use smithay::utils::{Buffer as BufferCoords, Physical, Size, Transform};

pub use frame::{Backdrop, Blur, RoundedRect, Shadow, VulkanFrame};
pub use import::set_hdr_content;
pub use rect::{Rect, disjoint};

const SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/compose.spv"));
const BYTES_PER_PIXEL: usize = 4;
const MIN_STAGING_BYTES: usize = 4 << 20;
const GPU_HANG_NANOS: u64 = 5_000_000_000;
const SETS_PER_POOL: u32 = 64;
const MIN_RENDERER_LIFETIME: Duration = Duration::from_secs(10);
const REQUIRED_EXTENSIONS: [&CStr; 2] = [
    ash::khr::external_memory_fd::NAME,
    ash::ext::external_memory_dma_buf::NAME,
];
const PROMOTED_EXTENSIONS: [&CStr; 2] = [
    ash::khr::dynamic_rendering::NAME,
    ash::khr::synchronization2::NAME,
];
const MODIFIER_EXTENSIONS: [&CStr; 2] = [
    ash::ext::image_drm_format_modifier::NAME,
    ash::ext::queue_family_foreign::NAME,
];
const MEMORY_PLANES: [vk::ImageAspectFlags; 4] = [
    vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
    vk::ImageAspectFlags::MEMORY_PLANE_1_EXT,
    vk::ImageAspectFlags::MEMORY_PLANE_2_EXT,
    vk::ImageAspectFlags::MEMORY_PLANE_3_EXT,
];
const TARGET_FORMATS: [vk::Format; 4] = [
    vk::Format::B8G8R8A8_UNORM,
    vk::Format::R8G8B8A8_UNORM,
    vk::Format::A2R10G10B10_UNORM_PACK32,
    vk::Format::A2B10G10R10_UNORM_PACK32,
];
const SDR_WHITE: f32 = 203.0;
const EXTENDED_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;
const PREPARED_FORMATS: [vk::Format; 2] = [vk::Format::B8G8R8A8_UNORM, vk::Format::R8G8B8A8_UNORM];
const PREPARED_PROGRAMS: [Program; 8] = [
    Program::Texture,
    Program::Video,
    Program::DisplayColour,
    Program::Solid,
    Program::Shadow,
    Program::BlurDown,
    Program::BlurUp,
    Program::BlurTone,
];
const TEXTURE_FOURCCS: [Fourcc; 3] = [Fourcc::Argb8888, Fourcc::Xrgb8888, Fourcc::Abgr8888];
const TARGET_FOURCCS: [Fourcc; 4] = [
    Fourcc::Argb8888,
    Fourcc::Xrgb8888,
    Fourcc::Abgr8888,
    Fourcc::Xbgr8888,
];

static NEXT_RENDERER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub enum RenderError {
    Loading(ash::LoadingError),
    Vulkan(vk::Result),
    NoDevice,
    NoMemoryType,
    BufferOutOfBounds,
    ImageTooLarge,
    UnalignedDmabuf,
    UnsupportedFormat,
    UnsupportedTransform,
    UnknownBuffer,
    ForeignResource,
    Interrupted,
    NothingRendered,
    FailedBefore,
    Io(std::io::Error),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loading(error) => write!(f, "cannot load the Vulkan library: {error}"),
            Self::Vulkan(result) => write!(f, "Vulkan call failed: {result}"),
            Self::NoDevice => {
                f.write_str("no usable Vulkan device (each one left out is logged with the reason)")
            }
            Self::NoMemoryType => f.write_str("no suitable Vulkan memory type"),
            Self::BufferOutOfBounds => f.write_str("buffer rows lie outside its pool"),
            Self::ImageTooLarge => f.write_str("the image is larger than the device allows"),
            Self::UnalignedDmabuf => {
                f.write_str("dma-buf offset or stride is not a multiple of the pixel size")
            }
            Self::UnsupportedFormat => f.write_str("pixel format or layout is not supported"),
            Self::UnsupportedTransform => f.write_str("flipped memory uploads are not supported"),
            Self::UnknownBuffer => f.write_str("buffer is neither wl_shm nor a dma-buf"),
            Self::ForeignResource => {
                f.write_str("texture or render target belongs to another renderer")
            }
            Self::Interrupted => f.write_str("waiting for a sync point was interrupted"),
            Self::NothingRendered => f.write_str("no frame has been rendered yet"),
            Self::FailedBefore => f.write_str("this dma-buf failed to import before"),
            Self::Io(error) => write!(f, "cannot size the dma-buf: {error}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl RenderError {
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Io(_)
                | Self::Interrupted
                | Self::Vulkan(
                    vk::Result::ERROR_OUT_OF_DEVICE_MEMORY
                        | vk::Result::ERROR_OUT_OF_HOST_MEMORY
                        | vk::Result::ERROR_TOO_MANY_OBJECTS
                        | vk::Result::ERROR_DEVICE_LOST
                        | vk::Result::TIMEOUT
                )
        )
    }

    pub fn is_device_lost(&self) -> bool {
        matches!(
            self,
            Self::Vulkan(vk::Result::ERROR_DEVICE_LOST | vk::Result::TIMEOUT)
        )
    }
}

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
    pub driver_id: String,
    pub driver_info: String,
    pub api_version: String,
    pub extensions: Vec<String>,
    pub software: bool,
    pub max_image_side: u32,
    pub render_device: Option<u64>,
}

impl fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}, {} {}, Vulkan {})",
            self.name, self.driver_id, self.driver, self.driver_info, self.api_version
        )
    }
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
    chroma: Option<Arc<VulkanTexture>>,
    hdr: Option<HdrContent>,
    renderer: u64,
    width: i32,
    height: i32,
    format: Fourcc,
}

impl VulkanTexture {
    pub fn is_hdr(&self) -> bool {
        self.hdr.is_some()
    }

    /// The same texture with its pixels read as `content`.
    pub fn showing(self, content: Option<HdrContent>) -> Self {
        Self {
            hdr: content,
            ..self
        }
    }
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
    colour: Option<OutputColour>,
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
    memory: Vec<vk::DeviceMemory>,
    view: vk::ImageView,
    width: i32,
    height: i32,
}

struct TextureImage {
    image: Image,
    readable: bool,
    imported: bool,
}

#[derive(Clone, Copy)]
struct Imported {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: u64,
}

struct Export {
    mapped: MappedExport,
    device: Option<DeviceExport>,
    trusted: bool,
}

#[derive(Clone, Copy)]
struct DeviceExport {
    imported: Imported,
    offset: u64,
    row_texels: u32,
}

struct MappedExport {
    fd: OwnedFd,
    mapping: *mut u8,
    length: usize,
    offset: usize,
    stride: usize,
}

impl Drop for MappedExport {
    fn drop(&mut self) {
        // SAFETY: `mapping` and `length` are exactly what `mmap` returned in `mapped_export`, and
        // this value is the mapping's only owner, so nothing reads it afterwards.
        let _ = unsafe { mm::munmap(self.mapping.cast(), self.length) };
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputColour {
    pub matrix: [[f32; 3]; 3],
    pub encode: Encode,
    pub extended: bool,
}

/// How linear colours, 1.0 being the white of ordinary content, leave for the display.
#[derive(Debug, Clone, PartialEq)]
pub enum Encode {
    Srgb,
    Table(Arc<[[u16; 3]]>),
    Pq { white: f32, peak: f32 },
}

/// PQ-encoded BT.2020 content whose brightest pixel is `max_nits`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HdrContent {
    pub max_nits: f32,
}

struct UnconvertedFrame {
    image: vk::Image,
    view: vk::ImageView,
    format: vk::Format,
    drawn: bool,
}

struct Unconverted {
    image: Image,
    format: vk::Format,
    drawn: bool,
}

struct Target {
    image: Image,
    format: vk::Format,
    rendered: bool,
    export: Option<Export>,
    direct: bool,
    unconverted: Option<Unconverted>,
}

#[derive(Default)]
struct Descriptors {
    pools: Vec<(vk::DescriptorPool, u32)>,
    sets: HashMap<(vk::ImageView, vk::ImageView), (vk::DescriptorSet, vk::DescriptorPool)>,
}

struct Staging {
    last_used: Instant,
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MemoryUse {
    pub textures: (usize, u64),
    pub targets: (usize, u64),
    pub imports: (usize, u64),
    pub staging: (usize, u64),
    pub blur_levels: (usize, u64),
    pub heap_size: u64,
}

impl fmt::Display for MemoryUse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const MEBIBYTE: u64 = 1 << 20;
        for (name, (count, bytes)) in [
            ("textures", self.textures),
            ("targets", self.targets),
            ("imports", self.imports),
            ("staging", self.staging),
            ("blur levels", self.blur_levels),
        ] {
            write!(f, "{count} {name} {} MiB, ", bytes / MEBIBYTE)?;
        }
        write!(f, "of a {} MiB heap", self.heap_size / MEBIBYTE)
    }
}

pub struct VulkanRenderer {
    instance: ash::Instance,
    device: ash::Device,
    external_memory_fd: ash::khr::external_memory_fd::Device,
    queue: vk::Queue,
    queue_family: u32,
    foreign_family: u32,
    output_on_device: bool,
    modifiers: formats::Modifiers,
    refused: Vec<Format>,
    unproven: Vec<(vk::ImageUsageFlags, Format)>,
    proven: bool,
    output_colour: Option<OutputColour>,
    encode_tables: Vec<(Arc<[[u16; 3]]>, VulkanTexture)>,
    colour_scratch: HashMap<(i32, i32, vk::Format), Image>,
    memory: vk::PhysicalDeviceMemoryProperties,
    info: DeviceInfo,
    reduced_effects: bool,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    submissions: submit::Submissions,
    garbage: RefCell<Vec<(u64, submit::Garbage)>>,
    external_semaphore_fd: Option<ash::khr::external_semaphore_fd::Device>,
    timestamp_period: Option<f32>,
    timestamp_bits: u32,
    sampler: vk::Sampler,
    set_layout: vk::DescriptorSetLayout,
    descriptors: RefCell<Descriptors>,
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
    sampled_dmabufs: HashMap<WeakDmabuf, VulkanTexture>,
    failed_dmabufs: HashSet<WeakDmabuf>,
    linear_feedback: Option<smithay::wayland::dmabuf::DmabufFeedback>,
    linear_surfaces: Vec<smithay::wayland::dmabuf::SurfaceDmabufFeedbackState>,
    imports: HashMap<WeakDmabuf, Imported>,
    staging: Vec<Staging>,
    copies: Vec<PendingCopy>,
    counters: Counters,
    blur_chains: HashMap<u64, frame::BlurChain>,
    lost: AtomicBool,
    exhausted: AtomicBool,
    renewable_from: Instant,
    messenger: Option<(ash::ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    display: Option<u64>,
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
    Video,
    Hdr,
    HdrVideo,
    Solid,
    Shadow,
    BlurDown,
    BlurUp,
    BlurTone,
    DisplayColour,
}

impl Program {
    fn entry_point(self) -> &'static CStr {
        match self {
            Self::Texture => c"fragment",
            Self::Video => c"video",
            Self::Hdr => c"hdr",
            Self::HdrVideo => c"hdr_video",
            Self::DisplayColour => c"display_colour",
            Self::Solid => c"solid",
            Self::Shadow => c"shadow",
            Self::BlurDown => c"blur_down",
            Self::BlurUp => c"blur_up",
            Self::BlurTone => c"blur_tone",
        }
    }
}

struct Proven {
    info: DeviceInfo,
    modifiers: formats::Modifiers,
    refused: Vec<Format>,
}

struct Chosen {
    physical: vk::PhysicalDevice,
    family: u32,
    sync_files: bool,
    timestamp_period: Option<f32>,
    timestamp_bits: u32,
    output_on_device: bool,
    modifiers: formats::Modifiers,
    extensions: Vec<&'static CStr>,
    info: DeviceInfo,
}

fn device_rank(kind: vk::PhysicalDeviceType, drives_display: bool) -> u8 {
    if drives_display {
        return 0;
    }
    match kind {
        vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
        vk::PhysicalDeviceType::DISCRETE_GPU => 2,
        vk::PhysicalDeviceType::VIRTUAL_GPU => 3,
        vk::PhysicalDeviceType::CPU => 4,
        _ => 5,
    }
}

fn node_of(major: i64, minor: i64) -> u64 {
    smithay::reexports::rustix::fs::makedev(major as u32, minor as u32)
}

fn text(name: Result<&CStr, std::ffi::FromBytesUntilNulError>) -> String {
    name.map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn examine(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    display: Option<u64>,
) -> Result<(u8, Chosen), (String, String)> {
    // SAFETY: `physical` was just enumerated from this live `instance`.
    let properties = unsafe { instance.get_physical_device_properties(physical) };
    let name = text(properties.device_name_as_c_str());
    let refuse = |reason: String| (name.clone(), reason);
    let version = properties.api_version;
    if version < vk::API_VERSION_1_2 {
        return Err(refuse(format!(
            "it offers Vulkan {}.{}, and 1.2 is needed",
            vk::api_version_major(version),
            vk::api_version_minor(version)
        )));
    }
    // SAFETY: `physical` was just enumerated from this live `instance`.
    let extensions = unsafe { instance.enumerate_device_extension_properties(physical) }
        .map_err(|error| refuse(format!("its extensions cannot be listed: {error}")))?;
    let offers = |wanted: &CStr| {
        extensions
            .iter()
            .any(|extension| extension.extension_name_as_c_str() == Ok(wanted))
    };
    let promoted: &[&CStr] = match version < vk::API_VERSION_1_3 {
        true => &PROMOTED_EXTENSIONS,
        false => &[],
    };
    let missing: Vec<_> = REQUIRED_EXTENSIONS
        .iter()
        .chain(promoted)
        .filter(|&&required| !offers(required))
        .map(|required| required.to_string_lossy())
        .collect();
    if !missing.is_empty() {
        return Err(refuse(format!("it lacks {}", missing.join(", "))));
    }
    let mut rendering = vk::PhysicalDeviceDynamicRenderingFeatures::default();
    let mut synchronization = vk::PhysicalDeviceSynchronization2Features::default();
    let mut features = vk::PhysicalDeviceFeatures2::default()
        .push_next(&mut rendering)
        .push_next(&mut synchronization);
    // SAFETY: `physical` belongs to this `instance`, which is Vulkan 1.3, and the chained structs
    // outlive the call.
    unsafe { instance.get_physical_device_features2(physical, &mut features) };
    if rendering.dynamic_rendering != vk::TRUE || synchronization.synchronization2 != vk::TRUE {
        return Err(refuse(
            "it lacks dynamic rendering or synchronization2".to_owned(),
        ));
    }
    // SAFETY: `physical` was just enumerated from this live `instance`.
    let families = unsafe { instance.get_physical_device_queue_family_properties(physical) };
    let (family, timed) = families
        .iter()
        .zip(0u32..)
        .find(|(family, _)| family.queue_flags.contains(vk::QueueFlags::GRAPHICS))
        .map(|(properties, family)| (family, properties.timestamp_valid_bits))
        .ok_or_else(|| refuse("it has no graphics queue".to_owned()))?;
    let sync_files = offers(ash::khr::external_semaphore_fd::NAME) && {
        let mut external = vk::ExternalSemaphoreProperties::default();
        // SAFETY: `physical` belongs to this Vulkan 1.1 or later `instance`, and `external`
        // outlives the call.
        unsafe {
            instance.get_physical_device_external_semaphore_properties(
                physical,
                &vk::PhysicalDeviceExternalSemaphoreInfo::default()
                    .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD),
                &mut external,
            )
        };
        external
            .external_semaphore_features
            .contains(vk::ExternalSemaphoreFeatureFlags::EXPORTABLE)
    };
    let mut driver = vk::PhysicalDeviceDriverProperties::default();
    let mut node = vk::PhysicalDeviceDrmPropertiesEXT::default();
    let mut properties2 = vk::PhysicalDeviceProperties2::default().push_next(&mut driver);
    if offers(ash::ext::physical_device_drm::NAME) {
        properties2 = properties2.push_next(&mut node);
    }
    // SAFETY: `physical` belongs to this `instance`; `node` is chained only when the device
    // lists `VK_EXT_physical_device_drm`, and both chained structs outlive the call.
    unsafe { instance.get_physical_device_properties2(physical, &mut properties2) };
    let on_real_gpu = matches!(
        properties.device_type,
        vk::PhysicalDeviceType::INTEGRATED_GPU | vk::PhysicalDeviceType::DISCRETE_GPU
    );
    let info = DeviceInfo {
        name,
        driver: text(driver.driver_name_as_c_str()),
        driver_id: format!("{:?}", driver.driver_id),
        driver_info: text(driver.driver_info_as_c_str()),
        api_version: format!(
            "{}.{}.{}",
            vk::api_version_major(version),
            vk::api_version_minor(version),
            vk::api_version_patch(version)
        ),
        extensions: Vec::new(),
        software: properties.device_type == vk::PhysicalDeviceType::CPU,
        max_image_side: properties.limits.max_image_dimension2_d,
        render_device: (node.has_render == vk::TRUE)
            .then(|| node_of(node.render_major, node.render_minor)),
    };
    let primary =
        (node.has_primary == vk::TRUE).then(|| node_of(node.primary_major, node.primary_minor));
    let drives_display =
        display.is_some_and(|display| [primary, info.render_device].contains(&Some(display)));
    let modifiers = !info.software && MODIFIER_EXTENSIONS.iter().all(|&wanted| offers(wanted));
    let extensions = REQUIRED_EXTENSIONS
        .iter()
        .chain(promoted)
        .chain(
            modifiers
                .then_some(&MODIFIER_EXTENSIONS)
                .into_iter()
                .flatten(),
        )
        .chain(sync_files.then_some(&ash::khr::external_semaphore_fd::NAME))
        .copied()
        .collect();
    Ok((
        device_rank(properties.device_type, drives_display),
        Chosen {
            physical,
            family,
            sync_files,
            timestamp_period: (timed > 0).then_some(properties.limits.timestamp_period),
            timestamp_bits: timed,
            output_on_device: on_real_gpu,
            modifiers: match modifiers {
                true => formats::query(instance, physical),
                false => formats::Modifiers::default(),
            },
            extensions,
            info,
        },
    ))
}

fn choose_device(instance: &ash::Instance, display: Option<u64>) -> Result<Chosen, RenderError> {
    // SAFETY: `instance` is a live instance, not destroyed before `choose_device` returns.
    let physicals = unsafe { instance.enumerate_physical_devices() }?;
    physicals
        .into_iter()
        .filter_map(|physical| {
            examine(instance, physical, display)
                .inspect_err(|(name, reason)| {
                    tracing::warn!("not composing on {name}: {reason}");
                })
                .ok()
        })
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, chosen)| chosen)
        .ok_or(RenderError::NoDevice)
}

fn target_format(format: Fourcc) -> Option<vk::Format> {
    match format {
        Fourcc::Argb8888 | Fourcc::Xrgb8888 => Some(vk::Format::B8G8R8A8_UNORM),
        Fourcc::Abgr8888 | Fourcc::Xbgr8888 => Some(vk::Format::R8G8B8A8_UNORM),
        Fourcc::Abgr2101010 => Some(vk::Format::A2B10G10R10_UNORM_PACK32),
        _ => None,
    }
}

fn is_deep_target(format: vk::Format) -> bool {
    !PREPARED_FORMATS.contains(&format)
}

fn is_opaque(format: Fourcc) -> Option<bool> {
    match format {
        Fourcc::Argb8888 | Fourcc::Abgr8888 | Fourcc::Abgr2101010 => Some(false),
        Fourcc::Xrgb8888 => Some(true),
        _ => None,
    }
}

fn linear(codes: &[Fourcc]) -> impl Iterator<Item = Format> + '_ {
    codes.iter().map(|&code| Format {
        code,
        modifier: Modifier::Linear,
    })
}

type Proof = Result<(Image, Option<vk::ImageView>), (&'static str, RenderError)>;

impl VulkanRenderer {
    pub fn new(display: Option<u64>) -> Result<Self, RenderError> {
        Self::create(display, None)
    }

    pub fn renew(&self) -> Result<Self, RenderError> {
        let proven = self.proven.then(|| Proven {
            info: self.info.clone(),
            modifiers: self.modifiers.clone(),
            refused: self.refused.clone(),
        });
        Self::create(self.display, proven)
    }

    fn create(display: Option<u64>, proven: Option<Proven>) -> Result<Self, RenderError> {
        // SAFETY: loading runs the system Vulkan loader's initialisers once; the returned `Entry`
        // keeps the library loaded for as long as the renderer holds it in `_entry`.
        let entry = unsafe { ash::Entry::load() }?;
        let application = vk::ApplicationInfo::default()
            .application_name(c"bande-compositor")
            .api_version(vk::API_VERSION_1_3);
        // SAFETY: `entry` is the loaded library, queried before any instance exists.
        let reports = unsafe { entry.enumerate_instance_extension_properties(None) }?
            .iter()
            .any(|extension| {
                extension.extension_name_as_c_str() == Ok(ash::ext::debug_utils::NAME)
            });
        let mut report_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
            .message_severity(
                vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                    | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
            )
            .message_type(
                vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                    | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                    | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
            )
            .pfn_user_callback(Some(report));
        let instance_extensions: Vec<_> = reports
            .then_some(ash::ext::debug_utils::NAME.as_ptr())
            .into_iter()
            .collect();
        let mut instance_info = vk::InstanceCreateInfo::default()
            .application_info(&application)
            .enabled_extension_names(&instance_extensions);
        if reports {
            instance_info = instance_info.push_next(&mut report_info);
        }
        // SAFETY: `application`, the extension names and `report_info` outlive the call; the
        // instance is destroyed only by `forget_instance` on the error paths below or by `Drop`.
        let instance = unsafe { entry.create_instance(&instance_info, None) }?;
        let messenger = reports
            .then(|| {
                let utils = ash::ext::debug_utils::Instance::new(&entry, &instance);
                // SAFETY: the instance was created with `VK_EXT_debug_utils`, and `report_info` names a
                // callback that lives for the whole program.
                let created = unsafe { utils.create_debug_utils_messenger(&report_info, None) };
                created
                    .inspect_err(|error| tracing::warn!("Vulkan cannot report problems: {error}"))
                    .ok()
                    .map(|messenger| (utils, messenger))
            })
            .flatten();
        let chosen = match choose_device(&instance, display) {
            Ok(chosen) => chosen,
            Err(error) => {
                forget_instance(&instance, messenger.as_ref());
                return Err(error);
            }
        };
        let priorities = [1.0];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(chosen.family)
            .queue_priorities(&priorities);
        let mut rendering =
            vk::PhysicalDeviceDynamicRenderingFeatures::default().dynamic_rendering(true);
        let mut synchronization =
            vk::PhysicalDeviceSynchronization2Features::default().synchronization2(true);
        let extensions: Vec<_> = chosen.extensions.iter().map(|name| name.as_ptr()).collect();
        // SAFETY: `chosen.physical` belongs to `instance` and supports both features;
        // `queue_info`, `priorities`, the feature structs and `extensions` (pointers into
        // `'static` names) outlive the call.
        let created = unsafe {
            instance.create_device(
                chosen.physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(slice::from_ref(&queue_info))
                    .enabled_extension_names(&extensions)
                    .push_next(&mut rendering)
                    .push_next(&mut synchronization),
                None,
            )
        };
        let device = match created {
            // SAFETY: `device` was just created from `instance`; a Vulkan 1.2 device resolves the
            // core 1.3 commands Bande uses through the `KHR` extensions enabled above, which have
            // the same signatures.
            Ok(device) => unsafe { promoted_commands(&instance, device.handle()) },
            Err(error) => {
                forget_instance(&instance, messenger.as_ref());
                return Err(error.into());
            }
        };
        let mut info = chosen.info;
        info.extensions = chosen
            .extensions
            .iter()
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        let mut renderer = Self {
            external_memory_fd: ash::khr::external_memory_fd::Device::new(&instance, &device),
            // SAFETY: the device was created with one queue in `chosen.family`, so queue 0 of that
            // family exists.
            queue: unsafe { device.get_device_queue(chosen.family, 0) },
            queue_family: chosen.family,
            foreign_family: match chosen.modifiers.any() {
                true => vk::QUEUE_FAMILY_FOREIGN_EXT,
                false => vk::QUEUE_FAMILY_EXTERNAL,
            },
            output_on_device: chosen.output_on_device,
            modifiers: chosen.modifiers,
            refused: Vec::new(),
            unproven: Vec::new(),
            proven: false,
            output_colour: None,
            encode_tables: Vec::new(),
            colour_scratch: HashMap::new(),
            // SAFETY: `chosen.physical` belongs to the live `instance`.
            memory: unsafe { instance.get_physical_device_memory_properties(chosen.physical) },
            reduced_effects: info.software,
            info,
            command_pool: vk::CommandPool::null(),
            command_buffer: vk::CommandBuffer::null(),
            submissions: submit::Submissions::new(),
            garbage: RefCell::default(),
            external_semaphore_fd: chosen
                .sync_files
                .then(|| ash::khr::external_semaphore_fd::Device::new(&instance, &device)),
            timestamp_period: chosen.timestamp_period,
            timestamp_bits: chosen.timestamp_bits,
            sampler: vk::Sampler::null(),
            set_layout: vk::DescriptorSetLayout::null(),
            descriptors: RefCell::default(),
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
            sampled_dmabufs: HashMap::new(),
            failed_dmabufs: HashSet::new(),
            linear_feedback: None,
            linear_surfaces: Vec::new(),
            imports: HashMap::new(),
            staging: Vec::new(),
            copies: Vec::new(),
            counters: Counters::default(),
            blur_chains: HashMap::new(),
            lost: AtomicBool::new(false),
            exhausted: AtomicBool::new(false),
            renewable_from: Instant::now() + MIN_RENDERER_LIFETIME,
            messenger,
            display,
            instance,
            device,
            _entry: entry,
        };
        renderer.create_objects(chosen.family)?;
        match proven {
            Some(proven) if proven.info == renderer.info => {
                renderer.modifiers = proven.modifiers;
                renderer.refused = proven.refused;
                renderer.proven = true;
            }
            _ => renderer.prove_modifiers(),
        }
        Ok(renderer)
    }

    fn create_objects(&mut self, family: u32) -> Result<(), RenderError> {
        let device = &self.device;
        // SAFETY: `family` is the queue family the device was created with; the pool is destroyed
        // only in `Drop`, after the device is idle.
        self.command_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
        }?;

        // SAFETY: the sampler is destroyed only in `Drop`, after the pipeline layout and the
        // descriptor set layout that use it as an immutable sampler.
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
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        // SAFETY: `bindings` and `samplers` outlive the call.
        self.set_layout = unsafe {
            device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )
        }?;
        let quad = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .size(frame::QUAD_BYTES);
        // SAFETY: `set_layout` was created just above and `quad` covers exactly the `QUAD_BYTES`
        // every draw pushes.
        self.pipeline_layout = unsafe {
            device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(slice::from_ref(&self.set_layout))
                    .push_constant_ranges(slice::from_ref(&quad)),
                None,
            )
        }?;
        for format in PREPARED_FORMATS {
            self.prepare_pipelines(format)?;
        }
        Ok(())
    }

    /// Makes what drawing into `format` needs, ahead of the first frame.
    pub fn prepare(&mut self, format: Fourcc) -> Result<(), RenderError> {
        let texel = formats::texel(format).ok_or(RenderError::UnsupportedFormat)?;
        self.prepare_pipelines(texel.format)
    }

    /// Makes what drawing HDR content needs, ahead of the first frame with it.
    pub fn prepare_hdr(&mut self) -> Result<(), RenderError> {
        self.prepare_pipelines(EXTENDED_FORMAT)?;
        for program in [Program::Hdr, Program::HdrVideo] {
            self.prepare_pipeline(EXTENDED_FORMAT, program)?;
        }
        Ok(())
    }

    fn prepare_pipeline(
        &mut self,
        format: vk::Format,
        program: Program,
    ) -> Result<vk::Pipeline, RenderError> {
        if let Some(pipeline) = self.pipelines.get(&(format, program)) {
            return Ok(*pipeline);
        }
        let pipeline = self.create_pipeline(format, program)?;
        self.pipelines.insert((format, program), pipeline);
        Ok(pipeline)
    }

    fn prepare_pipelines(&mut self, format: vk::Format) -> Result<(), RenderError> {
        for program in PREPARED_PROGRAMS {
            self.prepare_pipeline(format, program)?;
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
        // SAFETY: `words` is the build script's SPIR-V as properly aligned `u32`s and outlives the
        // call.
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
            .blend_enable(!matches!(program, Program::BlurDown | Program::BlurUp))
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
        // SAFETY: `module`, `pipeline_layout` and every state struct `info` points to are alive
        // until the call returns.
        let pipelines = unsafe {
            self.device.create_graphics_pipelines(
                vk::PipelineCache::null(),
                slice::from_ref(&info),
                None,
            )
        };
        // SAFETY: pipeline creation has returned, so nothing references `module` any more.
        unsafe { self.device.destroy_shader_module(module, None) };
        pipelines
            .map_err(|(_, result)| RenderError::Vulkan(result))?
            .into_iter()
            .next()
            .ok_or(RenderError::Vulkan(vk::Result::ERROR_UNKNOWN))
    }

    fn forget_blurs(&mut self) {
        for chain in std::mem::take(&mut self.blur_chains).into_values() {
            chain
                .levels
                .iter()
                .for_each(|image| self.destroy_image(image));
        }
    }

    fn retire_unused_blur_chains(&mut self) {
        let (kept, retired): (HashMap<_, _>, HashMap<_, _>) = std::mem::take(&mut self.blur_chains)
            .into_iter()
            .partition(|(_, chain)| chain.last_used.elapsed() < frame::KEEP_UNUSED);
        for chain in retired.values() {
            chain
                .levels
                .iter()
                .for_each(|image| self.destroy_image(image));
        }
        self.blur_chains = kept;
    }

    /// Draws with every dma-buf bound by `draw` shown in `colour`; other targets stay sRGB.
    pub fn converting<T>(
        &mut self,
        colour: Option<&OutputColour>,
        draw: impl FnOnce(&mut Self) -> T,
    ) -> Result<T, RenderError> {
        if let Some(colour) = colour {
            self.load_encode_table(colour)?;
        }
        self.output_colour = colour.cloned();
        let drawn = draw(self);
        self.output_colour = None;
        Ok(drawn)
    }

    fn load_encode_table(&mut self, colour: &OutputColour) -> Result<(), RenderError> {
        let Encode::Table(table) = &colour.encode else {
            return Ok(());
        };
        self.encode_tables
            .retain(|(loaded, _)| Arc::strong_count(loaded) > 1);
        if self.encode_table(table).is_some() {
            return Ok(());
        }
        let ten = |value: u16| (u32::from(value) * 1023 + 32767) / 65535;
        let pixels: Vec<u8> = table
            .iter()
            .map(|channels| channels.map(ten))
            .flat_map(|[red, green, blue]| (3 << 30 | blue << 20 | green << 10 | red).to_le_bytes())
            .collect();
        let size = (table.len() as i32, 1).into();
        let texture = self.import_memory(&pixels, Fourcc::Abgr2101010, size, false)?;
        self.encode_tables.push((table.clone(), texture));
        Ok(())
    }

    fn encode_table(&self, table: &Arc<[[u16; 3]]>) -> Option<vk::ImageView> {
        self.encode_tables
            .iter()
            .find(|(loaded, _)| Arc::ptr_eq(loaded, table))
            .and_then(|(_, texture)| self.textures.get(&texture.id))
            .map(|texture| texture.image.view)
    }

    pub fn dmabuf_planes(&self) -> HashMap<Format, usize> {
        linear(&TEXTURE_FOURCCS)
            .map(|format| (format, 1))
            .chain(linear(&[Fourcc::Nv12]).map(|format| (format, 2)))
            .chain(self.modifiers.sampled.planes())
            .collect()
    }

    pub fn refused_formats(&self) -> &[Format] {
        &self.refused
    }

    pub fn render_formats(&self) -> FormatSet {
        linear(&TARGET_FOURCCS)
            .chain(self.modifiers.rendered.formats())
            .collect()
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
                // SAFETY: `copy_target_to` waited for the copy and its host-read barrier, and
                // `readback` maps at least `bytes` bytes, as does `pixels`; the two never overlap.
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
        if width.unsigned_abs().max(height.unsigned_abs()) > self.info.max_image_side {
            return Err(RenderError::ImageTooLarge);
        }
        // SAFETY: the create info outlives the call and the size was checked against
        // `max_image_side`; the image is destroyed by `destroy_image` on failure or when forgotten.
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
            memory: Vec::new(),
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

    pub(crate) fn import_image(
        &self,
        dmabuf: &Dmabuf,
        texel: formats::Texel,
        (usage, capability): (vk::ImageUsageFlags, formats::Capability),
    ) -> Result<Image, RenderError> {
        let (width, height) = (dmabuf.size().w, dmabuf.size().h);
        let (max_width, max_height) = capability.max_extent;
        if width <= 0
            || height <= 0
            || width.max(height).unsigned_abs() > self.info.max_image_side
            || width.unsigned_abs() > max_width
            || height.unsigned_abs() > max_height
        {
            return Err(RenderError::ImageTooLarge);
        }
        let planes = import::dmabuf_planes(dmabuf, capability, texel)?;
        let mut explicit = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
            .drm_format_modifier(u64::from(dmabuf.format().modifier))
            .plane_layouts(&planes.layouts);
        let mut external = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let view_formats = texel.view_formats();
        let mut list = vk::ImageFormatListCreateInfo::default().view_formats(&view_formats);
        let flags = match planes.disjoint {
            true => texel.flags() | vk::ImageCreateFlags::DISJOINT,
            false => texel.flags(),
        };
        let mut info = vk::ImageCreateInfo::default()
            .flags(flags)
            .image_type(vk::ImageType::TYPE_2D)
            .format(texel.format)
            .extent(vk::Extent3D {
                width: width.unsigned_abs(),
                height: height.unsigned_abs(),
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut explicit)
            .push_next(&mut external);
        if !view_formats.is_empty() {
            info = info.push_next(&mut list);
        }
        // SAFETY: the format, modifier and usage are ones the device reported importable through
        // `VK_EXT_image_drm_format_modifier` (enabled whenever any modifier is), and `dmabuf_planes`
        // gave exactly the modifier's `drmFormatModifierPlaneCount` layouts with zero size and
        // pitches; `DISJOINT` is set only when the modifier supports it, the extent is within the
        // device's limits, and every struct chained into `info` outlives the call.
        let image = unsafe { self.device.create_image(&info, None) }?;
        let mut created = Image {
            image,
            memory: Vec::new(),
            view: vk::ImageView::null(),
            width,
            height,
        };
        match self.bind_imported_image(&mut created, planes.objects, planes.disjoint) {
            Ok(()) => Ok(created),
            Err(error) => {
                self.destroy_image(&created);
                Err(error)
            }
        }
    }

    fn prove_modifiers(&mut self) {
        let started = Instant::now();
        let pairs: Vec<_> = [formats::SAMPLED, formats::RENDERED]
            .into_iter()
            .flat_map(|usage| {
                self.supported(usage)
                    .listed()
                    .map(move |(format, capability)| (usage, format, capability))
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut proofs = Vec::new();
        for (usage, format, capability) in pairs {
            let Some(texel) = formats::texel(format.code) else {
                continue;
            };
            let exported = capability
                .exportable
                .then(|| self.exported(format, texel, (usage, capability)).ok())
                .flatten();
            let Some((source, dmabuf)) = exported else {
                self.unproven.push((usage, format));
                continue;
            };
            let proof = self.prove(&dmabuf, texel, (usage, capability));
            self.destroy_image(&source);
            proofs.push((format, usage, proof));
        }
        let proved = proofs.len();
        self.hand_over_proofs(proofs);
        self.proven = !self.is_lost();
        tracing::info!(
            "proved {proved} dma-buf formats on {} in {:.1?}, {} left to prove by allocation",
            self.info.name,
            started.elapsed(),
            self.unproven.len()
        );
    }

    pub fn prove_unexported(&mut self, mut allocate: impl FnMut(Format) -> Option<Dmabuf>) {
        let mut proofs = Vec::new();
        for (usage, format) in std::mem::take(&mut self.unproven) {
            let (Some(texel), Some(capability)) = (
                formats::texel(format.code),
                self.supported(usage).usable(&format),
            ) else {
                continue;
            };
            let proof = match allocate(format) {
                Some(dmabuf) => self.prove(&dmabuf, texel, (usage, capability)),
                None if format.modifier == Modifier::Linear => continue,
                None => Err(("allocate (GBM)", RenderError::UnsupportedFormat)),
            };
            proofs.push((format, usage, proof));
        }
        self.hand_over_proofs(proofs);
        self.proven &= !self.is_lost();
    }

    fn hand_over_proofs(&mut self, proofs: Vec<(Format, vk::ImageUsageFlags, Proof)>) {
        let mut images = Vec::new();
        for (format, usage, proof) in proofs {
            match proof {
                Ok(proved) => images.push((usage, proved)),
                Err(failure) => self.judge(format, usage, failure),
            }
        }
        let handed = self.begin_commands().and_then(|()| {
            for (usage, (image, _)) in &images {
                let using = match *usage {
                    formats::SAMPLED => SAMPLED,
                    _ => DRAWING,
                };
                for acquire in [true, false] {
                    hand_over(
                        &self.device,
                        self.command_buffer,
                        image.image,
                        self.queue_family,
                        (acquire, using),
                    );
                }
            }
            self.submit_and_wait()
        });
        if let Err(error) = handed {
            tracing::warn!(
                "{} could not hand {} proven images to the foreign queue ({error}); keeping them",
                self.info,
                images.len()
            );
        }
        if self.is_lost() {
            return;
        }
        for (_, (image, chroma)) in images {
            if let Some(chroma) = chroma {
                // SAFETY: the device is not lost, so the submission that used the image finished
                // before `submit_and_wait` returned, and nothing else holds `chroma`.
                unsafe { self.device.destroy_image_view(chroma, None) };
            }
            self.destroy_image(&image);
        }
        self.exhausted.store(false, Ordering::Relaxed);
    }

    fn supported(&self, usage: vk::ImageUsageFlags) -> &formats::Supported {
        match usage {
            formats::SAMPLED => &self.modifiers.sampled,
            _ => &self.modifiers.rendered,
        }
    }

    fn judge(
        &mut self,
        format: Format,
        usage: vk::ImageUsageFlags,
        (stage, error): (&'static str, RenderError),
    ) {
        if error.is_transient() || self.is_lost() {
            tracing::warn!(
                "keeping {:?} with modifier {:?} unproven: {stage} failed for now ({error})",
                format.code,
                format.modifier
            );
            return;
        }
        let action = match usage {
            formats::SAMPLED => "sampling",
            _ => "rendering",
        };
        tracing::warn!(
            "{} lists {:?} with modifier {:?} for {action} but cannot {stage} it ({error}); not \
             offering it",
            self.info,
            format.code,
            format.modifier,
        );
        match usage {
            formats::SAMPLED => self.modifiers.sampled.remove(&format),
            _ => self.modifiers.rendered.remove(&format),
        }
        if !self.refused.contains(&format) {
            self.refused.push(format);
        }
    }

    fn prove(
        &self,
        dmabuf: &Dmabuf,
        texel: formats::Texel,
        (usage, capability): (vk::ImageUsageFlags, formats::Capability),
    ) -> Proof {
        let mut image = self
            .import_image(dmabuf, texel, (usage, capability))
            .map_err(|error| ("import", error))?;
        let views = match usage {
            formats::SAMPLED => self.sampled_views(image.image, texel),
            _ => self
                .create_view(image.image, texel.format, vk::ComponentSwizzle::IDENTITY)
                .map(|view| (view, None)),
        };
        match views {
            Ok((view, chroma)) => {
                image.view = view;
                Ok((image, chroma))
            }
            Err(error) => {
                self.destroy_image(&image);
                Err(("view", error))
            }
        }
    }

    fn exported(
        &self,
        format: Format,
        texel: formats::Texel,
        (usage, capability): (vk::ImageUsageFlags, formats::Capability),
    ) -> Result<(Image, Dmabuf), RenderError> {
        const SIDE: u32 = 64;
        let modifiers = [u64::from(format.modifier)];
        let mut list =
            vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(&modifiers);
        let mut external = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let view_formats = texel.view_formats();
        let mut views = vk::ImageFormatListCreateInfo::default().view_formats(&view_formats);
        let mut info = vk::ImageCreateInfo::default()
            .flags(texel.flags())
            .image_type(vk::ImageType::TYPE_2D)
            .format(texel.format)
            .extent(vk::Extent3D {
                width: SIDE,
                height: SIDE,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut list)
            .push_next(&mut external);
        if !view_formats.is_empty() {
            info = info.push_next(&mut views);
        }
        // SAFETY: the device reported this format, modifier and usage as an exportable dma-buf
        // image, and every struct chained into `info` outlives the call.
        let image = unsafe { self.device.create_image(&info, None) }?;
        let mut exported = Image {
            image,
            memory: Vec::new(),
            view: vk::ImageView::null(),
            width: SIDE as i32,
            height: SIDE as i32,
        };
        match self.export_image(&mut exported, capability.planes, format) {
            Ok(dmabuf) => Ok((exported, dmabuf)),
            Err(error) => {
                self.destroy_image(&exported);
                Err(error)
            }
        }
    }

    fn export_image(
        &self,
        image: &mut Image,
        planes: usize,
        format: Format,
    ) -> Result<Dmabuf, RenderError> {
        // SAFETY: `image.image` is a live image of this device without memory yet.
        let requirements = unsafe { self.device.get_image_memory_requirements(image.image) };
        let kind = self.memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::empty(),
        )?;
        let mut export = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image.image);
        let memory = self.allocate(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(kind)
                .push_next(&mut export)
                .push_next(&mut dedicated),
        )?;
        image.memory.push(memory);
        // SAFETY: `memory` was just allocated for this image, which has none bound yet.
        unsafe { self.device.bind_image_memory(image.image, memory, 0) }?;
        // SAFETY: `memory` was allocated exportable as a dma-buf.
        let fd = unsafe {
            self.external_memory_fd.get_memory_fd(
                &vk::MemoryGetFdInfoKHR::default()
                    .memory(memory)
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT),
            )
        }?;
        // SAFETY: `get_memory_fd` returns a new fd that nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut builder = Dmabuf::builder(
            (image.width, image.height),
            format.code,
            format.modifier,
            DmabufFlags::empty(),
        );
        for (index, aspect) in MEMORY_PLANES.into_iter().take(planes).enumerate() {
            // SAFETY: the image has a DRM format modifier with `planes` memory planes, of which
            // `aspect` is one.
            let layout = unsafe {
                self.device.get_image_subresource_layout(
                    image.image,
                    vk::ImageSubresource::default().aspect_mask(aspect),
                )
            };
            let offset =
                u32::try_from(layout.offset).map_err(|_| RenderError::BufferOutOfBounds)?;
            let stride =
                u32::try_from(layout.row_pitch).map_err(|_| RenderError::BufferOutOfBounds)?;
            builder.add_plane(fd.try_clone()?, index as u32, offset, stride);
        }
        builder.build().ok_or(RenderError::UnsupportedFormat)
    }

    fn bind_imported_image(
        &self,
        image: &mut Image,
        objects: Vec<File>,
        disjoint: bool,
    ) -> Result<(), RenderError> {
        if objects.len() > MEMORY_PLANES.len() {
            return Err(RenderError::UnsupportedFormat);
        }
        for (mut file, aspect) in objects.into_iter().zip(MEMORY_PLANES) {
            let size = file.seek(SeekFrom::End(0))?;
            let mut plane = vk::ImagePlaneMemoryRequirementsInfo::default().plane_aspect(aspect);
            let mut info = vk::ImageMemoryRequirementsInfo2::default().image(image.image);
            if disjoint {
                info = info.push_next(&mut plane);
            }
            let mut dedicated = vk::MemoryDedicatedRequirements::default();
            let mut requirements = vk::MemoryRequirements2::default().push_next(&mut dedicated);
            // SAFETY: `image.image` is a live image of this device without memory yet, and
            // `plane` names one of its memory planes only when it was created `DISJOINT`.
            unsafe {
                self.device
                    .get_image_memory_requirements2(&info, &mut requirements)
            };
            let requirements = requirements.memory_requirements;
            if disjoint && dedicated.requires_dedicated_allocation == vk::TRUE {
                return Err(RenderError::UnsupportedFormat);
            }
            if requirements.size > size {
                return Err(RenderError::BufferOutOfBounds);
            }
            image.memory.push(self.import_memory_fd(
                file,
                requirements,
                (!disjoint).then_some(image.image),
            )?);
        }
        let mut planes: Vec<_> = MEMORY_PLANES
            .iter()
            .take(image.memory.len())
            .map(|&aspect| vk::BindImagePlaneMemoryInfo::default().plane_aspect(aspect))
            .collect();
        let binds: Vec<_> = image
            .memory
            .iter()
            .zip(&mut planes)
            .map(|(&memory, plane)| {
                let bind = vk::BindImageMemoryInfo::default()
                    .image(image.image)
                    .memory(memory);
                match disjoint {
                    true => bind.push_next(plane),
                    false => bind,
                }
            })
            .collect();
        // SAFETY: each memory was just imported for its plane of `image.image`, which has none bound
        // yet, and covers that plane's requirements; a disjoint image binds every memory plane once.
        unsafe { self.device.bind_image_memory2(&binds) }?;
        Ok(())
    }

    fn import_memory_fd(
        &self,
        file: File,
        requirements: vk::MemoryRequirements,
        dedicated: Option<vk::Image>,
    ) -> Result<vk::DeviceMemory, RenderError> {
        let mut properties = vk::MemoryFdPropertiesKHR::default();
        // SAFETY: `file` stays open across the call, and querying the fd does not take ownership of
        // it.
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
        let mut whole = vk::MemoryDedicatedAllocateInfo::default();
        let mut info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(kind)
            .push_next(&mut import);
        if let Some(image) = dedicated {
            whole = whole.image(image);
            info = info.push_next(&mut whole);
        }
        self.allocate(&info).map_err(|error| {
            // SAFETY: the import failed, so Vulkan did not take ownership of `fd`, which
            // `into_raw_fd` released from `file`; this is its only close.
            unsafe { smithay::reexports::rustix::io::close(fd) };
            error.into()
        })
    }

    fn plane_view(
        &self,
        image: vk::Image,
        format: vk::Format,
        aspect: vk::ImageAspectFlags,
    ) -> Result<vk::ImageView, RenderError> {
        // SAFETY: `image` is a live multi-planar image of this device, and `aspect` names one of
        // its planes.
        Ok(unsafe {
            self.device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: aspect,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    }),
                None,
            )
        }?)
    }

    fn allocate(&self, info: &vk::MemoryAllocateInfo) -> Result<vk::DeviceMemory, vk::Result> {
        // SAFETY: `info` and every struct chained into it are borrowed for the call.
        unsafe { self.device.allocate_memory(info, None) }.inspect_err(|error| {
            if *error == vk::Result::ERROR_OUT_OF_DEVICE_MEMORY {
                self.exhausted.store(true, Ordering::Relaxed);
            }
        })
    }

    fn bind_image_memory(&self, image: &mut Image) -> Result<(), RenderError> {
        // SAFETY: `image.image` is a live image of this device.
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
        let memory = self.allocate(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(kind),
        )?;
        image.memory.push(memory);
        // SAFETY: `memory` was just allocated from the image's own requirements, and the image has
        // no memory bound yet.
        unsafe { self.device.bind_image_memory(image.image, memory, 0) }?;
        Ok(())
    }

    fn create_view(
        &self,
        image: vk::Image,
        format: vk::Format,
        alpha: vk::ComponentSwizzle,
    ) -> Result<vk::ImageView, RenderError> {
        // SAFETY: `image` is a live image of this device in a format the view can use, and the
        // create info outlives the call.
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
        self.discard(submit::Garbage::Image {
            image: image.image,
            view: image.view,
            memory: image.memory.clone(),
        });
    }

    fn destroy_imported(&self, imported: &Imported) {
        self.discard(submit::Garbage::Buffer {
            buffer: imported.buffer,
            memory: imported.memory,
        });
    }

    fn create_staging(&self, capacity: usize) -> Result<Staging, RenderError> {
        // SAFETY: the create info outlives the call; the buffer is destroyed by `destroy_staging`.
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
            last_used: Instant::now(),
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
        // SAFETY: `staging.buffer` is a live buffer of this device.
        let requirements = unsafe { self.device.get_buffer_memory_requirements(staging.buffer) };
        let kind = self.memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        staging.memory = self.allocate(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(kind),
        )?;
        // SAFETY: `staging.memory` was just allocated from the buffer's own requirements in a
        // host-visible, coherent type, and is neither bound nor mapped yet; `WHOLE_SIZE` maps all
        // of it, at least `capacity` bytes.
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
        self.discard(submit::Garbage::Buffer {
            buffer: staging.buffer,
            memory: staging.memory,
        });
    }

    fn reserve(&mut self, bytes: usize) -> Result<(usize, usize), RenderError> {
        let fits = self
            .staging
            .last()
            .is_some_and(|staging| staging.capacity - staging.used >= bytes);
        if !fits {
            let staging = match self.take_spare_staging(bytes) {
                Some(staging) => staging,
                None => self.create_staging(bytes.max(MIN_STAGING_BYTES))?,
            };
            self.staging.push(staging);
        }
        let index = self.staging.len() - 1;
        let staging = &mut self.staging[index];
        let offset = staging.used;
        staging.used += bytes;
        staging.last_used = Instant::now();
        Ok((index, offset))
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
        // SAFETY: `external` is chained into a create info that outlives the call; the buffer is
        // destroyed by `destroy_imported`, or below if binding fails.
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
                // SAFETY: binding failed, so the buffer was never used.
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
        // SAFETY: `buffer` is a live buffer of this device without memory yet.
        let requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        if requirements.size > size {
            return Err(RenderError::BufferOutOfBounds);
        }
        let memory = self.import_memory_fd(
            file,
            vk::MemoryRequirements {
                size,
                ..requirements
            },
            None,
        )?;
        // SAFETY: `memory` was just imported for `buffer`, which has no memory yet, and covers its
        // requirements as checked above.
        if let Err(error) = unsafe { self.device.bind_buffer_memory(buffer, memory, 0) } {
            // SAFETY: binding failed, so nothing uses `memory`; freeing it also closes the imported
            // fd it owns.
            unsafe { self.device.free_memory(memory, None) };
            return Err(error.into());
        }
        Ok(memory)
    }

    fn direct_target(
        &mut self,
        dmabuf: &Dmabuf,
        capability: formats::Capability,
    ) -> Result<u64, RenderError> {
        let texel = formats::texel(dmabuf.format().code).ok_or(RenderError::UnsupportedFormat)?;
        self.prepare_pipelines(texel.format)?;
        let mut image = self.import_image(dmabuf, texel, (formats::RENDERED, capability))?;
        image.view =
            match self.create_view(image.image, texel.format, vk::ComponentSwizzle::IDENTITY) {
                Ok(view) => view,
                Err(error) => {
                    self.destroy_image(&image);
                    return Err(error);
                }
            };
        let id = self.next_id();
        self.targets.insert(
            id,
            Target {
                image,
                format: texel.format,
                rendered: false,
                export: None,
                direct: true,
                unconverted: None,
            },
        );
        Ok(id)
    }

    fn dmabuf_target(&mut self, dmabuf: &Dmabuf) -> Result<u64, RenderError> {
        let format = dmabuf.format();
        if let Some(capability) = self.modifiers.rendered.usable(&format) {
            return self.direct_target(dmabuf, capability);
        }
        let export = self.dmabuf_export(dmabuf)?;
        let size = dmabuf.size();
        self.create_target(size.w, size.h, format.code, Some(export))
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
                    vk::ImageUsageFlags::COLOR_ATTACHMENT
                        | vk::ImageUsageFlags::TRANSFER_SRC
                        | vk::ImageUsageFlags::SAMPLED,
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
                direct: false,
                unconverted: None,
            },
        );
        Ok(id)
    }

    fn unconverted_image(
        &mut self,
        id: u64,
        colour: Option<&OutputColour>,
    ) -> Result<Option<UnconvertedFrame>, RenderError> {
        let target = self
            .targets
            .get_mut(&id)
            .ok_or(RenderError::ForeignResource)?;
        let format = match colour {
            Some(colour) if colour.extended => EXTENDED_FORMAT,
            _ => target.format,
        };
        let stale = target
            .unconverted
            .as_ref()
            .is_some_and(|unconverted| unconverted.format != format);
        if (!target.direct || colour.is_none() || stale)
            && let Some(unconverted) = target.unconverted.take()
        {
            self.destroy_image(&unconverted.image);
        }
        let target = self
            .targets
            .get_mut(&id)
            .ok_or(RenderError::ForeignResource)?;
        if !target.direct || colour.is_none() {
            return Ok(None);
        }
        if let Some(unconverted) = &target.unconverted {
            let image = &unconverted.image;
            return Ok(Some(UnconvertedFrame {
                image: image.image,
                view: image.view,
                format,
                drawn: unconverted.drawn,
            }));
        }
        let (width, height) = (target.image.width, target.image.height);
        self.prepare_pipelines(format)?;
        let image = self.create_image(
            width,
            height,
            format,
            vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_SRC,
            vk::ComponentSwizzle::IDENTITY,
        )?;
        let unconverted = UnconvertedFrame {
            image: image.image,
            view: image.view,
            format,
            drawn: false,
        };
        if let Some(target) = self.targets.get_mut(&id) {
            target.unconverted = Some(Unconverted {
                image,
                format,
                drawn: false,
            });
        }
        Ok(Some(unconverted))
    }

    fn mark_rendered(&mut self, id: u64) {
        if let Some(target) = self.targets.get_mut(&id) {
            target.rendered = true;
            if let Some(unconverted) = &mut target.unconverted {
                unconverted.drawn = true;
            }
        }
    }

    fn forget_texture(&mut self, id: u64) {
        if let Some(removed) = self.textures.remove(&id) {
            self.destroy_image(&removed.image);
        }
        self.copies.retain(|copy| copy.texture != id);
    }

    fn forget_target(&mut self, id: u64) {
        if let Some(removed) = self.targets.remove(&id) {
            self.destroy_target(&removed);
            let size = (removed.image.width, removed.image.height);
            let (kept, retired): (HashMap<_, _>, HashMap<_, _>) =
                std::mem::take(&mut self.colour_scratch)
                    .into_iter()
                    .partition(|((width, height, _), _)| {
                        (*width, *height) != size
                            || self
                                .targets
                                .values()
                                .any(|target| (target.image.width, target.image.height) == size)
                    });
            for image in retired.values() {
                self.destroy_image(image);
            }
            self.colour_scratch = kept;
        }
    }

    fn destroy_target(&self, target: &Target) {
        self.destroy_image(&target.image);
        if let Some(unconverted) = &target.unconverted {
            self.destroy_image(&unconverted.image);
        }
        if let Some(device) = target.export.as_ref().and_then(|export| export.device) {
            self.destroy_imported(&device.imported);
        }
    }

    fn collect_garbage(&mut self) {
        self.dmabuf_textures.retain(|weak, _| !weak.is_gone());
        self.sampled_dmabufs.retain(|weak, _| !weak.is_gone());
        self.failed_dmabufs.retain(|weak| !weak.is_gone());
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

    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Relaxed)
    }

    pub fn out_of_memory(&self) -> bool {
        self.exhausted.load(Ordering::Relaxed)
    }

    pub fn renewal_wait(&self) -> Option<Duration> {
        match (self.is_lost(), self.out_of_memory()) {
            (true, _) => Some(Duration::ZERO),
            (false, true) => Some(
                self.renewable_from
                    .saturating_duration_since(Instant::now()),
            ),
            (false, false) => None,
        }
    }

    pub fn renewed_if_failed(&mut self) -> Result<Option<Self>, RenderError> {
        if self.renewal_wait() != Some(Duration::ZERO) {
            return Ok(None);
        }
        let renewed = self.renew();
        self.kept_if_usable(renewed)
    }

    fn kept_if_usable(
        &mut self,
        renewed: Result<Self, RenderError>,
    ) -> Result<Option<Self>, RenderError> {
        match renewed {
            Err(error) if !self.is_lost() => {
                tracing::warn!("keeping the renderer, as another cannot start: {error}");
                self.renewable_from = Instant::now() + MIN_RENDERER_LIFETIME;
                Ok(None)
            }
            renewed => renewed.map(Some),
        }
    }

    pub fn memory_use(&self) -> MemoryUse {
        let image_bytes = |image: &Image| {
            u64::from(image.width.unsigned_abs())
                * u64::from(image.height.unsigned_abs())
                * BYTES_PER_PIXEL as u64
        };
        let levels = self
            .blur_chains
            .values()
            .flat_map(|chain| chain.levels.iter());
        let staging = self.staging.iter().chain(self.reserved_staging());
        MemoryUse {
            textures: (
                self.textures.len(),
                self.textures
                    .values()
                    .filter(|texture| !texture.imported)
                    .map(|texture| image_bytes(&texture.image))
                    .sum(),
            ),
            targets: (
                self.targets.len(),
                self.targets
                    .values()
                    .map(|target| image_bytes(&target.image))
                    .sum(),
            ),
            imports: (
                self.imports.len(),
                self.imports.values().map(|imported| imported.size).sum(),
            ),
            staging: (
                staging.clone().count(),
                staging.map(|staging| staging.capacity as u64).sum(),
            ),
            blur_levels: (levels.clone().count(), levels.map(image_bytes).sum()),
            heap_size: self.memory.memory_heaps[..self.memory.memory_heap_count as usize]
                .iter()
                .map(|heap| heap.size)
                .sum(),
        }
    }

    pub(crate) fn descriptor_set(
        &mut self,
        view: vk::ImageView,
        second: vk::ImageView,
    ) -> Result<vk::DescriptorSet, RenderError> {
        if let Some(&(set, _)) = self.descriptors.get_mut().sets.get(&(view, second)) {
            return Ok(set);
        }
        let set = match self.allocate_set() {
            Ok(allocated) => allocated,
            Err(vk::Result::ERROR_OUT_OF_POOL_MEMORY | vk::Result::ERROR_FRAGMENTED_POOL) => {
                self.add_descriptor_pool()?;
                self.allocate_set()?
            }
            Err(error) => return Err(error.into()),
        };
        let infos = [view, second].map(|view| {
            vk::DescriptorImageInfo::default()
                .image_view(view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        });
        let writes = [(0, &infos[0]), (2, &infos[1])].map(|(binding, info)| {
            vk::WriteDescriptorSet::default()
                .dst_set(set.0)
                .dst_binding(binding)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(slice::from_ref(info))
        });
        // SAFETY: `set` was just allocated and no command buffer uses it yet; both views are live
        // views of this device, and the set is freed with whichever of them is destroyed first.
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        self.descriptors.get_mut().sets.insert((view, second), set);
        Ok(set.0)
    }

    fn allocate_set(&mut self) -> Result<(vk::DescriptorSet, vk::DescriptorPool), vk::Result> {
        let Some((pool, used)) = self
            .descriptors
            .get_mut()
            .pools
            .iter_mut()
            .find(|(_, used)| *used < SETS_PER_POOL)
        else {
            return Err(vk::Result::ERROR_OUT_OF_POOL_MEMORY);
        };
        let pool = *pool;
        *used += 1;
        // SAFETY: `pool` and `set_layout` belong to this device, and the pool is used only from
        // the renderer's thread.
        let sets = unsafe {
            self.device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(pool)
                    .set_layouts(slice::from_ref(&self.set_layout)),
            )
        }
        .inspect_err(|_| self.release_set_of(pool))?;
        sets.into_iter()
            .next()
            .map(|set| (set, pool))
            .ok_or(vk::Result::ERROR_OUT_OF_POOL_MEMORY)
    }

    fn add_descriptor_pool(&mut self) -> Result<(), RenderError> {
        let sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(2 * SETS_PER_POOL),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(SETS_PER_POOL),
        ];
        // SAFETY: the create info outlives the call; the pool is destroyed in `Drop`, after the
        // device is idle.
        let pool = unsafe {
            self.device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
                    .max_sets(SETS_PER_POOL)
                    .pool_sizes(&sizes),
                None,
            )
        }?;
        self.descriptors.get_mut().pools.push((pool, 0));
        Ok(())
    }

    fn release_set_of(&self, pool: vk::DescriptorPool) {
        let mut descriptors = self.descriptors.borrow_mut();
        let Some(index) = descriptors
            .pools
            .iter()
            .position(|(owner, _)| *owner == pool)
        else {
            return;
        };
        let used = &mut descriptors.pools[index].1;
        *used = used.saturating_sub(1);
        if *used == 0 && descriptors.pools.len() > 1 {
            descriptors.pools.swap_remove(index);
            // SAFETY: every set of the pool was freed after the submissions that used it
            // finished, so nothing refers to the pool any more.
            unsafe { self.device.destroy_descriptor_pool(pool, None) };
        }
    }

    fn forget_sets_of(&self, view: vk::ImageView) {
        let mut descriptors = self.descriptors.borrow_mut();
        let gone: Vec<_> = descriptors
            .sets
            .keys()
            .filter(|(first, second)| *first == view || *second == view)
            .copied()
            .collect();
        let mut freed = Vec::new();
        for key in gone {
            if let Some((set, pool)) = descriptors.sets.remove(&key) {
                // SAFETY: the set's image is being destroyed after every submission that read it
                // finished, so no pending command buffer uses the set; it came from `pool`, which
                // was created with `FREE_DESCRIPTOR_SET`.
                if let Err(error) = unsafe {
                    self.device
                        .free_descriptor_sets(pool, slice::from_ref(&set))
                } {
                    tracing::warn!("cannot free a descriptor set: {error}");
                }
                freed.push(pool);
            }
        }
        drop(descriptors);
        for pool in freed {
            self.release_set_of(pool);
        }
    }

    pub fn finish_uploads(&mut self) -> Result<(), RenderError> {
        if self.copies.is_empty() {
            return Ok(());
        }
        let from_clients = self.copies.iter().any(|copy| {
            self.imports
                .values()
                .any(|imported| imported.buffer == copy.source)
        });
        self.begin_commands()?;
        self.record_copies();
        let serial = self.submit(None)?;
        self.hand_staging_to(serial);
        match from_clients {
            true => self.wait_for(serial),
            false => Ok(()),
        }
    }

    fn record_copies(&mut self) {
        let mut by_texture: HashMap<u64, Vec<&PendingCopy>> = HashMap::new();
        for copy in &self.copies {
            by_texture.entry(copy.texture).or_default().push(copy);
        }
        let sources: Vec<vk::Buffer> = self
            .imports
            .values()
            .map(|imported| imported.buffer)
            .filter(|buffer| self.copies.iter().any(|copy| copy.source == *buffer))
            .collect();
        let families = (self.queue_family, self.foreign_family);
        let reading = (
            vk::PipelineStageFlags2::COPY,
            vk::AccessFlags2::TRANSFER_READ,
        );
        hand_over_buffers(
            &self.device,
            self.command_buffer,
            &sources,
            families,
            (true, reading),
        );
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
                // SAFETY: recording since `begin_commands`; `copy.source` is a live staging or
                // imported buffer (forgetting either drops its pending copies) and the barrier
                // above moved `image` to `TRANSFER_DST_OPTIMAL`.
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
        hand_over_buffers(
            &self.device,
            self.command_buffer,
            &sources,
            families,
            (false, reading),
        );
        self.copies.clear();
    }

    fn copy_target_to(
        &mut self,
        readback: &Staging,
        image: vk::Image,
        width: i32,
        height: i32,
    ) -> Result<(), RenderError> {
        let region = vk::BufferImageCopy::default()
            .image_subresource(COLOR_LAYERS)
            .image_extent(vk::Extent3D {
                width: width.unsigned_abs(),
                height: height.unsigned_abs(),
                depth: 1,
            });
        self.begin_commands()?;
        let device = &self.device;
        let commands = self.command_buffer;
        barrier(device, commands, image, ATTACHMENT, TRANSFER_SOURCE);
        // SAFETY: recording since `begin_commands`; the barrier above moved the live target `image`
        // to `TRANSFER_SRC_OPTIMAL`, and `readback` holds `width * height` pixels.
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
        // SAFETY: recording since `begin_commands` on this thread, and nothing else records into
        // the buffer.
        unsafe {
            device.cmd_pipeline_barrier2(
                commands,
                &vk::DependencyInfo::default().memory_barriers(slice::from_ref(&host_read)),
            );
        }
        self.submit_and_wait()
    }

    fn device_export(&self, dmabuf: &Dmabuf) -> Result<DeviceExport, RenderError> {
        let layout = import::single_plane_layout(dmabuf)?;
        let imported = self.import_dmabuf_memory(layout.fd, vk::BufferUsageFlags::TRANSFER_DST)?;
        if !layout.fits(dmabuf.size(), imported.size) {
            self.destroy_imported(&imported);
            return Err(RenderError::BufferOutOfBounds);
        }
        Ok(DeviceExport {
            imported,
            offset: u64::from(layout.offset),
            row_texels: layout.stride / BYTES_PER_PIXEL as u32,
        })
    }

    fn dmabuf_export(&self, dmabuf: &Dmabuf) -> Result<Export, RenderError> {
        target_format(dmabuf.format().code).ok_or(RenderError::UnsupportedFormat)?;
        let mapped = self.mapped_export(dmabuf)?;
        let device = self
            .output_on_device
            .then(|| self.device_export(dmabuf))
            .and_then(|exported| {
                exported
                    .inspect_err(|error| tracing::warn!("copying frames on the CPU: {error}"))
                    .ok()
            });
        Ok(Export {
            mapped,
            device,
            trusted: false,
        })
    }

    fn mapped_export(&self, dmabuf: &Dmabuf) -> Result<MappedExport, RenderError> {
        let layout = import::single_plane_layout(dmabuf)?;
        let fd = layout.fd.try_clone_to_owned()?;
        let length = usize::try_from(File::from(fd.try_clone()?).seek(SeekFrom::End(0))?)
            .map_err(|_| RenderError::BufferOutOfBounds)?;
        if !layout.fits(dmabuf.size(), length as u64) {
            return Err(RenderError::BufferOutOfBounds);
        }
        // SAFETY: a new shared mapping of the dma-buf's whole `length`; `MappedExport` owns it and
        // unmaps it on drop, and every access through it is bounds-checked against `length` by
        // `rows_of`.
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
        Ok(MappedExport {
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
            colour: None,
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
                let id = self.dmabuf_target(dmabuf)?;
                self.dmabuf_targets.insert(dmabuf.weak(), id);
                id
            }
        };
        Ok(VulkanFramebuffer {
            id,
            width: size.w,
            height: size.h,
            format,
            colour: self.output_colour.clone(),
            _target: PhantomData,
        })
    }

    fn supported_formats(&self) -> Option<FormatSet> {
        Some(self.render_formats())
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        if self.is_lost() {
            return;
        }
        // SAFETY: `Drop` has the only access to the device, which is still alive.
        if let Err(error) = unsafe { self.device.device_wait_idle() } {
            tracing::warn!("the GPU did not finish before the renderer stopped: {error}");
        }
        self.retire();
        self.dmabuf_textures.clear();
        self.sampled_dmabufs.clear();
        for (_, texture) in std::mem::take(&mut self.textures) {
            self.destroy_image(&texture.image);
        }
        for (_, target) in std::mem::take(&mut self.targets) {
            self.destroy_target(&target);
        }
        for (_, image) in std::mem::take(&mut self.colour_scratch) {
            self.destroy_image(&image);
        }
        for staging in std::mem::take(&mut self.staging) {
            self.destroy_staging(&staging);
        }
        self.forget_blurs();
        for (_, imported) in std::mem::take(&mut self.imports) {
            self.destroy_imported(&imported);
        }
        self.release_submissions();
        // SAFETY: the device is idle and every image, buffer and allocation was destroyed above;
        // these objects were created once by `create_objects`, and children go before the device
        // and the device before the instance.
        unsafe {
            for (_, pipeline) in self.pipelines.drain() {
                self.device.destroy_pipeline(pipeline, None);
            }
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            for (pool, _) in std::mem::take(&mut self.descriptors.get_mut().pools) {
                self.device.destroy_descriptor_pool(pool, None);
            }
            self.device
                .destroy_descriptor_set_layout(self.set_layout, None);
            self.device.destroy_sampler(self.sampler, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
        }
        forget_instance(&self.instance, self.messenger.as_ref());
    }
}

static VULKAN_ERRORS: AtomicU64 = AtomicU64::new(0);

pub fn vulkan_errors() -> u64 {
    VULKAN_ERRORS.load(Ordering::Relaxed)
}

unsafe extern "system" fn report(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _: *mut std::ffi::c_void,
) -> vk::Bool32 {
    // SAFETY: the loader passes callback data, and the message it points to, valid for this call.
    let message = unsafe { data.as_ref().and_then(|data| data.message_as_c_str()) }
        .map(CStr::to_string_lossy)
        .unwrap_or_default();
    let error = severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR);
    if error {
        VULKAN_ERRORS.fetch_add(1, Ordering::Relaxed);
    }
    match error {
        true => tracing::error!("Vulkan: {message}"),
        false => tracing::warn!("Vulkan: {message}"),
    }
    vk::FALSE
}

unsafe fn promoted_commands(instance: &ash::Instance, device: vk::Device) -> ash::Device {
    let find = |name: &CStr| {
        // SAFETY: `device` is a live device of `instance` and `name` is a NUL-terminated name.
        unsafe { instance.get_device_proc_addr(device, name.as_ptr()) }
    };
    let load = |name: &CStr| {
        let command = find(name).or_else(|| {
            let mut promoted = name.to_bytes().to_vec();
            promoted.extend_from_slice(b"KHR");
            std::ffi::CString::new(promoted)
                .ok()
                .and_then(|promoted| find(&promoted))
        });
        command.map_or(std::ptr::null(), |command| {
            command as *const std::ffi::c_void
        })
    };
    // SAFETY: `load` returns each command of `device` by its name, or the `KHR` command it was
    // promoted from, or null, which ash replaces by a stub that panics if called.
    unsafe { ash::Device::load_with(load, device) }
}

fn forget_instance(
    instance: &ash::Instance,
    messenger: Option<&(ash::ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
) {
    // SAFETY: callers pass an instance whose devices are destroyed and that is never used again,
    // with the messenger created from it.
    unsafe {
        if let Some((utils, messenger)) = messenger {
            utils.destroy_debug_utils_messenger(*messenger, None);
        }
        instance.destroy_instance(None);
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

const DRAWING: Usage = (
    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
    vk::AccessFlags2::from_raw(
        vk::AccessFlags2::COLOR_ATTACHMENT_READ.as_raw()
            | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE.as_raw(),
    ),
);

const SAMPLED: Usage = (
    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
    vk::PipelineStageFlags2::FRAGMENT_SHADER,
    vk::AccessFlags2::SHADER_SAMPLED_READ,
);

const TRANSFER_SOURCE: Usage = (
    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
    vk::PipelineStageFlags2::COPY,
    vk::AccessFlags2::TRANSFER_READ,
);

fn hand_over(
    device: &ash::Device,
    commands: vk::CommandBuffer,
    image: vk::Image,
    family: u32,
    (acquire, usage): (bool, Usage),
) {
    let (layout, stage, access) = usage;
    let foreign = vk::ImageMemoryBarrier2::default()
        .image(image)
        .subresource_range(COLOR_RANGE);
    let foreign = match acquire {
        true => foreign
            .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
            .dst_queue_family_index(family)
            .old_layout(vk::ImageLayout::GENERAL)
            .new_layout(layout)
            .src_stage_mask(stage)
            .dst_stage_mask(stage)
            .dst_access_mask(access),
        false => foreign
            .src_queue_family_index(family)
            .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
            .old_layout(layout)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_stage_mask(stage)
            .src_access_mask(access & !vk::AccessFlags2::SHADER_SAMPLED_READ),
    };
    // SAFETY: callers record between `begin_commands` and the end of the buffer, on the renderer's
    // thread, with `image` a live image imported with `QUEUE_FAMILY_FOREIGN_EXT` enabled: it is
    // acquired from the foreign queue in `GENERAL` before the buffer uses it and released back
    // the same way before the buffer ends, so it is never ours between buffers.
    unsafe {
        device.cmd_pipeline_barrier2(
            commands,
            &vk::DependencyInfo::default().image_memory_barriers(slice::from_ref(&foreign)),
        );
    }
}

fn hand_over_buffers(
    device: &ash::Device,
    commands: vk::CommandBuffer,
    buffers: &[vk::Buffer],
    (family, foreign): (u32, u32),
    (acquire, (stage, access)): (bool, (vk::PipelineStageFlags2, vk::AccessFlags2)),
) {
    if buffers.is_empty() {
        return;
    }
    let barriers: Vec<_> = buffers
        .iter()
        .map(|&buffer| {
            let barrier = vk::BufferMemoryBarrier2::default()
                .buffer(buffer)
                .size(vk::WHOLE_SIZE);
            match acquire {
                true => barrier
                    .src_queue_family_index(foreign)
                    .dst_queue_family_index(family)
                    .src_stage_mask(stage)
                    .dst_stage_mask(stage)
                    .dst_access_mask(access),
                false => barrier
                    .src_queue_family_index(family)
                    .dst_queue_family_index(foreign)
                    .src_stage_mask(stage)
                    .src_access_mask(access & vk::AccessFlags2::TRANSFER_WRITE),
            }
        })
        .collect();
    // SAFETY: callers record between `begin_commands` and the end of the buffer, on the renderer's
    // thread, with live dma-buf imports: each is acquired from `foreign` before the buffer uses it
    // and released back before the buffer ends.
    unsafe {
        device.cmd_pipeline_barrier2(
            commands,
            &vk::DependencyInfo::default().buffer_memory_barriers(&barriers),
        );
    }
}

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
    // SAFETY: callers record between `begin_commands` and the end of the buffer, on the renderer's
    // thread, with `image` a live image currently in the `from` layout.
    unsafe {
        device.cmd_pipeline_barrier2(
            commands,
            &vk::DependencyInfo::default().image_memory_barriers(slice::from_ref(&barrier)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_renderer_out_of_memory_outlives_a_failed_renewal_and_tries_again()
    -> Result<(), RenderError> {
        let Ok(mut renderer) = VulkanRenderer::new(None) else {
            println!("skipped: no Vulkan device");
            return Ok(());
        };
        assert_eq!(renderer.renewal_wait(), None);
        renderer.exhausted.store(true, Ordering::Relaxed);
        renderer.renewable_from = Instant::now();
        assert_eq!(renderer.renewal_wait(), Some(Duration::ZERO));
        assert!(
            renderer
                .kept_if_usable(Err(RenderError::NoDevice))?
                .is_none()
        );
        assert!(
            renderer
                .renewal_wait()
                .is_some_and(|wait| wait > Duration::ZERO)
        );
        assert!(renderer.renewed_if_failed()?.is_none());
        renderer.renewable_from = Instant::now();
        assert!(renderer.renewed_if_failed()?.is_some());
        renderer.lost.store(true, Ordering::Relaxed);
        assert!(renderer.kept_if_usable(Err(RenderError::NoDevice)).is_err());
        Ok(())
    }
}
