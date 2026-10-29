use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};
use std::slice;

use rustix::fs::OFlags;
use rustix::mm::{MapFlags, ProtFlags, mmap, munmap};
use smithay::reexports::drm::buffer::DrmFourcc;
use smithay::reexports::drm::control::{
    Device as ControlDevice, crtc, framebuffer, plane, property,
};
use smithay::reexports::drm::{ClientCapability, Device};

pub struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl Device for Card {}
impl ControlDevice for Card {}

impl Card {
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(Card(OpenOptions::new().read(true).write(true).open(path)?))
    }

    pub fn with_writeback_connectors(self) -> io::Result<Self> {
        self.set_client_capability(ClientCapability::Atomic, true)?;
        self.set_client_capability(ClientCapability::WritebackConnectors, true)?;
        Ok(self)
    }

    pub fn driver(&self) -> String {
        let driver = self.get_driver().expect("DRM driver version");
        driver.name().to_string_lossy().into_owned()
    }

    pub fn plane_type(&self, plane: plane::Handle) -> String {
        let value_type = |handle| self.get_property(handle).ok().map(|info| info.value_type());
        self.plane_property(plane, "type")
            .and_then(|(handle, raw)| {
                value_type(handle)?
                    .convert_value(raw)
                    .as_enum()
                    .map(|value| value.name().to_string_lossy().into_owned())
            })
            .expect("every plane has a type")
    }

    fn plane_property(&self, plane: plane::Handle, name: &str) -> Option<(property::Handle, u64)> {
        let properties = self.get_properties(plane).expect("plane properties");
        properties.iter().find_map(|(&handle, &raw)| {
            let info = self.get_property(handle).ok()?;
            (info.name().to_bytes() == name.as_bytes()).then_some((handle, raw))
        })
    }

    fn plane_value(&self, plane: plane::Handle, name: &str) -> i64 {
        self.plane_property(plane, name)
            .map(|(_, raw)| raw as i64)
            .unwrap_or_else(|| panic!("plane property {name}"))
    }
}

pub fn cards() -> Vec<PathBuf> {
    let mut cards: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
        .expect("/dev/dri")
        .map(|entry| entry.expect("/dev/dri entry").path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("card"))
        })
        .collect();
    cards.sort();
    cards
}

pub fn card_path(driver: &str) -> PathBuf {
    cards()
        .into_iter()
        .find(|path| Card::open(path).expect("open DRM card").driver() == driver)
        .unwrap_or_else(|| panic!("no DRM card driven by {driver}"))
}

pub fn card_with_driver(driver: &str) -> Card {
    Card::open(&card_path(driver)).expect("open DRM card")
}

pub struct Scanout {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<[u8; 3]>,
}

impl Scanout {
    pub fn save(&self, name: &str) {
        let Some(directory) = std::env::var_os("UNBOUND_SCREENDUMPS") else {
            return;
        };
        let bytes: Vec<u8> = self.rgb.iter().flatten().copied().collect();
        let image = image::RgbImage::from_raw(self.width as u32, self.height as u32, bytes)
            .expect("screendump dimensions");
        std::fs::create_dir_all(&directory).expect("screendump directory");
        image
            .save(Path::new(&directory).join(format!("{name}.png")))
            .expect("save the screendump");
    }
}

impl Card {
    pub fn scanout(&self) -> Scanout {
        self.current_scanout().expect("a framebuffer on the CRTC")
    }

