use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use rustix::ioctl::{Setter, Updater, ioctl, opcode};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_protocols::wp::linux_drm_syncobj::v1::client::{
    wp_linux_drm_syncobj_manager_v1::WpLinuxDrmSyncobjManagerV1,
    wp_linux_drm_syncobj_surface_v1::WpLinuxDrmSyncobjSurfaceV1,
    wp_linux_drm_syncobj_timeline_v1::WpLinuxDrmSyncobjTimelineV1,
};

use crate::{Globals, Headless, Memory, ShmImage, TestClient};

const SW_SYNC: &str = "/sys/kernel/debug/sync/sw_sync";
const DMA_BUF_SYNC_WRITE: u32 = 2;

#[repr(C)]
struct SwSyncCreateFence {
    value: u32,
    name: [u8; 32],
    fence: i32,
}

#[repr(C)]
struct DmaBufImportSyncFile {
    flags: u32,
    fd: i32,
}

pub struct SwSyncTimeline(File);

impl SwSyncTimeline {
    pub fn new() -> io::Result<Self> {
        File::options()
            .read(true)
            .write(true)
            .open(SW_SYNC)
            .map(Self)
    }

    pub fn fence(&self, point: u32) -> io::Result<OwnedFd> {
        let mut request = SwSyncCreateFence {
            value: point,
            name: [0; 32],
            fence: -1,
        };
        unsafe {
            ioctl(
                &self.0,
                Updater::<{ opcode::read_write::<SwSyncCreateFence>(b'W', 0) }, _>::new(
                    &mut request,
                ),
            )
        }?;
        Ok(unsafe { OwnedFd::from_raw_fd(request.fence) })
    }

    pub fn advance(&self, by: u32) -> io::Result<()> {
        unsafe {
            ioctl(
                &self.0,
                Setter::<{ opcode::write::<u32>(b'W', 1) }, u32>::new(by),
            )
        }?;
        Ok(())
    }
}

pub(crate) fn add_write_fence(dmabuf: &OwnedFd, fence: BorrowedFd<'_>) -> io::Result<()> {
    let request = DmaBufImportSyncFile {
        flags: DMA_BUF_SYNC_WRITE,
        fd: fence.as_raw_fd(),
    };
    unsafe {
        ioctl(
            dmabuf,
            Setter::<{ opcode::write::<DmaBufImportSyncFile>(b'b', 3) }, _>::new(request),
        )
    }?;
    Ok(())
}

impl TestClient {
    pub fn offers_explicit_sync(&self) -> bool {
        self.globals
            .advertised
            .iter()
            .any(|(_, interface)| interface == "wp_linux_drm_syncobj_manager_v1")
    }

    pub fn show_synced_window(
        &mut self,
        server: &mut Headless,
        image: &ShmImage,
        timeline: BorrowedFd<'_>,
        points: (u64, u64),
    ) {
        let manager: WpLinuxDrmSyncobjManagerV1 = self.bind("wp_linux_drm_syncobj_manager_v1", 1);
        let qh = self.queue.handle();
        let timeline = manager.import_timeline(timeline.as_fd(), &qh, ());
        self.map_window(
            server,
            image,
            Memory::Udmabuf,
            |client, surface: &WlSurface, _| {
                let synced = manager.get_surface(surface, &client.queue.handle(), ());
                let split = |point: u64| ((point >> 32) as u32, point as u32);
                let (acquire_hi, acquire_lo) = split(points.0);
                let (release_hi, release_lo) = split(points.1);
                synced.set_acquire_point(&timeline, acquire_hi, acquire_lo);
                synced.set_release_point(&timeline, release_hi, release_lo);
            },
        );
    }
}

wayland_client::delegate_noop!(Globals: ignore WpLinuxDrmSyncobjManagerV1);
wayland_client::delegate_noop!(Globals: ignore WpLinuxDrmSyncobjTimelineV1);
wayland_client::delegate_noop!(Globals: ignore WpLinuxDrmSyncobjSurfaceV1);
