use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use smithay::wayland::session_lock::{
    LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
};

use crate::state::{State, mapped_size};

pub(crate) struct Lock {
    owner: Option<ClientId>,
    surfaces: Vec<(String, LockSurface)>,
    previous: Option<WlSurface>,
    confirmation: Option<SessionLocker>,
    blank_latched: Vec<String>,
    blank_presented: Vec<String>,
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

    fn lock_surface_on(&self, screen: usize) -> Option<&WlSurface> {
        let name = self.screens[screen].name();
        self.lock
            .as_ref()?
            .surfaces
            .iter()
            .find(|(output, surface)| *output == name && surface.alive())
            .map(|(_, surface)| surface.wl_surface())
    }

    fn lock_surface(&self) -> Option<&WlSurface> {
        let mapped = |surface: &&WlSurface| mapped_size(surface).is_some();
        self.lock_surface_on(0).filter(mapped).or_else(|| {
            (1..self.screens.len()).find_map(|screen| self.lock_surface_on(screen).filter(mapped))
        })
    }

    pub(crate) fn lock_shown(&self) -> bool {
        self.lock_surface().is_some()
    }

    fn is_lock_surface(&self, surface: &WlSurface) -> bool {
        self.lock.as_ref().is_some_and(|lock| {
            lock.surfaces
                .iter()
                .any(|(_, known)| known.wl_surface() == surface)
        })
    }

    pub(crate) fn locked_stack(&self, screen: usize) -> Vec<(WlSurface, Point<i32, Logical>)> {
        self.lock_surface_on(screen)
            .filter(|surface| mapped_size(surface).is_some())
            .map(|surface| (surface.clone(), self.screens[screen].settings.position))
            .into_iter()
            .collect()
    }

    pub(crate) fn redirect_focus_while_locked(&mut self, surface: &Option<WlSurface>) -> bool {
        let is_lock_surface = surface
            .as_ref()
            .is_some_and(|surface| self.is_lock_surface(surface));
        let Some(lock) = &mut self.lock else {
            return false;
        };
        if is_lock_surface {
            return false;
        }
        if surface.is_some() {
            lock.previous = surface.clone();
        }
        true
    }

    pub(crate) fn commit_lock_surface(&mut self, surface: &WlSurface) -> bool {
        if !self.is_lock_surface(surface) {
            return false;
        }
        let focused_lock = self.focused().is_some_and(|focused| {
            self.is_lock_surface(&focused) && mapped_size(&focused).is_some()
        });
        if !focused_lock && self.lock_surface() == Some(surface) {
            self.set_keyboard_focus(Some(surface.clone()));
        }
        true
    }

    pub(crate) fn resize_lock_surfaces(&mut self) {
        let Some(lock) = &self.lock else {
            return;
        };
        for (output, surface) in &lock.surfaces {
            let Some(screen) = self.screen_named(output) else {
                continue;
            };
            let size = self.screen_rect(screen).size;
            surface.with_pending_state(|state| {
                state.size = Some((size.w.unsigned_abs(), size.h.unsigned_abs()).into());
            });
            surface.send_configure();
        }
    }

    fn set_keyboard_focus(&mut self, surface: Option<WlSurface>) {
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, surface, SERIAL_COUNTER.next_serial());
        }
        self.sync_activated();
    }

    pub(crate) fn latch_lock(&mut self, screen: usize) {
        let name = self.screens[screen].name();
        if let Some(lock) = &mut self.lock
            && lock.confirmation.is_some()
            && !lock.blank_latched.contains(&name)
        {
            lock.blank_latched.push(name);
        }
    }

    pub(crate) fn confirm_lock(&mut self, screen: usize) {
        let name = self.screens[screen].name();
        let names: Vec<String> = self.screens.iter().map(|screen| screen.name()).collect();
        let Some(lock) = &mut self.lock else {
            return;
        };
        if lock.blank_latched.contains(&name) && !lock.blank_presented.contains(&name) {
            lock.blank_presented.push(name);
        }
        if names.iter().all(|name| lock.blank_presented.contains(name))
            && let Some(confirmation) = lock.confirmation.take()
        {
            confirmation.lock();
            self.report_lock_state(true);
        }
    }

    fn report_lock_state(&self, locked: bool) {
        if let Some(states) = &self.lock_states {
            let _ = states.send(locked);
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
            lock.surfaces.clear();
            lock.confirmation = Some(confirmation);
            lock.blank_latched.clear();
            lock.blank_presented.clear();
            return;
        }
        self.release_grabs();
        self.close_overview();
        let previous = self.key_window();
        self.lock = Some(Lock {
            owner,
            surfaces: Vec::new(),
            previous,
            confirmation: Some(confirmation),
            blank_latched: Vec::new(),
            blank_presented: Vec::new(),
        });
        self.set_keyboard_focus(None);
        self.needs_redraw = true;
    }

    fn unlock(&mut self) {
        let Some(lock) = self.lock.take() else {
            return;
        };
        self.report_lock_state(false);
        self.needs_redraw = true;
        let previous = lock.previous.filter(|previous| {
            self.windows
                .iter()
                .any(|window| window.surface() == previous && self.present(window))
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

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let client = surface.wl_surface().client().map(|client| client.id());
        let Some(lock) = &self.lock else {
            return;
        };
        if client.is_none() || client != lock.owner {
            return;
        }
        let Some(screen) = self
            .screens
            .iter()
            .position(|screen| screen.output.owns(&output))
        else {
            return;
        };
        let size = self.screen_rect(screen).size;
        surface.with_pending_state(|state| {
            state.size = Some((size.w.unsigned_abs(), size.h.unsigned_abs()).into());
        });
        surface.send_configure();
        let name = self.screens[screen].name();
        if let Some(lock) = &mut self.lock {
            lock.surfaces.retain(|(output, _)| *output != name);
            lock.surfaces.push((name, surface));
        }
    }
}

smithay::delegate_session_lock!(State);
