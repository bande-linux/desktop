use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_shm::Format;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_manager_v1::ExtSessionLockManagerV1;
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_surface_v1::{
    self, ExtSessionLockSurfaceV1,
};
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_v1::ExtSessionLockV1;

use crate::{Compositor, Globals, ShmImage, TestClient};

#[derive(Default)]
pub(crate) struct SessionLock {
    lock: Option<ExtSessionLockV1>,
    surface: Option<WlSurface>,
    size: Option<(u32, u32)>,
}

impl TestClient {
    pub fn lock_session(&mut self, server: &mut Compositor) {
        let manager: ExtSessionLockManagerV1 = self.bind("ext_session_lock_manager_v1", 1);
        self.globals.lock.lock = Some(manager.lock(&self.queue.handle(), ()));
        self.exchange(server);
    }

    pub fn show_lock_surface(&mut self, server: &mut Compositor, color: [u8; 4]) {
        let qh = self.queue.handle();
        let output: WlOutput = self.bind("wl_output", 1);
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let surface = compositor.create_surface(&qh, ());
        let lock = self.globals.lock.lock.as_ref().expect("lock_session first");
        lock.get_lock_surface(&surface, &output, &qh, ());
        self.exchange(server);
        let (width, height) = self
            .globals
            .lock
            .size
            .expect("the lock surface is configured");
        let pixels = color.repeat((width * height) as usize);
        let buffer = self.shm_buffer(&ShmImage {
            width: width as i32,
            height: height as i32,
            stride: (width * 4) as i32,
            format: Format::Argb8888,
            bytes: &pixels,
        });
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, width as i32, height as i32);
        surface.commit();
        self.globals.lock.surface = Some(surface);
        self.exchange(server);
    }

    pub fn lock_surface_has_keyboard(&self) -> bool {
        self.globals.lock.surface.is_some()
            && self.globals.input.keyboard_focus == self.globals.lock.surface
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        surface: &ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            surface.ack_configure(serial);
            globals.lock.size = Some((width, height));
        }
    }
}

delegate_noop!(Globals: ExtSessionLockManagerV1);
delegate_noop!(Globals: ignore ExtSessionLockV1);
delegate_noop!(Globals: ignore WlOutput);
