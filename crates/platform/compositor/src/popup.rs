use smithay::desktop::{
    PopupKeyboardGrab, PopupKind, PopupManager, PopupPointerGrab, PopupUngrabStrategy,
    WindowSurfaceType, find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output,
};
use smithay::input::Seat;
use smithay::input::pointer::Focus;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Serial};
use smithay::wayland::compositor::get_parent;
use smithay::wayland::shell::wlr_layer::KeyboardInteractivity;
use smithay::wayland::shell::xdg::{PopupSurface, PositionerState};

use crate::state::State;

impl State {
    pub(crate) fn toplevel_of(&self, surface: &WlSurface) -> Option<WlSurface> {
        let root = self.root_of(surface)?;
        self.window_index(&root).map(|_| root)
    }

    pub(crate) fn focusable_layer_of(&self, surface: &WlSurface) -> Option<WlSurface> {
        let root = self.root_of(surface)?;
        let map = layer_map_for_output(&self.screens[self.layer_screen(&root)?].output);
        let layer = map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)?;

        (layer.cached_state().keyboard_interactivity == KeyboardInteractivity::OnDemand)
            .then_some(root)
    }

    pub(crate) fn root_of(&self, surface: &WlSurface) -> Option<WlSurface> {
        let mut root = surface.clone();
        loop {
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            match self.popups.find_popup(&root) {
                Some(popup) => root = find_popup_root_surface(&popup).ok()?,
                None => return Some(root),
            }
        }
    }

    pub(crate) fn geometry_origin(&self, root: &WlSurface) -> Option<Point<i32, Logical>> {
        if let Some(index) = self.window_index(root) {
            return Some(self.windows[index].location);
        }
        self.layer_origin(root)
    }

    pub(crate) fn popup_surfaces(
        &self,
        root: &WlSurface,
        geometry_origin: Point<i32, Logical>,
    ) -> Vec<(WlSurface, Point<i32, Logical>)> {
        PopupManager::popups_for_surface(root)
            .map(|(popup, offset)| {
                let own = popup.geometry().loc;
                (
                    popup.wl_surface().clone(),
                    (
                        geometry_origin.x + offset.x - own.x,
                        geometry_origin.y + offset.y - own.y,
                    )
                        .into(),
                )
            })
            .collect()
    }

    fn unconstrained(
        &self,
        popup: &PopupSurface,
        positioner: PositionerState,
    ) -> Rectangle<i32, Logical> {
        let kind = PopupKind::Xdg(popup.clone());
        let Some((origin, output)) = find_popup_root_surface(&kind).ok().and_then(|root| {
            let origin = self.geometry_origin(&root)?;
            Some((origin, self.screen_rect(self.screen_of_surface(&root))))
        }) else {
            return positioner.get_geometry();
        };
        let parent = get_popup_toplevel_coords(&kind);
        let target = Rectangle::new(output.location - origin - parent, output.size);

        positioner.get_unconstrained_geometry(target)
    }

    pub(crate) fn new_popup(&mut self, popup: PopupSurface, positioner: PositionerState) {
        let geometry = self.unconstrained(&popup, positioner);
        popup.with_pending_state(|state| {
            state.geometry = geometry;
        });
        let _ = self.popups.track_popup(PopupKind::Xdg(popup));
    }

    pub(crate) fn reposition_popup(
        &mut self,
        popup: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        let geometry = self.unconstrained(&popup, positioner);
        popup.with_pending_state(|state| {
            state.geometry = geometry;
            state.positioner = positioner;
        });
        popup.send_repositioned(token);
    }

    pub(crate) fn commit_popup(&mut self, surface: &WlSurface) -> bool {
        self.popups.commit(surface);
        match self.popups.find_popup(surface) {
            Some(PopupKind::Xdg(popup)) => {
                if !popup.is_initial_configure_sent() {
                    let _ = popup.send_configure();
                }
                true
            }
            Some(PopupKind::InputMethod(candidates)) => {
                self.place_candidates(&candidates);
                true
            }
            None => false,
        }
    }

    pub(crate) fn grab_popup(&mut self, popup: PopupSurface, seat: &WlSeat, serial: Serial) {
        let Some(seat) = Seat::<State>::from_resource(seat) else {
            return;
        };
        let kind = PopupKind::Xdg(popup);
        let Some(root) = find_popup_root_surface(&kind).ok() else {
            return;
        };
        let Ok(mut grab) = self.popups.grab_popup(root, kind, &seat, serial) else {
            return;
        };
        let fallback = grab.previous_serial().unwrap_or(serial);
        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed() && !(keyboard.has_grab(serial) || keyboard.has_grab(fallback))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = seat.get_pointer() {
            let fallback = grab.previous_serial().unwrap_or_else(|| grab.serial());
            if pointer.is_grabbed() && !(pointer.has_grab(serial) || pointer.has_grab(fallback)) {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
        self.popup_grab = Some(grab);
    }

    pub(crate) fn dismisses_popups(&mut self, under: Option<&WlSurface>) -> bool {
        let Some(grab) = &mut self.popup_grab else {
            return false;
        };
        if grab.has_ended() {
            self.popup_grab = None;
            return false;
        }
        let inside = under.is_some_and(|surface| {
            grab.current_grab()
                .is_some_and(|popup| popup.client() == surface.client())
        });
        if inside {
            return false;
        }
        let root = grab.ungrab(PopupUngrabStrategy::All);
        self.popup_grab = None;
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.unset_grab(self, smithay::utils::SERIAL_COUNTER.next_serial(), 0);
        }
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.unset_grab(self);
        }
        let key = root.and_then(|root| self.toplevel_of(&root));
        if key.is_some() {
            self.focus(key);
        }
        true
    }
}
