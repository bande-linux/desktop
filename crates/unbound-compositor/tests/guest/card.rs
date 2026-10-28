use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};
use std::slice;

use rustix::fs::OFlags;
use rustix::mm::{MapFlags, ProtFlags, mmap, munmap};
use smithay::reexports::drm::buffer::DrmFourcc;
use smithay::reexports::drm::control::{Device as ControlDevice, plane};
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
        let properties = self.get_properties(plane).expect("plane properties");
        properties
            .iter()
            .find_map(|(&handle, &raw)| {
                let info = self.get_property(handle).ok()?;
                if info.name().to_bytes() != b"type" {
                    return None;
                }
                let value_type = info.value_type();
                let value = value_type.convert_value(raw);
                value
                    .as_enum()
                    .map(|value| value.name().to_string_lossy().into_owned())
            })
            .expect("every plane has a type")
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
        let resources = self.resource_handles().expect("KMS resources");
        let crtc = resources.crtcs()[0];
        let framebuffer = self
            .get_crtc(crtc)
            .expect("CRTC state")
            .framebuffer()
            .expect("a framebuffer on the CRTC");
        let info = self
            .get_planar_framebuffer(framebuffer)
            .expect("framebuffer layout");
        assert!(
            matches!(
                info.pixel_format(),
                DrmFourcc::Xrgb8888 | DrmFourcc::Argb8888
            ),
            "scanned-out format {:?}",
            info.pixel_format()
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
        let rgb = (0..height)
            .flat_map(|y| {
                let row = &bytes[offset + y * pitch..offset + y * pitch + width * 4];
                row.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|bgrx| [bgrx[2], bgrx[1], bgrx[0]])
                    .collect::<Vec<_>>()
            })
            .collect();
        unsafe { munmap(mapped, length) }.expect("unmap the scanned-out buffer");
        self.close_buffer(handle).expect("close the GEM handle");
        Scanout { width, height, rgb }
    }
}
