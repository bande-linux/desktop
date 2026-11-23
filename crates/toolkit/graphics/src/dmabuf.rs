use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use ash::{ext, khr, vk};
use wgpu::hal;
use wgpu::hal::api::Vulkan;

use crate::render::DrawError;

pub const SHARED_FOURCC: u32 = u32::from_le_bytes(*b"AR24");
pub(crate) const SHARED_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
const VULKAN_FORMAT: vk::Format = vk::Format::B8G8R8A8_UNORM;
const USAGE: vk::ImageUsageFlags = vk::ImageUsageFlags::COLOR_ATTACHMENT;
const DMA_BUF: vk::ExternalMemoryHandleTypeFlags = vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT;
const SYNC_FD: vk::ExternalSemaphoreHandleTypeFlags = vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD;
const DMA_BUF_IOCTL_IMPORT_SYNC_FILE: libc::c_ulong = 0x4008_6203;
const DMA_BUF_SYNC_WRITE: u32 = 2;
const READERS_PATIENCE: Duration = Duration::from_secs(1);

#[repr(C)]
struct ImportSyncFile {
    flags: u32,
    fd: RawFd,
}

pub struct SharedImage {
    pub(crate) texture: wgpu::Texture,
    image: vk::Image,
    handed_over: AtomicBool,
    pub fd: OwnedFd,
    pub modifier: u64,
    pub offset: u32,
    pub stride: u32,
}

pub(crate) struct Sharing {
    instance: ash::Instance,
    physical: vk::PhysicalDevice,
    device: ash::Device,
    memory_fd: khr::external_memory_fd::Device,
    modifier_properties: ext::image_drm_format_modifier::Device,
    family: u32,
    fence: Mutex<Option<Fence>>,
    renderable: Vec<u64>,
}

struct Fence {
    semaphore: vk::Semaphore,
    export: khr::external_semaphore_fd::Device,
}

fn failed(what: &str) -> impl Fn(vk::Result) -> DrawError + '_ {
    move |error| DrawError::Gpu(format!("{what}: {error}"))
}

impl Sharing {
    pub(crate) fn new(device: &wgpu::Device) -> Option<Self> {
        // SAFETY: the hal device is only read here and outlives the borrowed guard.
        let hal_device = unsafe { device.as_hal::<Vulkan>() }?;
        let enabled = hal_device.enabled_device_extensions();
        let needed = [
            khr::external_memory_fd::NAME,
            ext::external_memory_dma_buf::NAME,
            ext::image_drm_format_modifier::NAME,
            ext::queue_family_foreign::NAME,
        ];
        if !needed.iter().all(|name| enabled.contains(name)) {
            return None;
        }
        let instance = hal_device.shared_instance().raw_instance().clone();
        let physical = hal_device.raw_physical_device();
        let raw = hal_device.raw_device().clone();
        let renderable = renderable_modifiers(&instance, physical);
        if renderable.is_empty() {
            return None;
        }
        let fence = enabled
            .contains(&khr::external_semaphore_fd::NAME)
            .then(|| Fence::new(&instance, physical, &raw))
            .flatten();
        Some(Self {
            memory_fd: khr::external_memory_fd::Device::new(&instance, &raw),
            modifier_properties: ext::image_drm_format_modifier::Device::new(&instance, &raw),
            instance,
            physical,
            family: hal_device.queue_family_index(),
            device: raw,
            fence: Mutex::new(fence),
            renderable,
        })
    }

    pub(crate) fn accepts_any(&self, tranches: &[Vec<u64>]) -> bool {
        !self.preferred(tranches).is_empty()
    }

    fn preferred(&self, tranches: &[Vec<u64>]) -> Vec<u64> {
        tranches
            .iter()
            .map(|offered| {
                self.renderable
                    .iter()
                    .copied()
                    .filter(|modifier| offered.contains(modifier))
                    .collect::<Vec<u64>>()
            })
            .find(|modifiers| !modifiers.is_empty())
            .unwrap_or_default()
    }

