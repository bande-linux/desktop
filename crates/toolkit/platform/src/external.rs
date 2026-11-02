use std::cell::RefCell;
use std::io;
use std::os::fd::BorrowedFd;
use std::rc::{Rc, Weak};

use bande_graphics::{Rect, Region, SurfaceId};
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_output::Transform;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_subcompositor::WlSubcompositor;
use wayland_client::protocol::wl_subsurface::WlSubsurface;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::{
    Flags, ZwpLinuxBufferParamsV1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use crate::window::{State, Window, WindowError};

const ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
const XRGB8888: u32 = u32::from_le_bytes(*b"XR24");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShmLayout {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub offset: u32,
    pub fourcc: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct DmabufPlane<'a> {
    pub fd: BorrowedFd<'a>,
    pub offset: u32,
    pub stride: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Dmabuf<'a> {
    pub planes: &'a [DmabufPlane<'a>],
    pub width: u32,
    pub height: u32,
    pub fourcc: u32,
    pub modifier: u64,
    pub flipped: bool,
}

pub struct ExternalSurface {
    shared: Rc<RefCell<External>>,
}

pub(crate) struct External {
    id: SurfaceId,
    surface: WlSurface,
    subsurface: WlSubsurface,
    viewport: WpViewport,
    shm: WlShm,
    dmabuf: Option<ZwpLinuxDmabufV1>,
    handle: QueueHandle<State>,
    connection: Connection,
    buffer: Option<WlBuffer>,
    flipped: bool,
    position: Option<(i32, i32)>,
    destination: Option<(i32, i32)>,
    below: Option<bool>,
    synced: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExternalBuffer;

fn shm_format(fourcc: u32) -> wl_shm::Format {
    match fourcc {
        ARGB8888 => wl_shm::Format::Argb8888,
        XRGB8888 => wl_shm::Format::Xrgb8888,
        other => wl_shm::Format::try_from(other).unwrap_or(wl_shm::Format::Xrgb8888),
    }
}

fn whole(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

impl Window {
    pub fn create_external_surface(&mut self) -> Result<ExternalSurface, WindowError> {
        let state = &self.state;
        let compositor = state
            .compositor
            .as_ref()
            .ok_or(WindowError::Missing("wl_compositor"))?;
        let subcompositor: &WlSubcompositor = state
            .subcompositor
            .as_ref()
            .ok_or(WindowError::Missing("wl_subcompositor"))?;
        let viewporter = state
            .scaling
            .viewporter
            .as_ref()
            .ok_or(WindowError::Missing("wp_viewporter"))?;
        let shm = state.shm.clone().ok_or(WindowError::Missing("wl_shm"))?;
        let handle = self.queue.handle();
        let surface = compositor.create_surface(&handle, ());
        let subsurface = subcompositor.get_subsurface(&surface, &self.surface, &handle, ());
        subsurface.set_desync();
        let viewport = viewporter.get_viewport(&surface, &handle, ());
        let nowhere = compositor.create_region(&handle, ());
        surface.set_input_region(Some(&nowhere));
        nowhere.destroy();
        let shared = Rc::new(RefCell::new(External {
            id: SurfaceId::next(),
            surface,
            subsurface,
            viewport,
            shm,
            dmabuf: state.dmabuf.clone(),
            handle,
            connection: self.connection.clone(),
            buffer: None,
            flipped: false,
            position: None,
            destination: None,
            below: None,
            synced: false,
        }));
        self.externals.push(Rc::downgrade(&shared));
        self.stage();
        Ok(ExternalSurface { shared })
    }

    pub(crate) fn place_externals(&mut self, placed: &[(SurfaceId, Rect)], below: bool) {
        self.externals
            .retain(|external| external.strong_count() > 0);
        for external in self.externals.iter().filter_map(Weak::upgrade) {
            let mut external = external.borrow_mut();
            let area = placed
                .iter()
                .find(|(id, _)| *id == external.id)
                .map(|(_, area)| *area);
            external.place(area, below, &self.surface);
        }
    }

    pub(crate) fn settle_externals(&mut self) {
        for external in self.externals.iter().filter_map(Weak::upgrade) {
            let mut external = external.borrow_mut();
            if std::mem::take(&mut external.synced) {
                external.subsurface.set_desync();
            }
        }
    }
}

impl External {
    fn place(&mut self, area: Option<Rect>, below: bool, parent: &WlSurface) {
        let Some(area) = area else {
            if self.destination.take().is_some() {
                self.subsurface.set_sync();
                self.synced = true;
                self.surface.attach(None, 0, 0);
                self.surface.commit();
            }
            self.position = None;
            return;
        };
        if self.below != Some(below) {
            match below {
                true => self.subsurface.place_below(parent),
                false => self.subsurface.place_above(parent),
            }
            self.below = Some(below);
        }
        let position = (area.x.round() as i32, area.y.round() as i32);
        if self.position != Some(position) {
            self.subsurface.set_position(position.0, position.1);
            self.position = Some(position);
        }
        let destination = (
            (area.width.round() as i32).max(1),
            (area.height.round() as i32).max(1),
        );
        if self.destination == Some(destination) {
            return;
        }
        let mapping = self.destination.is_none();
        self.destination = Some(destination);
        self.subsurface.set_sync();
        self.synced = true;
        self.viewport.set_destination(destination.0, destination.1);
        if mapping && let Some(buffer) = &self.buffer {
            self.surface.attach(Some(buffer), 0, 0);
            self.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        }
        self.surface.commit();
    }

    fn show(
        &mut self,
        buffer: WlBuffer,
        size: (u32, u32),
        flipped: bool,
    ) -> Result<(), WindowError> {
        if self.flipped != flipped {
            self.surface.set_buffer_transform(match flipped {
                true => Transform::Flipped180,
                false => Transform::Normal,
            });
            self.flipped = flipped;
        }
        if self.destination.is_some() {
            self.surface.attach(Some(&buffer), 0, 0);
            self.surface
                .damage_buffer(0, 0, whole(size.0), whole(size.1));
            self.surface.commit();
        }
        if let Some(previous) = self.buffer.replace(buffer) {
            previous.destroy();
        }
        self.flush()
    }

    fn flush(&self) -> Result<(), WindowError> {
        match self.connection.flush() {
            Ok(()) => Ok(()),
            Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

impl ExternalSurface {
    pub fn id(&self) -> SurfaceId {
        self.shared.borrow().id
    }

    pub fn show_shm(&mut self, fd: BorrowedFd<'_>, layout: ShmLayout) -> Result<(), WindowError> {
        let mut external = self.shared.borrow_mut();
        let length = u64::from(layout.offset) + u64::from(layout.stride) * u64::from(layout.height);
        let length = i32::try_from(length).map_err(|_| {
            WindowError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the shared memory buffer is too large",
            ))
        })?;
        let pool = external.shm.create_pool(fd, length, &external.handle, ());
        let buffer = pool.create_buffer(
            whole(layout.offset),
            whole(layout.width),
            whole(layout.height),
            whole(layout.stride),
            shm_format(layout.fourcc),
            &external.handle,
            ExternalBuffer,
        );
        pool.destroy();
        external.show(buffer, (layout.width, layout.height), false)
    }

    pub fn show_dmabuf(&mut self, dmabuf: &Dmabuf<'_>) -> Result<(), WindowError> {
        let mut external = self.shared.borrow_mut();
        let manager = external
            .dmabuf
            .as_ref()
            .ok_or(WindowError::Missing("zwp_linux_dmabuf_v1"))?;
        let params = manager.create_params(&external.handle, ());
        for (index, plane) in dmabuf.planes.iter().enumerate() {
            params.add(
                plane.fd,
                u32::try_from(index).unwrap_or(u32::MAX),
                plane.offset,
                plane.stride,
                (dmabuf.modifier >> 32) as u32,
                dmabuf.modifier as u32,
            );
        }
        let buffer = params.create_immed(
            whole(dmabuf.width),
            whole(dmabuf.height),
            dmabuf.fourcc,
            Flags::empty(),
            &external.handle,
            ExternalBuffer,
        );
        params.destroy();
        external.show(buffer, (dmabuf.width, dmabuf.height), dmabuf.flipped)
    }

    pub fn damage(&mut self, regions: &[Region]) -> Result<(), WindowError> {
        let external = self.shared.borrow();
        let (Some(_), Some(buffer)) = (external.destination, &external.buffer) else {
            return Ok(());
        };
        external.surface.attach(Some(buffer), 0, 0);
        for region in regions {
            external.surface.damage_buffer(
                whole(region.x),
                whole(region.y),
                whole(region.width),
                whole(region.height),
            );
        }
        external.surface.commit();
        external.flush()
    }
}

impl Drop for External {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            buffer.destroy();
        }
        self.viewport.destroy();
        self.subsurface.destroy();
        self.surface.destroy();
        let _ = self.flush();
    }
}

impl Dispatch<WlBuffer, ExternalBuffer> for State {
    fn event(
        _: &mut Self,
        _: &WlBuffer,
        _: wayland_client::protocol::wl_buffer::Event,
        _: &ExternalBuffer,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore WlSubcompositor);
delegate_noop!(State: ignore WlSubsurface);
delegate_noop!(State: ignore ZwpLinuxDmabufV1);
delegate_noop!(State: ignore ZwpLinuxBufferParamsV1);
