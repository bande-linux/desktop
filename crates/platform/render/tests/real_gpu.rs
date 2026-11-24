use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;

use ash::vk;
use bande_render::{OffscreenTarget, VulkanRenderer, vulkan_errors};
use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf, DmabufFlags};
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBuffer, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Buffer, Format, Fourcc};
use smithay::backend::renderer::{Bind, Color32F, Frame, ImportDma, Offscreen, Renderer, Texture};
use smithay::reexports::rustix::{ioctl, mm};
use smithay::utils::{Physical, Rectangle, Transform};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

const SIZE: (i32, i32) = (64, 48);
const FRAMES: usize = 3;
const COLOUR: [u8; 3] = [0x20, 0x80, 0xc0];
const GREY: u8 = 130;
const DMA_BUF_SYNC_WRITE: u64 = 2;
const DMA_BUF_SYNC_END: u64 = 4;

struct Gpu {
    renderer: VulkanRenderer,
    gbm: GbmDevice<Arc<File>>,
    exporter: Exporter,
}

fn hardware() -> Outcome<Option<Gpu>> {
    let mut nodes: Vec<_> = std::fs::read_dir("/dev/dri")
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    nodes.retain(|path| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("renderD"))
    });
    nodes.sort();
    for node in nodes {
        let Ok(file) = OpenOptions::new().read(true).write(true).open(&node) else {
            continue;
        };
        let device = file.metadata()?.rdev();
        let Ok(mut renderer) = VulkanRenderer::new(Some(device)) else {
            continue;
        };
        let info = renderer.device_info();
        if !info.software && info.render_device == Some(device) {
            println!("testing {info} on {}", node.display());
            let gbm = GbmDevice::new(Arc::new(file))?;
            let mut allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING);
            renderer.prove_unexported(|format| {
                let buffer = allocator
                    .create_buffer(64, 64, format.code, &[format.modifier])
                    .ok()?;
                (buffer.format() == format)
                    .then(|| buffer.export().ok())
                    .flatten()
            });
            return Ok(Some(Gpu {
                renderer,
                gbm,
                exporter: Exporter::new(device)?,
            }));
        }
    }
    Ok(None)
}

struct Exporter {
    _entry: ash::Entry,
    instance: ash::Instance,
    physical: vk::PhysicalDevice,
    device: ash::Device,
    memory_fd: ash::khr::external_memory_fd::Device,
    images: Vec<(vk::Image, vk::DeviceMemory)>,
}