    pub(crate) fn image(
        &self,
        device: &wgpu::Device,
        (width, height): (u32, u32),
        tranches: &[Vec<u64>],
    ) -> Result<SharedImage, DrawError> {
        let modifiers = self.preferred(tranches);
        if modifiers.is_empty() {
            return Err(DrawError::Gpu(
                "the compositor takes none of the GPU's image layouts".to_owned(),
            ));
        }
        let mut list =
            vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(&modifiers);
        let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(DMA_BUF);
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(VULKAN_FORMAT)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(USAGE)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut list)
            .push_next(&mut external);
        // SAFETY: info and the structures it chains live across the call.
        let image = unsafe { self.device.create_image(&info, None) }
            .map_err(failed("cannot create a shared image"))?;
        // SAFETY: image was just created from this device without memory.
        let requirements = unsafe { self.device.get_image_memory_requirements(image) };
        let memory = self.allocate(image, requirements).inspect_err(|_| {
            // SAFETY: image was created above and nothing else uses it.
            unsafe { self.device.destroy_image(image, None) };
        })?;
        self.export(device, image, memory, (width, height))
            .inspect_err(|_| {
                // SAFETY: image and memory were created above and nothing else uses them.
                unsafe {
                    self.device.destroy_image(image, None);
                    self.device.free_memory(memory, None);
                }
            })
    }

    fn allocate(
        &self,
        image: vk::Image,
        requirements: vk::MemoryRequirements,
    ) -> Result<vk::DeviceMemory, DrawError> {
        // SAFETY: physical is the device's own physical device.
        let types = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical)
        };
        let allowed = (0..types.memory_type_count)
            .filter(|index| requirements.memory_type_bits & (1 << index) != 0);
        let index = allowed
            .clone()
            .find(|index| {
                types.memory_types[*index as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .or_else(|| allowed.clone().next())
            .ok_or_else(|| DrawError::Gpu("no memory type for a shared image".to_owned()))?;
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let mut export = vk::ExportMemoryAllocateInfo::default().handle_types(DMA_BUF);
        let info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(index)
            .push_next(&mut export)
            .push_next(&mut dedicated);
        // SAFETY: info and the structures it chains live across the call.
        unsafe { self.device.allocate_memory(&info, None) }
            .map_err(failed("cannot allocate a shared image"))
    }

    fn export(
        &self,
        device: &wgpu::Device,
        image: vk::Image,
        memory: vk::DeviceMemory,
        (width, height): (u32, u32),
    ) -> Result<SharedImage, DrawError> {
        // SAFETY: memory was allocated for image with a matching dedicated allocation.
        unsafe { self.device.bind_image_memory(image, memory, 0) }
            .map_err(failed("cannot bind a shared image"))?;
        let info = vk::MemoryGetFdInfoKHR::default()
            .memory(memory)
            .handle_type(DMA_BUF);
        // SAFETY: memory was allocated exportable as a dma-buf.
        let fd = unsafe { self.memory_fd.get_memory_fd(&info) }
            .map_err(failed("cannot export a shared image"))?;
        // SAFETY: vkGetMemoryFdKHR hands over a new file descriptor that nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut properties = vk::ImageDrmFormatModifierPropertiesEXT::default();
        // SAFETY: image was created with DRM format modifier tiling and is bound to memory.
        unsafe {
            self.modifier_properties
                .get_image_drm_format_modifier_properties(image, &mut properties)
        }
        .map_err(failed("cannot read a shared image's layout"))?;
        let plane = vk::ImageSubresource {
            aspect_mask: vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
            mip_level: 0,
            array_layer: 0,
        };
        // SAFETY: image has DRM format modifier tiling with one memory plane.
        let layout = unsafe { self.device.get_image_subresource_layout(image, plane) };
        let too_large = || DrawError::Gpu("the shared image's layout is too large".to_owned());
        let offset = u32::try_from(layout.offset).map_err(|_| too_large())?;
        let stride = u32::try_from(layout.row_pitch).map_err(|_| too_large())?;
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let descriptor = hal::TextureDescriptor {
            label: Some("toolkit shared frame"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHARED_FORMAT,
            usage: wgpu::TextureUses::COLOR_TARGET,
            memory_flags: hal::MemoryFlags::empty(),
            view_formats: Vec::new(),
        };
        // SAFETY: the guard is dropped before the texture is handed to wgpu.
        let hal_device = unsafe { device.as_hal::<Vulkan>() }
            .ok_or_else(|| DrawError::Gpu("the device is not a Vulkan device".to_owned()))?;
        // SAFETY: image was created from this device with descriptor's size, format and usage;
        // the hal texture takes ownership of image and memory and destroys both when dropped.
        let hal_texture = unsafe {
            hal_device.texture_from_raw(
                image,
                &descriptor,
                None,
                hal::vulkan::TextureMemory::Dedicated(memory),
            )
        };
        drop(hal_device);
        // SAFETY: hal_texture was created from this device for this descriptor and has not been
        // used, so wgpu may treat its contents as undefined.
        let texture = unsafe {
            device.create_texture_from_hal::<Vulkan>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("toolkit shared frame"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: SHARED_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
                wgpu::TextureUses::UNINITIALIZED,
            )
        };
        Ok(SharedImage {
            texture,
            image,
            handed_over: AtomicBool::new(false),
            fd,
            modifier: properties.drm_format_modifier,
            offset,
            stride,
        })
    }

    pub(crate) fn submit(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        draw: wgpu::CommandBuffer,
        image: &SharedImage,
    ) -> Result<(), DrawError> {
        let mut commands = Vec::with_capacity(3);
        if image.handed_over.load(Ordering::Acquire) {
            wait_for_readers(&image.fd);
            commands.push(self.hand_over(device, image.image, false)?);
        }
        commands.push(draw);
        commands.push(self.hand_over(device, image.image, true)?);
        image.handed_over.store(true, Ordering::Release);
        let fence = self.fence.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(active) = fence.as_ref() else {
            return wait(device, queue.submit(commands));
        };
        // SAFETY: the hal queue only queues the semaphore signal and is released before the
        // submission.
        match unsafe { queue.as_hal::<Vulkan>() } {
            Some(hal_queue) => hal_queue.add_signal_semaphore(active.semaphore, None),
            None => return wait(device, queue.submit(commands)),
        }
        let submission = queue.submit(commands);
        // SAFETY: as above.
        let unsignalled = unsafe { queue.as_hal::<Vulkan>() }
            .is_none_or(|hal_queue| hal_queue.remove_signal_semaphore(active.semaphore));
        if unsignalled {
            return wait(device, submission);
        }
        let attached = match active.export() {
            Ok(None) => return Ok(()),
            Ok(Some(finished)) => {
                attach_fence(&image.fd, &finished).map_err(|error| error.to_string())
            }
            Err(error) => Err(error.to_string()),
        };
        let Err(error) = attached else {
            return Ok(());
        };
        drop(fence);
        let waited = wait(device, submission);
        self.give_up_fences(&error);
        waited
    }

    pub(crate) fn give_up_fences(&self, why: &str) {
        let taken = self
            .fence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(fence) = taken else {
            return;
        };
        tracing::warn!("frames wait for the GPU from now on, they cannot carry a fence: {why}");
        // SAFETY: the device is idle once the wait returns, so no submission still refers to the
        // semaphore, whatever state the failed export left it in.
        unsafe {
            if let Err(error) = self.device.device_wait_idle() {
                tracing::warn!("cannot wait for the GPU before freeing a fence: {error}");
            }
            self.device.destroy_semaphore(fence.semaphore, None);
        }
    }

    fn hand_over(
        &self,
        device: &wgpu::Device,
        image: vk::Image,
        acquire: bool,
    ) -> Result<wgpu::CommandBuffer, DrawError> {
        let attachment = (
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        );
        let barrier = vk::ImageMemoryBarrier::default()
            .image(image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        let (barrier, from, to) = match acquire {
            true => (
                barrier
                    .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                    .dst_queue_family_index(self.family)
                    .old_layout(vk::ImageLayout::GENERAL)
                    .new_layout(attachment.0)
                    .dst_access_mask(attachment.2),
                vk::PipelineStageFlags::TOP_OF_PIPE,
                attachment.1,
            ),
            false => (
                barrier
                    .src_queue_family_index(self.family)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                    .old_layout(attachment.0)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE),
                attachment.1,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            ),
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some(match acquire {
                true => "toolkit frame from the compositor",
                false => "toolkit frame to the compositor",
            }),
        });
        // SAFETY: the encoder records only this barrier, in its own command buffer submitted next
        // to the frame's, on the queue of `family`; `image` is a live shared image created with
        // that family, and the barrier leaves it in the layout wgpu last saw (colour attachment)
        // on our side and in GENERAL, owned by the foreign queue, between frames, as the
        // compositor acquires and releases it.
        let recorded = unsafe {
            encoder.as_hal_mut::<Vulkan, _, _>(|hal| {
                hal.map(|hal| {
                    self.device.cmd_pipeline_barrier(
                        hal.raw_handle(),
                        from,
                        to,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[barrier],
                    );
                })
            })
        };
        recorded
            .map(|()| encoder.finish())
            .ok_or_else(|| DrawError::Gpu("cannot record a queue family transfer".to_owned()))
    }
}

impl Drop for Sharing {
    fn drop(&mut self) {
        let fence = self.fence.get_mut().unwrap_or_else(PoisonError::into_inner);
        if let Some(fence) = fence {
            // SAFETY: the device is idle once the wait returns, so no submission still refers
            // to the semaphore.
            unsafe {
                if let Err(error) = self.device.device_wait_idle() {
                    tracing::warn!("cannot wait for the GPU before freeing a fence: {error}");
                }
                self.device.destroy_semaphore(fence.semaphore, None);
            }
        }
    }
}

impl Fence {
    fn new(
        instance: &ash::Instance,
        physical: vk::PhysicalDevice,
        device: &ash::Device,
    ) -> Option<Self> {
        let info = vk::PhysicalDeviceExternalSemaphoreInfo::default().handle_type(SYNC_FD);
        let mut properties = vk::ExternalSemaphoreProperties::default();
        // SAFETY: info and properties live across the call and physical is a valid device.
        unsafe {
            instance.get_physical_device_external_semaphore_properties(
                physical,
                &info,
                &mut properties,
            );
        }
        if !properties
            .external_semaphore_features
            .contains(vk::ExternalSemaphoreFeatureFlags::EXPORTABLE)
        {
            return None;
        }
        let mut export = vk::ExportSemaphoreCreateInfo::default().handle_types(SYNC_FD);
        let info = vk::SemaphoreCreateInfo::default().push_next(&mut export);
        // SAFETY: info and the structure it chains live across the call.
        let semaphore = unsafe { device.create_semaphore(&info, None) }
            .inspect_err(|error| tracing::debug!("cannot create an exportable fence: {error}"))
            .ok()?;
        Some(Self {
            semaphore,
            export: khr::external_semaphore_fd::Device::new(instance, device),
        })
    }

    fn export(&self) -> Result<Option<OwnedFd>, DrawError> {
        let info = vk::SemaphoreGetFdInfoKHR::default()
            .semaphore(self.semaphore)
            .handle_type(SYNC_FD);
        // SAFETY: the semaphore has a signal operation pending from the submission just made;
        // exporting it as a sync file unsignals it for the next frame.
        let fd = unsafe { self.export.get_semaphore_fd(&info) }
            .map_err(failed("cannot export the frame's fence"))?;
        // SAFETY: a non-negative fd from vkGetSemaphoreFdKHR is new and owned by nobody else; -1
        // means the frame is already finished and there is nothing to own.
        Ok((fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) }))
    }
}

