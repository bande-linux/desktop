use std::sync::atomic::Ordering;

use super::{ClientState, ExclusiveLayer, State};

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::shell::wlr_layer::Layer;

use crate::shell;

impl State {
    pub(crate) fn focused(&self) -> Option<WlSurface> {
        self.seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
    }

    pub(crate) fn focus(&mut self, surface: Option<WlSurface>) {
        if self.redirect_focus_while_locked(&surface) {
            return;
        }
        let surface = surface.map(|surface| self.modal_blocker(&surface).unwrap_or(surface));
        if self.popup_grab.is_some() && surface != self.menu_root() {
            self.close_menus();
        }
        self.dismiss_window_menu_unless_on(surface.as_ref());
        let onto_layer = surface
            .as_ref()
            .is_some_and(|surface| self.toplevel_of(surface).is_none());
        self.key_behind_layer = self.key_window().filter(|_| onto_layer);
        if !onto_layer {
            self.windowless_app = None;
        }
        self.typed_at = None;
        if let Some(surface) = &surface
            && self.window_index(surface).is_some()
        {
            self.hide_desktop();
            self.show_workspace_of(surface);
        }
        if let Some(exclusive) = &mut self.exclusive_layer
            && surface.as_ref() != Some(&exclusive.surface)
        {
            exclusive.previous = surface.clone();
            let holder = exclusive.surface.clone();
            if let Some(index) = surface
                .as_ref()
                .and_then(|surface| self.window_index(surface))
            {
                self.raise(index);
            }
            if self.popup_grab.is_none()
                && let Some(keyboard) = self.seat.get_keyboard()
                && keyboard.current_focus().as_ref() != Some(&holder)
            {
                keyboard.set_focus(self, Some(holder), SERIAL_COUNTER.next_serial());
            }
            self.sync_activated();
            return;
        }
        if let Some(index) = surface
            .as_ref()
            .and_then(|surface| self.window_index(surface))
        {
            self.raise(index);
        }
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, surface, SERIAL_COUNTER.next_serial());
        }
        self.sync_activated();
    }

    pub(crate) fn frontmost(&self, owner: Option<&Client>) -> Option<WlSurface> {
        self.windows
            .iter()
            .rev()
            .filter(|window| self.present(window))
            .map(|window| window.toplevel.wl_surface())
            .find(|surface| {
                surface
                    .client()
                    .is_some_and(|client| owner.is_none_or(|owner| &client == owner))
            })
            .cloned()
    }

    pub(crate) fn frontmost_on(&self, screen: usize) -> Option<WlSurface> {
        self.windows
            .iter()
            .rev()
            .filter(|window| self.present(window) && self.screen_of(window) == screen)
            .map(|window| window.toplevel.wl_surface().clone())
            .next()
    }

    pub(crate) fn focus_behind_layer(&mut self) {
        let next = self
            .key_behind_layer
            .clone()
            .filter(|key| self.window_index(key).is_some())
            .or_else(|| self.frontmost(None));
        self.focus(next);
    }

    pub(crate) fn refocus_after_losing(&mut self, lost: &WlSurface) {
        let Some(focused) = self.focused() else {
            return;
        };
        if &focused != lost && focused.is_alive() && self.root_of(&focused).as_ref() != Some(lost) {
            return;
        }
        let owner = lost.client();
        let next = match &owner {
            Some(owner) => self.frontmost(Some(owner)),
            None => self.frontmost(None),
        };
        let windowless = next.is_none();
        let app_id = shell::toplevel_names(lost).1;
        self.focus(next);
        if windowless {
            self.windowless_app = owner.map(|owner| (owner, app_id));
            let key = self.key_window();
            self.announce_active_app(key.as_ref());
        }
    }

    pub(super) fn activate_next_app_if_the_windowless_one_quit(&mut self) {
        let quit = self.windowless_app.as_ref().is_some_and(|(app, _)| {
            app.get_data::<ClientState>()
                .is_some_and(|state| state.departed.load(Ordering::Relaxed))
        });
        if quit {
            self.windowless_app = None;
            if self.focused().is_none() {
                let next = self.frontmost(None);
                self.focus(next);
            }
        }
    }

    fn takes_focus_when_mapped(&self, surface: &WlSurface) -> bool {
        let Some(focused) = self.focused() else {
            return true;
        };
        let owner = surface.client();
        if focused.client() == owner {
            return true;
        }
        if self.would_interrupt_typing(surface, &focused) {
            return false;
        }
        !self.windows.iter().any(|window| {
            window.size.is_some()
                && window.toplevel.wl_surface() != surface
                && window.toplevel.wl_surface().client() == owner
        })
    }

    fn place_behind_frontmost(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        if self.stack_above_parent(index) {
            return;
        }
        let window = self.windows.remove(index);
        let behind = self.windows.len().saturating_sub(1);
        self.windows.insert(behind, window);
    }

    pub(super) fn follow_exclusive_layer(&mut self, surface: &WlSurface, exclusive: Option<Layer>) {
        let known = self
            .exclusive_candidates
            .iter()
            .position(|(candidate, _)| candidate == surface);
        match (exclusive, known) {
            (Some(layer), Some(index)) => self.exclusive_candidates[index].1 = layer,
            (Some(layer), None) => self.exclusive_candidates.push((surface.clone(), layer)),
            (None, Some(index)) => {
                self.exclusive_candidates.remove(index);
            }
            (None, None) => return,
        }
        let holder = self
            .exclusive_candidates
            .iter()
            .max_by_key(|(_, layer)| *layer == Layer::Overlay)
            .map(|(candidate, _)| candidate.clone());
        if holder.as_ref() == self.exclusive_layer.as_ref().map(|layer| &layer.surface) {
            return;
        }
        let Some(holder) = holder else {
            self.release_exclusive_layer();
            return;
        };
        let previous = match self.exclusive_layer.take() {
            Some(layer) => layer.previous,
            None => self.focused(),
        };
        self.focus(Some(holder.clone()));
        self.exclusive_layer = Some(ExclusiveLayer {
            surface: holder,
            previous,
        });
        self.sync_activated();
    }

    fn release_exclusive_layer(&mut self) {
        let Some(layer) = self.exclusive_layer.take() else {
            return;
        };
        if self.menu_root().is_some() {
            return;
        }
        let previous = layer.previous.filter(|previous| {
            self.windows
                .iter()
                .any(|window| window.surface() == previous && self.present(window))
        });
        let next = previous.or_else(|| self.frontmost(None));
        self.focus(next);
    }

    pub(crate) fn after_window_commit(
        &mut self,
        surface: &WlSurface,
        was_mapped: bool,
        mapped: bool,
    ) {
        match (was_mapped, mapped) {
            (false, true)
                if self.pending_activation.as_ref() == Some(surface)
                    || self.takes_focus_when_mapped(surface) =>
            {
                self.pending_activation = None;
                self.focus(Some(surface.clone()));
                self.sync_surface_outputs();
            }
            (false, true) => {
                if self
                    .focused()
                    .is_some_and(|focused| self.would_interrupt_typing(surface, &focused))
                {
                    self.request_attention(shell::toplevel_names(surface).1);
                }
                self.place_behind_frontmost(surface);
                self.sync_surface_outputs();
            }
            (true, false) => {
                if let Some(index) = self.window_index(surface) {
                    self.close_window(index);
                }
                self.refocus_after_losing(surface);
                if let Some(index) = self.window_index(surface) {
                    self.window_left_workspace(self.windows[index].workspace);
                }
            }
            _ => {}
        }
    }
}
