use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_shm::Format;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_manager_v1::ExtSessionLockManagerV1;
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_surface_v1::{
    self, ExtSessionLockSurfaceV1,
};
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_v1::{
    self, ExtSessionLockV1,
};

use crate::{Globals, Headless, ShmImage, TestClient};

#[derive(Default)]
pub(crate) struct SessionLock {
    lock: Option<ExtSessionLockV1>,
    surface: Option<WlSurface>,
    size: Option<(u32, u32)>,
    locked: bool,
    finished: bool,
}

impl TestClient {
    pub fn can_lock_session(&self) -> bool {
        self.advertises("ext_session_lock_manager_v1")
    }

    pub fn lock_session(&mut self, server: &mut Headless) {
        let manager: ExtSessionLockManagerV1 = self.bind("ext_session_lock_manager_v1", 1);
        self.globals.lock.lock = Some(manager.lock(&self.queue.handle(), ()));
        self.exchange(server);
    }

    pub fn session_locked(&self) -> bool {
        self.globals.lock.locked
    }

    pub fn lock_refused(&self) -> bool {
        self.globals.lock.finished
    }

    pub fn show_lock_surface(&mut self, server: &mut Headless, color: [u8; 4]) -> (u32, u32) {
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
        (width, height)
    }

    pub fn lock_surface_has_keyboard(&self) -> bool {
        self.globals.lock.surface.is_some()
            && self.globals.input.keyboard_surface() == self.globals.lock.surface.as_ref()
    }

    pub fn unlock_session(&mut self, server: &mut Headless) {
        if let Some(lock) = self.globals.lock.lock.take() {
            lock.unlock_and_destroy();
        }
        self.exchange(server);
    }
}

impl Dispatch<ExtSessionLockV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => globals.lock.locked = true,
            ext_session_lock_v1::Event::Finished => globals.lock.finished = true,
            _ => {}
        }
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
delegate_noop!(Globals: ignore WlOutput);
