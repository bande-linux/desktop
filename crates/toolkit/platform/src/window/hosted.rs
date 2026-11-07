use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bande_graphics::Rect;
use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_shm;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::shell::client::xdg_popup::{self, XdgPopup};
use wayland_protocols::xdg::shell::client::xdg_positioner::{
    Anchor, ConstraintAdjustment, Gravity, XdgPositioner,
};
use wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface;
use wayland_protocols::xdg::shell::client::xdg_wm_base::XdgWmBase;

use super::{Role, ShellSurface, State, Window, WindowError, WindowEvent};

const REPOSITION_SINCE: u32 = 3;

#[derive(Debug, Clone)]
pub struct HostedPopup {
    pub(super) parent: XdgSurface,
    pub(super) area: Rect,
    pub(super) grab: bool,
    pub(super) serial: Option<u32>,
}

impl PartialEq for HostedPopup {
    fn eq(&self, other: &Self) -> bool {
        self.parent == other.parent
            && self.area == other.area
            && self.grab == other.grab
            && self.serial == other.serial
    }
}

impl Eq for HostedPopup {}

impl HostedPopup {
    pub(super) fn positioner(
        &self,
        wm_base: &XdgWmBase,
        handle: &QueueHandle<State>,
        size: (u32, u32),
    ) -> XdgPositioner {
        positioner(wm_base, handle, self.area, size)
    }
}

fn positioner(
    wm_base: &XdgWmBase,
    handle: &QueueHandle<State>,
    area: Rect,
    size: (u32, u32),
) -> XdgPositioner {
    let whole = |value: f32| value.round() as i32;
    let positioner = wm_base.create_positioner(handle, ());
    positioner.set_size(
        i32::try_from(size.0.max(1)).unwrap_or(i32::MAX),
        i32::try_from(size.1.max(1)).unwrap_or(i32::MAX),
    );
    positioner.set_anchor_rect(whole(area.x), whole(area.y), 1, 1);
    positioner.set_anchor(Anchor::TopLeft);
    positioner.set_gravity(Gravity::BottomRight);
    positioner
        .set_constraint_adjustment(ConstraintAdjustment::SlideX | ConstraintAdjustment::SlideY);
    positioner
}

impl Dispatch<XdgPopup, ()> for State {
    fn event(
        state: &mut Self,
        _: &XdgPopup,
        event: xdg_popup::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_popup::Event::Configure {
                x,
                y,
                width,
                height,
            } => {
                if width > 0 && height > 0 {
                    state.suggested_size = Some((width.unsigned_abs(), height.unsigned_abs()));
                }
                state.events.push(WindowEvent::HostedMoved((x, y)));
            }
            xdg_popup::Event::PopupDone => state.events.push(WindowEvent::PopupDismissed),
            _ => {}
        }
    }
}

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
    pub fn hosted_popup(&self, area: Rect, grab: bool) -> Result<Role, WindowError> {
        let parent = match &self.role {
            ShellSurface::Toplevel(xdg_surface, _) | ShellSurface::Popup(xdg_surface, _) => {
                xdg_surface.clone()
            }
            ShellSurface::Layer(_) | ShellSurface::Lock(_) => {
                return Err(WindowError::Missing("a surface that takes popups"));
            }
        };
        Ok(Role::HostedPopup(HostedPopup {
            parent,
            area,
            grab,
            serial: self.state.input.serial,
        }))
    }

    pub fn move_hosted_popup(&mut self, area: Rect) -> Result<(), WindowError> {
        let size = (
            area.width.round().max(1.0) as u32,
            area.height.round().max(1.0) as u32,
        );
        let ShellSurface::Popup(_, popup) = &self.role else {
            return Ok(());
        };
        if popup.version() < REPOSITION_SINCE {
            return Ok(());
        }
        let wm_base = self
            .state
            .wm_base
            .clone()
            .ok_or(WindowError::Missing("xdg_wm_base"))?;
        let positioner = positioner(&wm_base, &self.queue.handle(), area, size);
        popup.reposition(&positioner, 0);
        positioner.destroy();
        self.flush()
    }

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
