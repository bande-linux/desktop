use std::cell::RefCell;
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::fs::FileExt;
use std::rc::{Rc, Weak};
use std::sync::Mutex;

use bande_graphics::{Rect, Region, SurfaceId};
use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_output::Transform;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_subcompositor::WlSubcompositor;
use wayland_client::protocol::wl_subsurface::WlSubsurface;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1;
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1;
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::{
    Flags, ZwpLinuxBufferParamsV1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::{
    self, ZwpLinuxDmabufV1,
};
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

struct HeldBuffer {
    buffer: WlBuffer,
    size: (u32, u32),
    flipped: bool,
}

impl HeldBuffer {
    fn released(&self) {
        if let Some(data) = self.buffer.data::<ExternalBuffer>() {
            data.released();
        }
    }
}

impl Drop for HeldBuffer {
    fn drop(&mut self) {
        self.buffer.destroy();
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CursorImage<'a> {
    pub width: u32,
    pub height: u32,
    pub hotspot: (u32, u32),
    pub pixels: &'a [u8],
}

pub(crate) struct ImageCursor {
    pub(crate) surface: WlSurface,
    viewport: WpViewport,
    buffer: Option<WlBuffer>,
    pub(crate) hotspot: (i32, i32),
}

impl Drop for ImageCursor {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            buffer.destroy();
        }
        self.viewport.destroy();
        self.surface.destroy();
    }
}

#[derive(Clone)]
pub struct SurfaceBuffer {
    held: Rc<HeldBuffer>,
}

struct Current {
    held: Rc<HeldBuffer>,
    attached: bool,
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
    current: Option<Current>,
    idle_inhibit: Option<ZwpIdleInhibitManagerV1>,
    idle_inhibitor: Option<ZwpIdleInhibitorV1>,
    flipped: bool,
    position: Option<(i32, i32)>,
    destination: Option<(i32, i32)>,
    below: Option<bool>,
    synced: bool,
}

type ReleaseCallback = Box<dyn FnMut() + Send>;

#[derive(Default)]
pub(crate) struct ExternalBuffer {
    on_release: Mutex<Option<ReleaseCallback>>,
}

impl ExternalBuffer {
    fn new(on_release: impl FnMut() + Send + 'static) -> Self {
        Self {
            on_release: Mutex::new(Some(Box::new(on_release))),
        }
    }

