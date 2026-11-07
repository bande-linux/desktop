use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::Proxy;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_shm;

use super::{ShellSurface, Window, WindowError, WindowEvent};

const BACKDROP_PIXEL_BYTES: i32 = 4;

#[derive(Default)]
pub(super) struct Backdrop {
    buffer: Option<WlBuffer>,
}

impl Backdrop {
    pub(super) fn destroy(mut self) {
        if let Some(buffer) = self.buffer.take() {
            buffer.destroy();
        }
    }
}

impl Window {
    pub fn surface_ptr(&self) -> *mut c_void {
        self.surface.id().as_ptr().cast()
    }

    pub fn set_hosted_backdrop(&mut self, bgra: [u8; 4]) -> Result<(), WindowError> {
        if self.surface_scale.is_none() {
            return Err(WindowError::Missing("wp_viewporter"));
        }
        let shm = self
            .state
            .shm
            .as_ref()
            .ok_or(WindowError::Missing("wl_shm"))?;
        let handle = self.queue.handle();
        let file = File::from(
            memfd_create("bande-backdrop", MemfdFlags::CLOEXEC).map_err(io::Error::from)?,
        );
        file.write_all_at(&bgra, 0)?;
        let pool = shm.create_pool(file.as_fd(), BACKDROP_PIXEL_BYTES, &handle, ());
        let buffer = pool.create_buffer(
            0,
            1,
            1,
            BACKDROP_PIXEL_BYTES,
            wl_shm::Format::Argb8888,
            &handle,
            Arc::new(AtomicBool::new(false)),
        );
        pool.destroy();
        let replaced = match &mut self.hosted {
            Some(backdrop) => backdrop.buffer.replace(buffer),
            None => None,
        };
        if let Some(replaced) = replaced {
            replaced.destroy();
        }
        self.state.configure_unapplied = true;
        self.apply_hosted_configure();
        self.flush()
    }

    pub fn set_hosted_size(&mut self, size: (u32, u32)) -> Result<(), WindowError> {
        self.size = size;
        self.state.suggested_size = None;
        self.state.configure_unapplied = true;
        self.apply_hosted_configure();
        self.flush()
    }

    pub fn set_hosted_size_limits(&mut self, min: (u32, u32), max: (u32, u32)) {
        if let ShellSurface::Toplevel(_, toplevel) = &self.role {
            let clamp = |value: u32| i32::try_from(value).unwrap_or(i32::MAX);
            toplevel.set_min_size(clamp(min.0), clamp(min.1));
            toplevel.set_max_size(clamp(max.0), clamp(max.1));
        }
    }

    pub fn set_full_screen(&mut self, full_screen: bool) -> Result<(), WindowError> {
        if self.toplevel_state().fullscreen != full_screen {
            self.toggle_full_screen()?;
        }
        Ok(())
    }

    pub fn set_key_detail(&mut self, detail: bool) {
        self.state.input.set_key_detail(detail);
    }

    pub(super) fn apply_hosted_configure(&mut self) {
        let Some(backdrop) = &self.hosted else {
            return;
        };
        if !std::mem::take(&mut self.state.configure_unapplied) {
            return;
        }
        let Some(buffer) = &backdrop.buffer else {
            self.state.configure_unapplied = true;
            return;
        };
        let size = self.state.suggested_size.take().unwrap_or(self.size);
        let resized = size != self.size;
        self.size = size;
        if let Some(surface_scale) = &self.surface_scale {
            surface_scale.show_at(size);
        }
        self.surface.attach(Some(buffer), 0, 0);
        self.surface.damage_buffer(0, 0, 1, 1);
        self.surface.commit();
        if resized {
            self.state.events.push(WindowEvent::HostedResized(size));
        }
    }
}
