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

pub(crate) fn is_deep(code: Fourcc) -> bool {
    texel(code).is_some_and(|texel| match texel.kind {
        Kind::Colour { .. } => crate::is_deep_target(texel.format),
        Kind::Video { luma, .. } => luma == vk::Format::R16_UNORM,
    })
}

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

    pub(crate) fn view_formats(self) -> Vec<vk::Format> {
        match self.kind {
            Kind::Colour { .. } => Vec::new(),
            Kind::Video { luma, chroma } => vec![luma, chroma],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Capability {
    pub(crate) planes: usize,
    pub(crate) disjoint: bool,
    pub(crate) exportable: bool,
    pub(crate) max_extent: (u32, u32),
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Supported(HashMap<Format, Capability>);

impl Supported {
    pub(crate) fn usable(&self, format: &Format) -> Option<Capability> {
        self.0.get(format).copied()
    }

    pub(crate) fn listed(&self) -> impl Iterator<Item = (Format, Capability)> + '_ {
        self.0
            .iter()
            .map(|(format, capability)| (*format, *capability))
    }

    pub(crate) fn planes(&self) -> impl Iterator<Item = (Format, usize)> + '_ {
        self.0
            .iter()
            .map(|(format, capability)| (*format, capability.planes))
    }

    pub(crate) fn formats(&self) -> impl Iterator<Item = Format> + '_ {
        self.0.keys().copied()
    }

    pub(crate) fn remove(&mut self, format: &Format) {
        self.0.remove(format);
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Modifiers {
    pub(crate) sampled: Supported,
    pub(crate) rendered: Supported,
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

struct Importable {
    max_extent: (u32, u32),
    dedicated_only: bool,
    exportable: bool,
}

fn importable(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    texel: Texel,
    modifier: u64,
    (usage, flags): (vk::ImageUsageFlags, vk::ImageCreateFlags),
) -> Option<Importable> {
    let mut modifier_info = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
        .drm_format_modifier(modifier)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let mut external_info = vk::PhysicalDeviceExternalImageFormatInfo::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let view_formats = texel.view_formats();
    let mut list = vk::ImageFormatListCreateInfo::default().view_formats(&view_formats);
    let mut info = vk::PhysicalDeviceImageFormatInfo2::default()
        .format(texel.format)
        .ty(vk::ImageType::TYPE_2D)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(usage)
        .flags(texel.flags() | flags)
        .push_next(&mut modifier_info)
        .push_next(&mut external_info);
    if !view_formats.is_empty() {
        info = info.push_next(&mut list);
    }
    let mut external = vk::ExternalImageFormatProperties::default();
    let mut result = vk::ImageFormatProperties2::default().push_next(&mut external);
    // SAFETY: `physical` belongs to this `instance`; `info`, `result` and the structs chained into
    // them outlive the call.
    unsafe { instance.get_physical_device_image_format_properties2(physical, &info, &mut result) }
        .ok()?;
    let extent = result.image_format_properties.max_extent;
    let features = external.external_memory_properties.external_memory_features;
    features
        .contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
        .then_some(Importable {
            max_extent: (extent.width, extent.height),
            dedicated_only: features.contains(vk::ExternalMemoryFeatureFlags::DEDICATED_ONLY),
            exportable: features.contains(vk::ExternalMemoryFeatureFlags::EXPORTABLE),
        })
}

fn capability(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    texel: Texel,
    listed: vk::DrmFormatModifierPropertiesEXT,
    usage: vk::ImageUsageFlags,
) -> Option<Capability> {
    let modifier = listed.drm_format_modifier;
    let found = importable(
        instance,
        physical,
        texel,
        modifier,
        (usage, vk::ImageCreateFlags::empty()),
    )?;
    let planes = listed.drm_format_modifier_plane_count as usize;
    let disjoint = planes > 1
        && listed
            .drm_format_modifier_tiling_features
            .contains(vk::FormatFeatureFlags::DISJOINT)
        && importable(
            instance,
            physical,
            texel,
            modifier,
            (usage, vk::ImageCreateFlags::DISJOINT),
        )
        .is_some_and(|disjoint| !disjoint.dedicated_only);
    Some(Capability {
        planes,
        disjoint,
        exportable: found.exportable,
        max_extent: found.max_extent,
    })
}

const SAMPLE_FEATURES: vk::FormatFeatureFlags = vk::FormatFeatureFlags::from_raw(
    vk::FormatFeatureFlags::SAMPLED_IMAGE.as_raw()
        | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR.as_raw(),
);

const RENDER_FEATURES: vk::FormatFeatureFlags = vk::FormatFeatureFlags::from_raw(
    vk::FormatFeatureFlags::COLOR_ATTACHMENT.as_raw()
        | vk::FormatFeatureFlags::COLOR_ATTACHMENT_BLEND.as_raw()
        | SAMPLE_FEATURES.as_raw()
        | vk::FormatFeatureFlags::TRANSFER_SRC.as_raw()
        | vk::FormatFeatureFlags::TRANSFER_DST.as_raw(),
);

pub(crate) fn query(instance: &ash::Instance, physical: vk::PhysicalDevice) -> Modifiers {
    let mut found = Modifiers::default();
    for texel in TEXELS {
        for listed in listed_modifiers(instance, physical, texel.format) {
            let format = Format {
                code: texel.code,
                modifier: Modifier::from(listed.drm_format_modifier),
            };
            let features = listed.drm_format_modifier_tiling_features;
            if features.contains(SAMPLE_FEATURES)
                && let Some(sampled) = capability(instance, physical, texel, listed, SAMPLED)
            {
                found.sampled.0.insert(format, sampled);
            }
            if crate::TARGET_FORMATS.contains(&texel.format)
                && format.modifier != Modifier::Linear
                && features.contains(RENDER_FEATURES)
                && let Some(rendered) = capability(instance, physical, texel, listed, RENDERED)
            {
                found.rendered.0.insert(format, rendered);
            }
        }
    }
    found
}

impl Modifiers {
    pub(crate) fn any(&self) -> bool {
        !self.sampled.0.is_empty() || !self.rendered.0.is_empty()
    }
}