    fn released(&self) {
        if let Ok(mut callback) = self.on_release.lock()
            && let Some(callback) = callback.as_mut()
        {
            callback();
        }
    }
}

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
            current: None,
            idle_inhibit: state.idle_inhibit_manager.clone(),
            idle_inhibitor: None,
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

    pub fn set_cursor_image(&mut self, image: CursorImage<'_>) -> Result<(), WindowError> {
        let stride = u64::from(image.width) * 4;
        let length = stride * u64::from(image.height);
        if image.width == 0 || image.height == 0 || image.pixels.len() as u64 != length {
            return Err(WindowError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the cursor image has the wrong size",
            )));
        }
        let handle = self.queue.handle();
        let state = &mut self.state;
        let shm = state.shm.as_ref().ok_or(WindowError::Missing("wl_shm"))?;
        let memory =
            File::from(memfd_create("bande-cursor", MemfdFlags::CLOEXEC).map_err(io::Error::from)?);
        memory.write_all_at(image.pixels, 0)?;
        let pool = shm.create_pool(
            memory.as_fd(),
            whole(image.pixels.len() as u32),
            &handle,
            (),
        );
        let buffer = pool.create_buffer(
            0,
            whole(image.width),
            whole(image.height),
            whole(image.width * 4),
            wl_shm::Format::Argb8888,
            &handle,
            ExternalBuffer::default(),
        );
        pool.destroy();
        let scale = state.scaling.scale();
        let logical = |pixels: u32| ((f64::from(pixels) / scale).round() as i32).max(1);
        let mut cursor = match state.input.take_cursor_image() {
            Some(cursor) => cursor,
            None => {
                let compositor = state
                    .compositor
                    .as_ref()
                    .ok_or(WindowError::Missing("wl_compositor"))?;
                let viewporter = state
                    .scaling
                    .viewporter
                    .as_ref()
                    .ok_or(WindowError::Missing("wp_viewporter"))?;
                let surface = compositor.create_surface(&handle, ());
                let viewport = viewporter.get_viewport(&surface, &handle, ());
                ImageCursor {
                    surface,
                    viewport,
                    buffer: None,
                    hotspot: (0, 0),
                }
            }
        };
        cursor
            .viewport
            .set_destination(logical(image.width), logical(image.height));
        cursor.surface.attach(Some(&buffer), 0, 0);
        cursor.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        cursor.surface.commit();
        if let Some(previous) = cursor.buffer.replace(buffer) {
            previous.destroy();
        }
        cursor.hotspot = (
            (f64::from(image.hotspot.0) / scale).round() as i32,
            (f64::from(image.hotspot.1) / scale).round() as i32,
        );
        state.input.show_cursor_image(cursor);
        self.flush()
    }

    pub fn supports_dmabuf(&self, fourcc: u32, modifier: u64) -> bool {
        self.state.dmabuf_formats.contains(&(fourcc, modifier))
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
        if mapping && let Some(current) = &mut self.current {
            self.surface.attach(Some(&current.held.buffer), 0, 0);
            self.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
            current.attached = true;
        }
        self.surface.commit();
    }

    fn present(&mut self, held: Rc<HeldBuffer>) -> Result<(), WindowError> {
        if self.flipped != held.flipped {
            self.surface.set_buffer_transform(match held.flipped {
                true => Transform::Flipped180,
                false => Transform::Normal,
            });
            self.flipped = held.flipped;
        }
        let attached = self.destination.is_some();
        if attached {
            self.surface.attach(Some(&held.buffer), 0, 0);
            self.surface
                .damage_buffer(0, 0, whole(held.size.0), whole(held.size.1));
            self.surface.commit();
        }
        let replaced = self.current.replace(Current { held, attached });
        if let Some(replaced) = replaced.filter(|replaced| !replaced.attached) {
            replaced.held.released();
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
        let buffer = self.create_shm(fd, layout, ExternalBuffer::default())?;
        self.present(&buffer)
    }

    pub fn show_dmabuf(&mut self, dmabuf: &Dmabuf<'_>) -> Result<(), WindowError> {
        let buffer = self.create_dmabuf(dmabuf, ExternalBuffer::default())?;
        self.present(&buffer)
    }

    pub fn shm_buffer(
        &self,
        fd: BorrowedFd<'_>,
        layout: ShmLayout,
        on_release: impl FnMut() + Send + 'static,
    ) -> Result<SurfaceBuffer, WindowError> {
        self.create_shm(fd, layout, ExternalBuffer::new(on_release))
    }

    pub fn dmabuf_buffer(
        &self,
        dmabuf: &Dmabuf<'_>,
        on_release: impl FnMut() + Send + 'static,
    ) -> Result<SurfaceBuffer, WindowError> {
        self.create_dmabuf(dmabuf, ExternalBuffer::new(on_release))
    }

    pub fn present(&mut self, buffer: &SurfaceBuffer) -> Result<(), WindowError> {
        self.shared.borrow_mut().present(Rc::clone(&buffer.held))
    }

    fn create_shm(
        &self,
        fd: BorrowedFd<'_>,
        layout: ShmLayout,
        data: ExternalBuffer,
    ) -> Result<SurfaceBuffer, WindowError> {
        let external = self.shared.borrow();
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
            data,
        );
        pool.destroy();
        Ok(SurfaceBuffer {
            held: Rc::new(HeldBuffer {
                buffer,
                size: (layout.width, layout.height),
                flipped: false,
            }),
        })
    }

    fn create_dmabuf(
        &self,
        dmabuf: &Dmabuf<'_>,
        data: ExternalBuffer,
    ) -> Result<SurfaceBuffer, WindowError> {
        let external = self.shared.borrow();
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
            data,
        );
        params.destroy();
        Ok(SurfaceBuffer {
            held: Rc::new(HeldBuffer {
                buffer,
                size: (dmabuf.width, dmabuf.height),
                flipped: dmabuf.flipped,
            }),
        })
    }

    pub fn set_idle_inhibited(&mut self, inhibited: bool) {
        let external = &mut *self.shared.borrow_mut();
        if !inhibited {
            if let Some(inhibitor) = external.idle_inhibitor.take() {
                inhibitor.destroy();
            }
        } else if external.idle_inhibitor.is_none()
            && let Some(manager) = &external.idle_inhibit
        {
            external.idle_inhibitor =
                Some(manager.create_inhibitor(&external.surface, &external.handle, ()));
        }
    }

    pub fn damage(&mut self, regions: &[Region]) -> Result<(), WindowError> {
        let external = self.shared.borrow();
        let (Some(_), Some(current)) = (external.destination, &external.current) else {
            return Ok(());
        };
        external.surface.attach(Some(&current.held.buffer), 0, 0);
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
        self.current = None;
        if let Some(inhibitor) = self.idle_inhibitor.take() {
            inhibitor.destroy();
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
        event: wayland_client::protocol::wl_buffer::Event,
        data: &ExternalBuffer,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wayland_client::protocol::wl_buffer::Event::Release) {
            data.released();
        }
    }
}

impl Dispatch<ZwpLinuxDmabufV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpLinuxDmabufV1,
        event: zwp_linux_dmabuf_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_linux_dmabuf_v1::Event::Modifier {
            format,
            modifier_hi,
            modifier_lo,
        } = event
        {
            let modifier = u64::from(modifier_hi) << 32 | u64::from(modifier_lo);
            state.dmabuf_formats.push((format, modifier));
        }
    }
}

delegate_noop!(State: ignore WlSubcompositor);
delegate_noop!(State: ignore WlSubsurface);
delegate_noop!(State: ignore ZwpLinuxBufferParamsV1);