    pub fn current_scanout(&self) -> Option<Scanout> {
        let resources = self.resource_handles().expect("KMS resources");
        let crtc = resources.crtcs()[0];
        let primary = self.get_crtc(crtc).expect("CRTC state").framebuffer()?;
        let primary = self.framebuffer_pixels(primary);
        let mut screen = primary.bgra;
        let (width, height) = (primary.width, primary.height);
        let overlays = self.overlays_on(crtc);
        for (_, plane, framebuffer) in overlays {
            let source = self.framebuffer_pixels(framebuffer);
            let value = |name| self.plane_value(plane, name);
            let (crtc_x, crtc_y, crtc_w, crtc_h) = (
                value("CRTC_X"),
                value("CRTC_Y"),
                value("CRTC_W"),
                value("CRTC_H"),
            );
            let (src_x, src_y, src_w, src_h) = (
                value("SRC_X") >> 16,
                value("SRC_Y") >> 16,
                value("SRC_W") >> 16,
                value("SRC_H") >> 16,
            );
            for y in crtc_y.max(0)..(crtc_y + crtc_h).min(height as i64) {
                for x in crtc_x.max(0)..(crtc_x + crtc_w).min(width as i64) {
                    let sx = (src_x + (x - crtc_x) * src_w / crtc_w) as usize;
                    let sy = (src_y + (y - crtc_y) * src_h / crtc_h) as usize;
                    let below = &mut screen[y as usize * width + x as usize];
                    *below = over(source.bgra[sy * source.width + sx], *below, source.format);
                }
            }
        }
        let rgb = screen
            .iter()
            .map(|bgra| [bgra[2], bgra[1], bgra[0]])
            .collect();
        Some(Scanout { width, height, rgb })
    }

    pub fn overlay_planes_in_use(&self) -> usize {
        let resources = self.resource_handles().expect("KMS resources");
        self.overlays_on(resources.crtcs()[0]).len()
    }

    fn overlays_on(&self, crtc: crtc::Handle) -> Vec<(u64, plane::Handle, framebuffer::Handle)> {
        let mut overlays: Vec<(u64, plane::Handle, framebuffer::Handle)> = self
            .plane_handles()
            .expect("planes")
            .into_iter()
            .filter_map(|plane| {
                let info = self.get_plane(plane).ok()?;
                let framebuffer = info.framebuffer()?;
                (info.crtc() == Some(crtc) && self.plane_type(plane) == "Overlay").then(|| {
                    let zpos = self.plane_property(plane, "zpos").map_or(0, |(_, raw)| raw);
                    (zpos, plane, framebuffer)
                })
            })
            .collect();
        overlays.sort_by_key(|&(zpos, _, _)| zpos);
        overlays
    }

    fn framebuffer_pixels(&self, framebuffer: framebuffer::Handle) -> Framebuffer {
        let info = self
            .get_planar_framebuffer(framebuffer)
            .expect("framebuffer layout");
        let format = info.pixel_format();
        assert!(
            matches!(format, DrmFourcc::Xrgb8888 | DrmFourcc::Argb8888),
            "scanned-out format {format:?}"
        );
        let handle = info.buffers()[0].expect("buffer of plane 0");
        let (width, height) = info.size();
        let (width, height) = (width as usize, height as usize);
        let pitch = info.pitches()[0] as usize;
        let offset = info.offsets()[0] as usize;
        let length = offset + pitch * height;
        let fd = self
            .buffer_to_prime_fd(handle, OFlags::CLOEXEC.bits())
            .expect("export the scanned-out buffer");
        let mapped = unsafe {
            mmap(
                std::ptr::null_mut(),
                length,
                ProtFlags::READ,
                MapFlags::SHARED,
                &fd,
                0,
            )
        }
        .expect("map the scanned-out buffer");
        let bytes = unsafe { slice::from_raw_parts(mapped.cast::<u8>(), length) };
        let bgra = (0..height)
            .flat_map(|y| {
                bytes[offset + y * pitch..offset + y * pitch + width * 4]
                    .as_chunks::<4>()
                    .0
                    .to_vec()
            })
            .collect();
        unsafe { munmap(mapped, length) }.expect("unmap the scanned-out buffer");
        self.close_buffer(handle).expect("close the GEM handle");
        Framebuffer {
            width,
            format,
            bgra,
            height,
        }
    }
}

struct Framebuffer {
    width: usize,
    height: usize,
    format: DrmFourcc,
    bgra: Vec<[u8; 4]>,
}

fn over(source: [u8; 4], below: [u8; 4], format: DrmFourcc) -> [u8; 4] {
    let alpha = match format {
        DrmFourcc::Argb8888 => u32::from(source[3]),
        _ => 0xff,
    };
    let blend = |channel: usize| {
        (u32::from(source[channel]) + u32::from(below[channel]) * (255 - alpha) / 255).min(255)
            as u8
    };
    [blend(0), blend(1), blend(2), 0xff]
}
