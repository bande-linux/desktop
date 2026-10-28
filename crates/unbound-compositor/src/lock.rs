use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::utils::{Physical, Point, SERIAL_COUNTER};
use smithay::wayland::session_lock::{
    LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
};

use crate::state::{State, mapped_size};

pub(crate) struct Lock {
    owner: Option<ClientId>,
    surface: Option<LockSurface>,
    previous: Option<WlSurface>,
    confirmation: Option<SessionLocker>,
    blank_latched: bool,
}

impl Lock {
    fn owner_alive(&self, display: &DisplayHandle) -> bool {
        self.owner.as_ref().is_some_and(|owner| {
            display
                .backend_handle()
                .get_client_data(owner.clone())
                .is_ok()
        })
    }
}

impl State {
    pub(crate) fn is_locked(&self) -> bool {
        self.lock.is_some()
    }

    pub(crate) fn lock_needs_locker(&self) -> bool {
        self.lock
            .as_ref()
            .is_some_and(|lock| !lock.owner_alive(&self.display_handle))
    }

    fn lock_surface(&self) -> Option<&WlSurface> {
        self.lock
            .as_ref()?
            .surface
            .as_ref()
            .filter(|surface| surface.alive())
            .map(LockSurface::wl_surface)
    }

    pub(crate) fn locked_stack(&self) -> Vec<(WlSurface, Point<i32, Physical>)> {
        self.lock_surface()
            .filter(|surface| mapped_size(surface).is_some())
            .map(|surface| (surface.clone(), Point::default()))
            .into_iter()
            .collect()
    }

    pub(crate) fn redirect_focus_while_locked(&mut self, surface: &Option<WlSurface>) -> bool {
        let lock_surface = self.lock_surface().cloned();
        let Some(lock) = &mut self.lock else {
            return false;
        };
        if surface.is_some() && *surface == lock_surface {
            return false;
        }
        if surface.is_some() {
            lock.previous = surface.clone();
        }
        true
    }

    pub(crate) fn commit_lock_surface(&mut self, surface: &WlSurface) -> bool {
        if self.lock_surface() != Some(surface) {
            return false;
        }
        if mapped_size(surface).is_some() && self.focused().as_ref() != Some(surface) {
            self.set_keyboard_focus(Some(surface.clone()));
        }
        true
    }

    fn set_keyboard_focus(&mut self, surface: Option<WlSurface>) {
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, surface, SERIAL_COUNTER.next_serial());
        }
        self.sync_activated();
    }

    pub(crate) fn latch_lock(&mut self) {
        if let Some(lock) = &mut self.lock
            && lock.confirmation.is_some()
        {
            lock.blank_latched = true;
        }
    }

    pub(crate) fn confirm_lock(&mut self) {
        if let Some(lock) = &mut self.lock
            && lock.blank_latched
            && let Some(confirmation) = lock.confirmation.take()
        {
            confirmation.lock();
        }
    }

    fn release_grabs(&mut self) {
        self.dismisses_popups(None);
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), 0);
        }
    }
}

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        let owner = confirmation
            .ext_session_lock()
            .client()
            .map(|client| client.id());
        if let Some(lock) = &mut self.lock {
            if lock.owner_alive(&self.display_handle) {
                return;
            }
            lock.owner = owner;
            lock.surface = None;
            lock.confirmation = Some(confirmation);
            lock.blank_latched = false;
            return;
        }
        self.release_grabs();
        let previous = self.key_window();
        self.lock = Some(Lock {
            owner,
            surface: None,
            previous,
            confirmation: Some(confirmation),
            blank_latched: false,
        });
        self.set_keyboard_focus(None);
        self.needs_redraw = true;
    }

    fn unlock(&mut self) {
        let Some(lock) = self.lock.take() else {
            return;
        };
        self.needs_redraw = true;
        let previous = lock.previous.filter(|previous| {
            self.windows
                .iter()
                .any(|window| window.surface() == previous && window.visible())
        });
        if let Some(layer) = &self.exclusive_layer {
            let surface = layer.surface.clone();
            self.set_keyboard_focus(Some(surface));
            self.focus(previous);
        } else {
            self.set_keyboard_focus(None);
            let next = previous.or_else(|| self.frontmost(None));
            self.focus(next);
        }
    }

    fn new_surface(&mut self, surface: LockSurface, _output: WlOutput) {
        let client = surface.wl_surface().client().map(|client| client.id());
        let Some(lock) = &self.lock else {
            return;
        };
        if client.is_none() || client != lock.owner {
            return;
        }
        let size = self.output_size;
        surface.with_pending_state(|state| {
            state.size = Some((size.w.unsigned_abs(), size.h.unsigned_abs()).into());
        });
        surface.send_configure();
        if let Some(lock) = &mut self.lock {
            lock.surface = Some(surface);
        }
    }
}

smithay::delegate_session_lock!(State);
