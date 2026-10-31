use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::desktop::{
    PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy, WindowSurfaceType,
    find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output,
};
use smithay::input::Seat;
use smithay::input::pointer::Focus;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial};
use smithay::wayland::compositor::get_parent;
use smithay::wayland::shell::wlr_layer::KeyboardInteractivity;
use smithay::wayland::shell::xdg::{PopupSurface, PositionerState};

use crate::motion::{BAR_NAMESPACE, DOCK_NAMESPACE};

use crate::state::{Compositor, MENU_BAR_NAMESPACE, Rect, State};

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
            return Some(self.windows[index].content_location());
        }
        self.layer_origin(root)
    }

    fn unconstrained(
        &self,
        popup: &PopupSurface,
        positioner: PositionerState,
    ) -> Rectangle<i32, Logical> {
        let kind = PopupKind::Xdg(popup.clone());
        let Some((root, origin)) = find_popup_root_surface(&kind)
            .ok()
            .and_then(|root| Some((root.clone(), self.geometry_origin(&root)?)))
        else {
            return positioner.get_geometry();
        };
        let parent = origin + get_popup_toplevel_coords(&kind);
        let anchor = positioner.anchor_rect;
        let anchor_centre =
            parent + anchor.loc + Point::from((anchor.size.w / 2, anchor.size.h / 2));
        let screen = self
            .screen_at(anchor_centre.to_f64())
            .unwrap_or_else(|| self.screen_of_surface(&root));
        let area = self.popup_area(&root, screen);
        positioner.get_unconstrained_geometry(Rectangle::new(area.location - parent, area.size))
    }

    fn popup_area(&self, root: &WlSurface, screen: usize) -> Rect {
        let whole_screen = self.is_panel(root)
            || self.window_index(root).is_some_and(|index| {
                self.windows[index].has_state(xdg_toplevel::State::Fullscreen)
            });
        match whole_screen {
            true => self.screen_rect(screen),
            false => self.usable_area(screen),
        }
    }

    pub(crate) fn new_popup(&mut self, popup: PopupSurface, positioner: PositionerState) {
        let geometry = self.unconstrained(&popup, positioner);
        popup.with_pending_state(|state| {
            state.geometry = geometry;
        });
        if let Err(error) = self.popups.track_popup(PopupKind::Xdg(popup)) {
            tracing::warn!("cannot track a popup: {error}");
        }
    }

    pub(crate) fn adopt_layer_popup(&mut self, popup: PopupSurface) {
        let positioner = popup.with_pending_state(|state| state.positioner);
        let geometry = self.unconstrained(&popup, positioner);
        popup.with_pending_state(|state| {
            state.geometry = geometry;
        });
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
                if !popup.is_initial_configure_sent()
                    && let Err(error) = popup.send_configure()
                {
                    tracing::warn!("cannot configure a popup: {error}");
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
        if self
            .menu_root()
            .is_some_and(|menu| menu.client() != popup.wl_surface().client())
        {
            popup.send_popup_done();
            return;
        }
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
            self.key_behind_layer = self.key_window();
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

    pub(crate) fn reachable_beside_menu(&self, surface: &WlSurface) -> bool {
        let Some(popup) = self
            .popup_grab
            .as_ref()
            .and_then(|grab| grab.current_grab())
        else {
            return true;
        };
        if popup.client() != surface.client() {
            return false;
        }
        let mut top = surface.clone();
        while let Some(parent) = get_parent(&top) {
            top = parent;
        }
        self.popups.find_popup(&top).is_some()
            || self
                .root_of(surface)
                .is_some_and(|root| self.is_panel(&root))
    }

    fn is_panel(&self, surface: &WlSurface) -> bool {
        self.screens.iter().any(|screen| {
            layer_map_for_output(&screen.output)
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .is_some_and(|layer| {
                    [MENU_BAR_NAMESPACE, BAR_NAMESPACE, DOCK_NAMESPACE].contains(&layer.namespace())
                })
        })
    }

    pub(crate) fn dismisses_popups(&mut self, under: Option<&WlSurface>) -> bool {
        let Some(grab) = &self.popup_grab else {
            return false;
        };
        if grab.has_ended() {
            self.return_from_menu();
            return false;
        }
        if under.is_some_and(|surface| self.reachable_beside_menu(surface)) {
            return false;
        }
        self.return_from_menu();
        true
    }

    pub(crate) fn return_from_menu(&mut self) {
        let key = self.end_popup_grab().and_then(|root| {
            self.toplevel_of(&root)
                .or_else(|| self.focusable_layer_of(&root))
        });
        if key.is_some() {
            self.focus(key);
        }
    }

    pub(crate) fn menu_root(&self) -> Option<WlSurface> {
        let popup = self
            .popup_grab
            .as_ref()
            .filter(|grab| !grab.has_ended())?
            .current_grab()?;
        self.root_of(&popup)
    }

    pub(crate) fn close_menus(&mut self) {
        self.dismiss_window_menu();
        if self.popup_grab.is_some() {
            self.end_popup_grab();
            self.refocus_pointer();
        }
    }

    pub(crate) fn end_popup_grab(&mut self) -> Option<WlSurface> {
        let mut grab = self.popup_grab.take()?;
        let serials = [
            grab.serial(),
            grab.previous_serial().unwrap_or(grab.serial()),
        ];
        let root = grab.ungrab(PopupUngrabStrategy::All);
        if let Some(pointer) = self.seat.get_pointer()
            && serials.iter().any(|serial| pointer.has_grab(*serial))
        {
            pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), 0);
        }
        if let Some(keyboard) = self.seat.get_keyboard()
            && serials.iter().any(|serial| keyboard.has_grab(*serial))
        {
            keyboard.unset_grab(self);
        }
        root
    }
}

impl Compositor {
    pub fn popup_rects(&self) -> Vec<Rect> {
        (0..self.state.screens.len())
            .flat_map(|screen| self.state.stack(screen))
            .filter_map(|(surface, origin)| {
                let geometry = self.state.popups.find_popup(&surface)?.geometry();
                let drawn = with_renderer_surface_state(&surface, |state| {
                    state.buffer().and(state.surface_size())
                })??;
                Some(Rect {
                    location: origin + geometry.loc,
                    size: match geometry.size.is_empty() {
                        true => drawn,
                        false => geometry.size,
                    },
                })
            })
            .collect()
    }

    pub fn pointer_grabbed(&self) -> bool {
        self.state.popup_grab.is_some()
            || self
                .state
                .seat
                .get_pointer()
                .is_some_and(|pointer| pointer.is_grabbed())
    }

    pub fn keyboard_grabbed(&self) -> bool {
        self.state
            .seat
            .get_keyboard()
            .is_some_and(|keyboard| keyboard.is_grabbed())
    }
}