fn wait(device: &wgpu::Device, submission: wgpu::SubmissionIndex) -> Result<(), DrawError> {
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .map(|_| ())
        .map_err(|error| DrawError::Gpu(error.to_string()))
}

fn wait_for_readers(dmabuf: &OwnedFd) {
    let mut readable = libc::pollfd {
        fd: dmabuf.as_raw_fd(),
        events: libc::POLLOUT,
        revents: 0,
    };
    let patience = libc::c_int::try_from(READERS_PATIENCE.as_millis()).unwrap_or(libc::c_int::MAX);
    // SAFETY: readable is one valid pollfd for an open descriptor and lives across the call.
    match unsafe { libc::poll(&mut readable, 1, patience) } {
        1.. => {}
        0 => tracing::warn!("the compositor is still reading a frame after {READERS_PATIENCE:?}"),
        _ => tracing::warn!(
            "cannot wait for the compositor to finish reading a frame: {}",
            io::Error::last_os_error()
        ),
    }
}

fn attach_fence(dmabuf: &OwnedFd, fence: &OwnedFd) -> io::Result<()> {
    let request = ImportSyncFile {
        flags: DMA_BUF_SYNC_WRITE,
        fd: fence.as_raw_fd(),
    };
    // SAFETY: DMA_BUF_IOCTL_IMPORT_SYNC_FILE reads one struct dma_buf_import_sync_file, which
    // request lays out, and both descriptors stay open across the call.
    let result =
        unsafe { libc::ioctl(dmabuf.as_raw_fd(), DMA_BUF_IOCTL_IMPORT_SYNC_FILE, &request) };
    match result {
        0.. => Ok(()),
        _ => Err(io::Error::last_os_error()),
    }
}

