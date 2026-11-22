use std::collections::HashMap;

use ash::vk;
use smithay::backend::allocator::{Format, Fourcc, Modifier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Colour {
        opaque: bool,
    },
    Video {
        luma: vk::Format,
        chroma: vk::Format,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Texel {
    pub(crate) code: Fourcc,
    pub(crate) format: vk::Format,
    pub(crate) kind: Kind,
}

impl Texel {
    pub(crate) fn plane_rows(&self, width: u32, height: u32) -> Vec<(u64, u64)> {
        let (width, height) = (u64::from(width), u64::from(height));
        let (half_width, half_height) = (width.div_ceil(2), height.div_ceil(2));
        match self.kind {
            Kind::Colour { .. } => vec![(width * 4, height)],
            Kind::Video { luma, .. } => {
                let sample = match luma {
                    vk::Format::R16_UNORM => 2,
                    _ => 1,
                };
                vec![
                    (width * sample, height),
                    (half_width * 2 * sample, half_height),
                ]
            }
        }
    }
}

const fn colour(code: Fourcc, format: vk::Format, opaque: bool) -> Texel {
    Texel {
        code,
        format,
        kind: Kind::Colour { opaque },
    }
}

const TEXELS: [Texel; 10] = [
    colour(Fourcc::Argb8888, vk::Format::B8G8R8A8_UNORM, false),
    colour(Fourcc::Xrgb8888, vk::Format::B8G8R8A8_UNORM, true),
    colour(Fourcc::Abgr8888, vk::Format::R8G8B8A8_UNORM, false),
    colour(Fourcc::Xbgr8888, vk::Format::R8G8B8A8_UNORM, true),
    colour(
        Fourcc::Argb2101010,
        vk::Format::A2R10G10B10_UNORM_PACK32,
        false,
    ),
    colour(
        Fourcc::Xrgb2101010,
        vk::Format::A2R10G10B10_UNORM_PACK32,
        true,
    ),
    colour(
        Fourcc::Abgr2101010,
        vk::Format::A2B10G10R10_UNORM_PACK32,
        false,
    ),
    colour(
        Fourcc::Xbgr2101010,
        vk::Format::A2B10G10R10_UNORM_PACK32,
        true,
    ),
    Texel {
        code: Fourcc::Nv12,
        format: vk::Format::G8_B8R8_2PLANE_420_UNORM,
        kind: Kind::Video {
            luma: vk::Format::R8_UNORM,
            chroma: vk::Format::R8G8_UNORM,
        },
    },
    Texel {
        code: Fourcc::P010,
        format: vk::Format::G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16,
        kind: Kind::Video {
            luma: vk::Format::R16_UNORM,
            chroma: vk::Format::R16G16_UNORM,
        },
    },
];

pub(crate) const SAMPLED: vk::ImageUsageFlags = vk::ImageUsageFlags::SAMPLED;
pub(crate) const RENDERED: vk::ImageUsageFlags = vk::ImageUsageFlags::from_raw(
    vk::ImageUsageFlags::COLOR_ATTACHMENT.as_raw()
        | vk::ImageUsageFlags::SAMPLED.as_raw()
        | vk::ImageUsageFlags::TRANSFER_SRC.as_raw()
        | vk::ImageUsageFlags::TRANSFER_DST.as_raw(),
);

pub(crate) fn texel(code: Fourcc) -> Option<Texel> {
    TEXELS.into_iter().find(|texel| texel.code == code)
}

impl Texel {
    pub(crate) fn flags(self) -> vk::ImageCreateFlags {
        match self.kind {
            Kind::Colour { .. } => vk::ImageCreateFlags::empty(),
            Kind::Video { .. } => vk::ImageCreateFlags::MUTABLE_FORMAT,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Modifiers {
    pub(crate) sampled: HashMap<Format, usize>,
    pub(crate) rendered: HashMap<Format, usize>,
}

fn listed_modifiers(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    format: vk::Format,
) -> Vec<vk::DrmFormatModifierPropertiesEXT> {
    let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
    let mut properties = vk::FormatProperties2::default().push_next(&mut list);
    // SAFETY: `physical` was enumerated from this `instance`, and `properties` with its chained
    // `list` outlives the call.
    unsafe { instance.get_physical_device_format_properties2(physical, format, &mut properties) };
    let mut modifiers = vec![
        vk::DrmFormatModifierPropertiesEXT::default();
        list.drm_format_modifier_count as usize
    ];
    let mut list = vk::DrmFormatModifierPropertiesListEXT::default()
        .drm_format_modifier_properties(&mut modifiers);
    let mut properties = vk::FormatProperties2::default().push_next(&mut list);
    // SAFETY: `physical` belongs to this `instance`; `list` now points at `modifiers`, which has
    // room for the `drm_format_modifier_count` entries the first query reported.
    unsafe { instance.get_physical_device_format_properties2(physical, format, &mut properties) };
    let count = list.drm_format_modifier_count as usize;
    modifiers.truncate(count);
    modifiers
}

fn importable(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    texel: Texel,
    modifier: u64,
    usage: vk::ImageUsageFlags,
) -> bool {
    let mut modifier_info = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
        .drm_format_modifier(modifier)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let mut external_info = vk::PhysicalDeviceExternalImageFormatInfo::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let info = vk::PhysicalDeviceImageFormatInfo2::default()
        .format(texel.format)
        .ty(vk::ImageType::TYPE_2D)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(usage)
        .flags(texel.flags())
        .push_next(&mut modifier_info)
        .push_next(&mut external_info);
    let mut external = vk::ExternalImageFormatProperties::default();
    let mut result = vk::ImageFormatProperties2::default().push_next(&mut external);
    // SAFETY: `physical` belongs to this `instance`; `info`, `result` and the structs chained into
    // them outlive the call.
    let supported = unsafe {
        instance.get_physical_device_image_format_properties2(physical, &info, &mut result)
    };
    supported.is_ok()
        && external
            .external_memory_properties
            .external_memory_features
            .contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
}

pub(crate) fn query(instance: &ash::Instance, physical: vk::PhysicalDevice) -> Modifiers {
    let mut found = Modifiers::default();
    for texel in TEXELS {
        for listed in listed_modifiers(instance, physical, texel.format) {
            let format = Format {
                code: texel.code,
                modifier: Modifier::from(listed.drm_format_modifier),
            };
            let features = listed.drm_format_modifier_tiling_features;
            let planes = listed.drm_format_modifier_plane_count as usize;
            let sampled = features.contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
                && importable(
                    instance,
                    physical,
                    texel,
                    listed.drm_format_modifier,
                    SAMPLED,
                );
            if sampled {
                found.sampled.insert(format, planes);
            }
            let rendered = crate::TARGET_FORMATS.contains(&texel.format)
                && format.modifier != Modifier::Linear
                && features.contains(
                    vk::FormatFeatureFlags::COLOR_ATTACHMENT
                        | vk::FormatFeatureFlags::TRANSFER_SRC
                        | vk::FormatFeatureFlags::TRANSFER_DST,
                )
                && importable(
                    instance,
                    physical,
                    texel,
                    listed.drm_format_modifier,
                    RENDERED,
                );
            if rendered {
                found.rendered.insert(format, planes);
            }
        }
    }
    found
}

impl Modifiers {
    pub(crate) fn any(&self) -> bool {
        !self.sampled.is_empty() || !self.rendered.is_empty()
    }
}