impl Exporter {
    fn new(render_device: u64) -> Outcome<Self> {
        // SAFETY: loading runs the system Vulkan loader once; `_entry` keeps it loaded.
        let entry = unsafe { ash::Entry::load() }?;
        let application = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_3);
        // SAFETY: the create info outlives the call; `Drop` destroys the instance.
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default().application_info(&application),
                None,
            )
        }?;
        // SAFETY: `instance` is live.
        let physicals = unsafe { instance.enumerate_physical_devices() }?;
        let physical = physicals
            .into_iter()
            .find(|&physical| {
                let mut node = vk::PhysicalDeviceDrmPropertiesEXT::default();
                let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut node);
                // SAFETY: `physical` belongs to `instance` and the chained struct outlives the call.
                unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
                node.has_render == vk::TRUE
                    && smithay::reexports::rustix::fs::makedev(
                        node.render_major as u32,
                        node.render_minor as u32,
                    ) == render_device
            })
            .ok_or("no Vulkan device for the render node")?;
        let extensions = [
            ash::khr::external_memory_fd::NAME.as_ptr(),
            ash::ext::external_memory_dma_buf::NAME.as_ptr(),
            ash::ext::image_drm_format_modifier::NAME.as_ptr(),
        ];
        let queue = vk::DeviceQueueCreateInfo::default().queue_priorities(&[1.0]);
        // SAFETY: `physical` belongs to `instance`; the create info outlives the call.
        let device = unsafe {
            instance.create_device(
                physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(std::slice::from_ref(&queue))
                    .enabled_extension_names(&extensions),
                None,
            )
        }?;
        Ok(Self {
            memory_fd: ash::khr::external_memory_fd::Device::new(&instance, &device),
            _entry: entry,
            instance,
            physical,
            device,
            images: Vec::new(),
        })
    }

    fn planes(&self, format: vk::Format, modifier: u64) -> Option<usize> {
        let mut modifiers = vec![vk::DrmFormatModifierPropertiesEXT::default(); 64];
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default()
            .drm_format_modifier_properties(&mut modifiers);
        let mut properties = vk::FormatProperties2::default().push_next(&mut list);
        // SAFETY: `physical` belongs to `instance`; `list` has room for 64 modifiers.
        unsafe {
            self.instance.get_physical_device_format_properties2(
                self.physical,
                format,
                &mut properties,
            )
        };
        let count = list.drm_format_modifier_count as usize;
        modifiers[..count.min(64)]
            .iter()
            .find(|listed| listed.drm_format_modifier == modifier)
            .map(|listed| listed.drm_format_modifier_plane_count as usize)
    }

    fn export(&mut self, format: Format, texel: vk::Format) -> Outcome<Option<Dmabuf>> {
        let modifier = u64::from(format.modifier);
        let Some(planes) = self.planes(texel, modifier) else {
            println!("skipped {format:?}: Vulkan cannot allocate it");
            return Ok(None);
        };
        if planes != 2 {
            println!("skipped {format:?}: its compression planes cannot be filled by the CPU");
            return Ok(None);
        }
        let modifiers = [modifier];
        let mut list =
            vk::ImageDrmFormatModifierListCreateInfoEXT::default().drm_format_modifiers(&modifiers);
        let mut external = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let (width, height) = (SIZE.0.unsigned_abs(), SIZE.1.unsigned_abs());
        // SAFETY: the create info and its chain outlive the call; `Drop` destroys the image.
        let image = unsafe {
            self.device.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(texel)
                    .extent(vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                    .usage(vk::ImageUsageFlags::SAMPLED)
                    .push_next(&mut list)
                    .push_next(&mut external),
                None,
            )
        }?;
        self.images.push((image, vk::DeviceMemory::null()));
        // SAFETY: `image` is a live image of this device.
        let requirements = unsafe { self.device.get_image_memory_requirements(image) };
        let mut export = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        // SAFETY: the allocate info and its chain outlive the call; `Drop` frees the memory.
        let memory = unsafe {
            self.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(requirements.memory_type_bits.trailing_zeros())
                    .push_next(&mut export)
                    .push_next(&mut dedicated),
                None,
            )
        }?;
        self.images.push((vk::Image::null(), memory));
        // SAFETY: `memory` was allocated for `image`, which has none bound.
        unsafe { self.device.bind_image_memory(image, memory, 0) }?;
        // SAFETY: `memory` was allocated exportable as a dma-buf.
        let fd = unsafe {
            self.memory_fd.get_memory_fd(
                &vk::MemoryGetFdInfoKHR::default()
                    .memory(memory)
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT),
            )
        }?;
        // SAFETY: `get_memory_fd` hands over a new fd that nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut builder = Dmabuf::builder(
            (SIZE.0, SIZE.1),
            format.code,
            format.modifier,
            DmabufFlags::empty(),
        );
        let aspects = [
            vk::ImageAspectFlags::MEMORY_PLANE_0_EXT,
            vk::ImageAspectFlags::MEMORY_PLANE_1_EXT,
            vk::ImageAspectFlags::MEMORY_PLANE_2_EXT,
            vk::ImageAspectFlags::MEMORY_PLANE_3_EXT,
        ];
        for (index, aspect) in aspects.into_iter().take(planes).enumerate() {
            // SAFETY: `image` has a DRM format modifier with `planes` memory planes.
            let layout = unsafe {
                self.device.get_image_subresource_layout(
                    image,
                    vk::ImageSubresource::default().aspect_mask(aspect),
                )
            };
            builder.add_plane(
                fd.try_clone()?,
                index as u32,
                u32::try_from(layout.offset)?,
                u32::try_from(layout.row_pitch)?,
            );
        }
        Ok(builder.build())
    }
}