fn renderable_modifiers(instance: &ash::Instance, physical: vk::PhysicalDevice) -> Vec<u64> {
    let mut count = vk::DrmFormatModifierPropertiesListEXT::default();
    let mut properties = vk::FormatProperties2::default().push_next(&mut count);
    // SAFETY: properties and the list it chains live across the call.
    unsafe {
        instance.get_physical_device_format_properties2(physical, VULKAN_FORMAT, &mut properties);
    }
    let mut entries = vec![
        vk::DrmFormatModifierPropertiesEXT::default();
        count.drm_format_modifier_count as usize
    ];
    let mut list = vk::DrmFormatModifierPropertiesListEXT::default()
        .drm_format_modifier_properties(&mut entries);
    let mut properties = vk::FormatProperties2::default().push_next(&mut list);
    // SAFETY: as above, with room for every modifier the first call counted.
    unsafe {
        instance.get_physical_device_format_properties2(physical, VULKAN_FORMAT, &mut properties);
    }
    let needed =
        vk::FormatFeatureFlags::COLOR_ATTACHMENT | vk::FormatFeatureFlags::COLOR_ATTACHMENT_BLEND;
    entries
        .iter()
        .filter(|entry| {
            entry.drm_format_modifier_plane_count == 1
                && entry.drm_format_modifier_tiling_features.contains(needed)
        })
        .map(|entry| entry.drm_format_modifier)
        .filter(|&modifier| exportable(instance, physical, modifier))
        .collect()
}

fn exportable(instance: &ash::Instance, physical: vk::PhysicalDevice, modifier: u64) -> bool {
    let mut layout = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
        .drm_format_modifier(modifier)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let mut handle = vk::PhysicalDeviceExternalImageFormatInfo::default().handle_type(DMA_BUF);
    let info = vk::PhysicalDeviceImageFormatInfo2::default()
        .format(VULKAN_FORMAT)
        .ty(vk::ImageType::TYPE_2D)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(USAGE)
        .push_next(&mut layout)
        .push_next(&mut handle);
    let mut external = vk::ExternalImageFormatProperties::default();
    let mut properties = vk::ImageFormatProperties2::default().push_next(&mut external);
    // SAFETY: info, properties and the structures they chain live across the call.
    let supported = unsafe {
        instance.get_physical_device_image_format_properties2(physical, &info, &mut properties)
    }
    .is_ok();
    supported
        && external
            .external_memory_properties
            .external_memory_features
            .contains(vk::ExternalMemoryFeatureFlags::EXPORTABLE)
}
