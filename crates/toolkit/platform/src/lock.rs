use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_manager_v1::ExtSessionLockManagerV1;
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_surface_v1::{
    self, ExtSessionLockSurfaceV1,
};
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_v1::{
    self, ExtSessionLockV1,
};

use crate::window::{State, WindowError, WindowEvent};

const OUTPUT_VARIABLE: &str = "BANDE_OUTPUT";

#[derive(Default)]
pub(crate) struct SessionLock {
    pub(crate) manager: Option<ExtSessionLockManagerV1>,
    pub(crate) output: Option<WlOutput>,
    pub(crate) named: Vec<(WlOutput, String)>,
    pub(crate) wanted_name: Option<String>,
    locked: bool,
    refused: bool,
}

pub(crate) struct NamedOutput;

pub(crate) fn wanted_output() -> Option<String> {
    std::env::var(OUTPUT_VARIABLE)
        .ok()
        .filter(|name| !name.is_empty())
}

impl SessionLock {
    pub(crate) fn wanted(&self) -> Option<WlOutput> {
        let wanted = self.wanted_name.as_ref()?;
        self.named
            .iter()
            .find(|(_, name)| name == wanted)
            .map(|(output, _)| output.clone())
    }
}

impl Dispatch<WlOutput, NamedOutput> for State {
    fn event(
        state: &mut Self,
        output: &WlOutput,
        event: wl_output::Event,
        _: &NamedOutput,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            state.lock.named.retain(|(known, _)| known != output);
            state.lock.named.push((output.clone(), name));
        }
    }
}

pub(crate) struct Locked {
    lock: ExtSessionLockV1,
    surface: ExtSessionLockSurfaceV1,
}

impl Locked {
    pub(crate) fn lock(
        state: &mut State,
        queue: &mut EventQueue<State>,
        surface: &WlSurface,
    ) -> Result<Self, WindowError> {
        let handle = queue.handle();
        let manager = state
            .lock
            .manager
            .clone()
            .ok_or(WindowError::Missing("ext_session_lock_manager_v1"))?;
        let output = state
            .lock
            .wanted()
            .or_else(|| state.lock.output.clone())
            .ok_or(WindowError::Missing("wl_output"))?;

        let lock = manager.lock(&handle, ());
        while !state.lock.locked {
            if state.lock.refused {
                lock.destroy();
                return Err(WindowError::LockRefused);
            }
            queue.blocking_dispatch(state)?;
        }
        let surface = lock.get_lock_surface(surface, &output, &handle, ());
        Ok(Self { lock, surface })
    }

    pub(crate) fn unlock(&self) {
        self.surface.destroy();
        self.lock.unlock_and_destroy();
    }
}

impl Dispatch<ExtSessionLockV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => state.lock.locked = true,
            ext_session_lock_v1::Event::Finished => {
                state.lock.refused = true;
                state.events.push(WindowEvent::CloseRequested);
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
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
            state.configure(width, height);
        }
    }
}

delegate_noop!(State: ignore ExtSessionLockManagerV1);
delegate_noop!(State: ignore WlOutput);
