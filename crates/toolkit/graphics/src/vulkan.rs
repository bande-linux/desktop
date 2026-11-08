use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ash::{khr, lunarg, vk};
use libloading::Library;
use wgpu::hal;

const RENDER_NODES: &str = "/sys/class/drm";
const ICD_DIRECTORIES: [&str; 3] = [
    "/etc/vulkan/icd.d",
    "/usr/local/share/vulkan/icd.d",
    "/usr/share/vulkan/icd.d",
];
const LOADER_OVERRIDES: [&str; 5] = [
    "VK_DRIVER_FILES",
    "VK_ICD_FILENAMES",
    "VK_ADD_DRIVER_FILES",
    "VK_LOADER_DRIVERS_SELECT",
    "VK_LOADER_DRIVERS_DISABLE",
];
const ENTRY_POINT: &[u8] = b"vk_icdGetInstanceProcAddr\0";
const HARDWARE_DRIVERS: [(&str, &[&str]); 9] = [
    ("asahi", &["asahi"]),
    ("radeon", &["amdgpu", "radeon"]),
    ("intel", &["i915", "xe"]),
    ("intel_hasvk", &["i915"]),
    ("nouveau", &["nouveau"]),
    ("freedreno", &["msm"]),
    ("panfrost", &["panfrost", "panthor"]),
    ("broadcom", &["v3d"]),
    ("powervr_mesa", &["pvr"]),
];
const EXTENSIONS: [&CStr; 2] = [khr::surface::NAME, khr::wayland_surface::NAME];

type GetInstanceProcAddr =
    unsafe extern "system" fn(vk::Instance, *const c_char) -> vk::PFN_vkVoidFunction;

struct Driver {
    _library: Library,
    get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddrLUNARG,
}

static DRIVERS: OnceLock<Option<Vec<Driver>>> = OnceLock::new();

fn render_node_drivers() -> Vec<String> {
    std::fs::read_dir(RENDER_NODES)
        .map(|nodes| {
            nodes
                .flatten()
                .filter(|node| node.file_name().to_string_lossy().starts_with("renderD"))
                .filter_map(|node| std::fs::read_link(node.path().join("device/driver")).ok())
                .filter_map(|driver| Some(driver.file_name()?.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

fn needed_drivers() -> Option<Vec<&'static str>> {
    if LOADER_OVERRIDES
        .iter()
        .any(|variable| std::env::var_os(variable).is_some())
    {
        return None;
    }
    let present = render_node_drivers();
    let known = |kernel: &String| {
        HARDWARE_DRIVERS
            .iter()
            .any(|(_, served)| served.contains(&kernel.as_str()))
    };
    if present.is_empty() || !present.iter().all(known) {
        return None;
    }
    Some(
        HARDWARE_DRIVERS
            .iter()
            .filter(|(_, served)| {
                present
                    .iter()
                    .any(|kernel| served.contains(&kernel.as_str()))
            })
            .map(|(name, _)| *name)
            .collect(),
    )
}

fn manifests(name: &str) -> Vec<PathBuf> {
    let prefix = format!("{name}_icd.");
    ICD_DIRECTORIES
        .iter()
        .filter_map(|directory| std::fs::read_dir(directory).ok())
        .flat_map(|entries| entries.flatten().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .map(|file| file.to_string_lossy())
                .is_some_and(|file| file.starts_with(&prefix) && file.ends_with(".json"))
        })
        .collect()
}

fn library_path(manifest: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let library = Path::new(value.get("ICD")?.get("library_path")?.as_str()?);
    match library.is_relative() && library.components().count() > 1 {
        true => Some(manifest.parent()?.join(library)),
        false => Some(library.to_owned()),
    }
}

fn open(manifest: &Path) -> Option<Driver> {
    let path = library_path(manifest)?;
    let library = unsafe { Library::new(&path) }
        .inspect_err(|error| tracing::debug!(%error, "cannot open {}", path.display()))
        .ok()?;
    let entry = unsafe { library.get::<GetInstanceProcAddr>(ENTRY_POINT) }.ok()?;
    let get_instance_proc_addr = Some(*entry);
    Some(Driver {
        _library: library,
        get_instance_proc_addr,
    })
}

fn load() -> Option<Vec<Driver>> {
    let drivers: Vec<Driver> = needed_drivers()?
        .into_iter()
        .filter_map(|name| manifests(name).iter().find_map(|manifest| open(manifest)))
        .collect();
    (!drivers.is_empty()).then_some(drivers)
}

pub(crate) fn hardware_instance() -> Option<wgpu::Instance> {
    let drivers = DRIVERS.get_or_init(load).as_deref()?;
    let instance = unsafe { create(drivers) }
        .inspect_err(|error| tracing::debug!("loading the Vulkan drivers directly failed: {error}"))
        .ok()?;
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
    (!adapters.is_empty()).then_some(instance)
}

unsafe fn create(drivers: &[Driver]) -> Result<wgpu::Instance, String> {
    let entry = unsafe { ash::Entry::load() }.map_err(|error| error.to_string())?;
    let version = unsafe { entry.try_enumerate_instance_version() }
        .map_err(|error| error.to_string())?
        .unwrap_or(vk::API_VERSION_1_0);
    if version < vk::API_VERSION_1_1 {
        return Err("the Vulkan loader is older than 1.1".to_owned());
    }
    let infos: Vec<vk::DirectDriverLoadingInfoLUNARG> = drivers
        .iter()
        .map(|driver| {
            vk::DirectDriverLoadingInfoLUNARG::default()
                .pfn_get_instance_proc_addr(driver.get_instance_proc_addr)
        })
        .collect();
    let mut list = vk::DirectDriverLoadingListLUNARG::default()
        .mode(vk::DirectDriverLoadingModeLUNARG::EXCLUSIVE)
        .drivers(&infos);
    let names: Vec<*const c_char> = EXTENSIONS
        .iter()
        .chain([&lunarg::direct_driver_loading::NAME])
        .map(|name| name.as_ptr())
        .collect();
    let application = vk::ApplicationInfo::default()
        .engine_name(c"wgpu-hal")
        .engine_version(2)
        .api_version(vk::API_VERSION_1_3);
    let info = vk::InstanceCreateInfo::default()
        .application_info(&application)
        .enabled_extension_names(&names)
        .push_next(&mut list);
    let raw = unsafe { entry.create_instance(&info, None) }.map_err(|error| error.to_string())?;
    let instance = unsafe {
        hal::vulkan::Instance::from_raw(
            entry,
            raw,
            version,
            0,
            None,
            EXTENSIONS.to_vec(),
            wgpu::InstanceFlags::empty(),
            wgpu::MemoryBudgetThresholds::default(),
            false,
            None,
        )
    }
    .map_err(|error| error.to_string())?;
    Ok(unsafe { wgpu::Instance::from_hal::<hal::api::Vulkan>(instance) })
}