impl Drop for Exporter {
    fn drop(&mut self) {
        // SAFETY: nothing uses the images, their memory, the device or the instance any more.
        unsafe {
            for (image, memory) in self.images.drain(..) {
                self.device.destroy_image(image, None);
                self.device.free_memory(memory, None);
            }
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

fn packed(code: Fourcc, colour: [f32; 3]) -> Option<[u8; 4]> {
    let [red, green, blue] = colour.map(|value| (value * 255.0).round() as u8);
    let [red_ten, green_ten, blue_ten] = colour.map(|value| (value * 1023.0).round() as u32);
    let pixel = match code {
        Fourcc::Argb8888 | Fourcc::Xrgb8888 => return Some([blue, green, red, 0xff]),
        Fourcc::Abgr8888 | Fourcc::Xbgr8888 => return Some([red, green, blue, 0xff]),
        Fourcc::Argb2101010 | Fourcc::Xrgb2101010 => {
            3 << 30 | red_ten << 20 | green_ten << 10 | blue_ten
        }
        Fourcc::Abgr2101010 | Fourcc::Xbgr2101010 => {
            3 << 30 | blue_ten << 20 | green_ten << 10 | red_ten
        }
        _ => return None,
    };
    Some(pixel.to_le_bytes())
}

fn video(code: Fourcc) -> Option<(vk::Format, [&'static [u8]; 2])> {
    match code {
        Fourcc::Nv12 => Some((
            vk::Format::G8_B8R8_2PLANE_420_UNORM,
            [&[0x80], &[0x80, 0x80]],
        )),
        Fourcc::P010 => Some((
            vk::Format::G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16,
            [&[0x00, 0x80], &[0x00, 0x80, 0x00, 0x80]],
        )),
        _ => None,
    }
}

fn dma_buf_sync(file: &File, flags: u64) -> std::io::Result<()> {
    const SYNC: ioctl::Opcode = ioctl::opcode::write::<u64>(b'b', 0);
    // SAFETY: `DMA_BUF_IOCTL_SYNC` is `_IOW('b', 0, struct dma_buf_sync)`, a single `__u64`.
    let request = unsafe { ioctl::Setter::<SYNC, u64>::new(flags) };
    // SAFETY: `file` is the dma-buf the request was encoded for.
    unsafe { ioctl::ioctl(file, request) }?;
    Ok(())
}

fn fill_planes(dmabuf: &Dmabuf, patterns: [&[u8]; 2]) -> std::io::Result<()> {
    let offsets: Vec<u32> = dmabuf.offsets().collect();
    for (index, (fd, pattern)) in dmabuf.handles().zip(patterns).enumerate() {
        let file = File::from(fd.try_clone_to_owned()?);
        let length = (&file).seek(SeekFrom::End(0))? as usize;
        let start = offsets[index] as usize;
        let end = offsets.get(index + 1).map_or(length, |&next| next as usize);
        // SAFETY: a fresh shared mapping of the whole dma-buf, unmapped below before `file` closes.
        let mapping = unsafe {
            mm::mmap(
                std::ptr::null_mut(),
                length,
                mm::ProtFlags::READ | mm::ProtFlags::WRITE,
                mm::MapFlags::SHARED,
                &file,
                0,
            )
        }?;
        dma_buf_sync(&file, DMA_BUF_SYNC_WRITE)?;
        // SAFETY: `mapping` holds `length` bytes, nothing else accesses it, and `start..end` lies
        // within it.
        let bytes = unsafe { std::slice::from_raw_parts_mut(mapping.cast::<u8>(), length) };
        for (byte, value) in bytes[start..end].iter_mut().zip(pattern.iter().cycle()) {
            *byte = *value;
        }
        dma_buf_sync(&file, DMA_BUF_SYNC_END | DMA_BUF_SYNC_WRITE)?;
        // SAFETY: `mapping` and `length` are exactly what `mmap` returned.
        unsafe { mm::munmap(mapping, length) }?;
    }
    Ok(())
}

fn fill_colour(buffer: &mut GbmBuffer, pixel: [u8; 4]) -> std::io::Result<()> {
    let (width, height) = (SIZE.0.unsigned_abs(), SIZE.1.unsigned_abs());
    buffer.map_mut(0, 0, width, height, |mapped| {
        let stride = mapped.stride() as usize;
        for row in mapped.buffer_mut().chunks_mut(stride) {
            for (byte, value) in row[..width as usize * 4]
                .iter_mut()
                .zip(pixel.iter().cycle())
            {
                *byte = *value;
            }
        }
    })
}

fn clear(
    renderer: &mut VulkanRenderer,
    dmabuf: &mut Dmabuf,
    [red, green, blue]: [f32; 3],
    damage: Rectangle<i32, Physical>,
) -> Outcome {
    let mut framebuffer = renderer.bind(dmabuf)?;
    let mut frame = renderer.render(&mut framebuffer, SIZE.into(), Transform::Normal)?;
    frame.clear(Color32F::new(red, green, blue, 1.0), &[damage])?;
    frame.finish()?.wait()?;
    Ok(())
}

fn drawn(renderer: &mut VulkanRenderer, dmabuf: &Dmabuf) -> Outcome<[u8; 4]> {
    let mut target: OffscreenTarget = renderer.create_buffer(Fourcc::Argb8888, SIZE.into())?;
    let whole = Rectangle::from_size(SIZE.into());
    for _ in 0..FRAMES {
        let texture = renderer.import_dmabuf(dmabuf, None)?;
        let size = (f64::from(texture.width()), f64::from(texture.height()));
        let mut framebuffer = renderer.bind(&mut target)?;
        let mut frame = renderer.render(&mut framebuffer, SIZE.into(), Transform::Normal)?;
        frame.clear(Color32F::new(1.0, 0.0, 1.0, 1.0), &[whole])?;
        frame.render_texture_from_to(
            &texture,
            Rectangle::from_size(size.into()),
            whole,
            &[whole],
            &[],
            Transform::Normal,
            1.0,
        )?;
        frame.finish()?.wait()?;
    }
    let pixels = renderer.read_target(&target)?;
    let centre = ((SIZE.1 / 2 * SIZE.0 + SIZE.0 / 2) * 4) as usize;
    Ok([0, 1, 2, 3].map(|channel| pixels[centre + channel]))
}

fn close(found: [u8; 4], wanted: [u8; 4], tolerance: u8) -> bool {
    found
        .iter()
        .zip(wanted)
        .all(|(found, wanted)| found.abs_diff(wanted) <= tolerance)
}

fn allocate(
    gbm: &GbmDevice<Arc<File>>,
    format: Format,
    flags: GbmBufferFlags,
) -> Option<GbmBuffer> {
    let mut allocator = GbmAllocator::new(gbm.clone(), flags);
    let (width, height) = (SIZE.0.unsigned_abs(), SIZE.1.unsigned_abs());
    match allocator.create_buffer(width, height, format.code, &[format.modifier]) {
        Ok(buffer) if buffer.format() == format => Some(buffer),
        Ok(buffer) => {
            println!("skipped {format:?}: GBM allocated {:?}", buffer.format());
            None
        }
        Err(error) => {
            println!("skipped {format:?}: GBM cannot allocate it ({error})");
            None
        }
    }
}

fn sample(gpu: &mut Gpu, format: Format) -> Outcome<bool> {
    let (dmabuf, wanted) = match (
        packed(format.code, COLOUR.map(|value| f32::from(value) / 255.0)),
        video(format.code),
    ) {
        (Some(pixel), _) => {
            let Some(mut buffer) = allocate(&gpu.gbm, format, GbmBufferFlags::RENDERING) else {
                return Ok(false);
            };
            if let Err(error) = fill_colour(&mut buffer, pixel) {
                if !gpu.renderer.render_formats().contains(&format) {
                    println!("skipped {format:?}: GBM cannot map it ({error})");
                    return Ok(false);
                }
                println!("{format:?}: GBM cannot map it ({error}), drawing it instead");
                let colour = COLOUR.map(|value| f32::from(value) / 255.0);
                let whole = Rectangle::from_size(SIZE.into());
                clear(&mut gpu.renderer, &mut buffer.export()?, colour, whole)?;
            }
            (buffer.export()?, [COLOUR[2], COLOUR[1], COLOUR[0], 0xff])
        }
        (None, Some((texel, planes))) => {
            let dmabuf = match gpu.exporter.export(format, texel) {
                Ok(Some(dmabuf)) => dmabuf,
                Ok(None) => return Ok(false),
                Err(error) => {
                    println!("skipped {format:?}: Vulkan cannot export it ({error})");
                    return Ok(false);
                }
            };
            if let Err(error) = fill_planes(&dmabuf, planes) {
                println!("skipped {format:?}: the CPU cannot fill it ({error})");
                return Ok(false);
            }
            (dmabuf, [GREY, GREY, GREY, 0xff])
        }
        (None, None) => return Err(format!("no test pattern for {format:?}").into()),
    };
    let found = drawn(&mut gpu.renderer, &dmabuf)?;
    println!("sampled {format:?}: {found:?}");
    assert!(
        close(found, wanted, 3),
        "{format:?} drew {found:?}, not {wanted:?}"
    );
    Ok(true)
}

fn foreign_pixels(
    gbm: &GbmDevice<Arc<File>>,
    dmabuf: &Dmabuf,
    points: [(u32, u32); 2],
) -> Outcome<[[u8; 4]; 2]> {
    let mut fds = [None; 4];
    let (mut strides, mut offsets) = ([0; 4], [0; 4]);
    for (index, ((fd, offset), stride)) in dmabuf
        .handles()
        .zip(dmabuf.offsets())
        .zip(dmabuf.strides())
        .enumerate()
    {
        fds[index] = Some(fd);
        offsets[index] = i32::try_from(offset)?;
        strides[index] = i32::try_from(stride)?;
    }
    let (width, height) = (SIZE.0.unsigned_abs(), SIZE.1.unsigned_abs());
    let imported = gbm.import_buffer_object_from_dma_buf_with_modifiers::<()>(
        dmabuf.num_planes() as u32,
        fds,
        width,
        height,
        dmabuf.format().code,
        GbmBufferFlags::RENDERING,
        strides,
        offsets,
        dmabuf.format().modifier,
    )?;
    let mut pixels = [[0; 4]; 2];
    for (pixel, (x, y)) in pixels.iter_mut().zip(points) {
        *pixel = imported.map(x, y, 1, 1, |mapped| {
            <[u8; 4]>::try_from(&mapped.buffer()[..4])
        })??;
    }
    Ok(pixels)
}

fn scan_out(gpu: &mut Gpu, format: Format) -> Outcome<bool> {
    let flags = GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT;
    let Some(buffer) = allocate(&gpu.gbm, format, flags) else {
        return Ok(false);
    };
    let mut dmabuf = buffer.export()?;
    let whole = Rectangle::from_size(SIZE.into());
    let colours = [[0.75, 0.25, 0.5], [0.25, 0.5, 0.75], [0.5, 0.75, 0.25]];
    for colour in colours {
        clear(&mut gpu.renderer, &mut dmabuf, colour, whole)?;
    }
    let corner = [0.0, 0.0, 1.0];
    let damage = Rectangle::from_size((16, 16).into());
    clear(&mut gpu.renderer, &mut dmabuf, corner, damage)?;
    let bytes = |colour: [f32; 3]| packed(format.code, colour).ok_or("not a colour format");
    let mut wanted = [bytes(corner)?, bytes(colours[FRAMES - 1])?];
    let centre = (SIZE.0.unsigned_abs() / 2, SIZE.1.unsigned_abs() / 2);
    let found = match foreign_pixels(&gpu.gbm, &dmabuf, [(4, 4), centre]) {
        Ok(found) => found,
        Err(error) => {
            println!("scan-out not checked for {format:?}: GBM cannot read it back ({error})");
            return Ok(false);
        }
    };
    println!("scanned out {format:?}: {found:?}");
    if matches!(format.code, Fourcc::Xrgb8888 | Fourcc::Xbgr8888) {
        for (wanted, found) in wanted.iter_mut().zip(found) {
            wanted[3] = found[3];
        }
    }
    for (found, wanted) in found.into_iter().zip(wanted) {
        assert!(
            close(found, wanted, 1),
            "{format:?} kept {found:?} after a partial frame, not {wanted:?}"
        );
    }
    Ok(true)
}

fn client_frames(gpu: &mut Gpu, client: &mut VulkanRenderer, format: Format) -> Outcome<bool> {
    let mut buffers = Vec::new();
    for _ in 0..2 {
        let Some(buffer) = allocate(&gpu.gbm, format, GbmBufferFlags::RENDERING) else {
            return Ok(false);
        };
        let dmabuf = buffer.export()?;
        buffers.push((buffer, dmabuf));
    }
    let colours = [
        [0.75, 0.25, 0.5],
        [0.25, 0.5, 0.75],
        [0.5, 0.75, 0.25],
        [1.0, 1.0, 0.0],
    ];
    let whole = Rectangle::from_size(SIZE.into());
    for (frame, colour) in colours.iter().cycle().take(12).enumerate() {
        let (_, dmabuf) = &mut buffers[frame % 2];
        clear(client, dmabuf, *colour, whole)?;
        let found = drawn(&mut gpu.renderer, dmabuf)?;
        let [red, green, blue] = colour.map(|value| (value * 255.0).round() as u8);
        let wanted = [blue, green, red, 0xff];
        assert!(
            close(found, wanted, 1),
            "frame {frame} of a client drawing {format:?} showed {found:?}, not {wanted:?}"
        );
    }
    println!("sampled a client redrawing {format:?} every frame");
    Ok(true)
}

#[test]
fn every_advertised_dma_buf_format_draws_and_scans_out_on_the_gpu() -> Outcome {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let Some(mut gpu) = hardware()? else {
        println!("skipped: no hardware Vulkan device with a DRM render node (lavapipe only)");
        return Ok(());
    };
    let order = |format: &Format| (format.code as u32, u64::from(format.modifier));
    let sampled_formats = gpu.renderer.dmabuf_formats();
    let mut sampled: Vec<Format> = sampled_formats.iter().copied().collect();
    sampled.sort_by_key(order);
    let sampled_total = sampled.len();
    let mut sampled_verified = 0;
    for format in sampled {
        sampled_verified += usize::from(sample(&mut gpu, format)?);
    }
    let mut rendered: Vec<Format> = gpu.renderer.render_formats().iter().copied().collect();
    rendered.sort_by_key(order);
    let mut client = VulkanRenderer::new(gpu.renderer.device_info().render_device)?;
    for format in rendered
        .iter()
        .filter(|format| sampled_formats.contains(format))
    {
        client_frames(&mut gpu, &mut client, *format)?;
    }
    let rendered_total = rendered.len();
    let mut rendered_verified = 0;
    for format in rendered {
        rendered_verified += usize::from(scan_out(&mut gpu, format)?);
    }
    println!(
        "verified {sampled_verified} of {sampled_total} sampled and {rendered_verified} of \
         {rendered_total} scanned-out formats; the others are skipped above"
    );
    assert!(sampled_verified > 0, "no sampled format could be checked");
    if rendered_total > 0 && rendered_verified == 0 {
        println!("scan-out not checked: GBM could read none of the scanout buffers back");
    }
    let refused = gpu.renderer.refused_formats();
    assert!(refused.is_empty(), "the driver refused {refused:?}");
    assert_eq!(vulkan_errors(), 0, "Vulkan reported errors");
    Ok(())
}
