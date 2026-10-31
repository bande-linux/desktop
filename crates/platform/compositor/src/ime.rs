use smithay::desktop::{PopupKind, PopupManager};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::input_method::{InputMethodHandler, PopupSurface};
use smithay::{
    delegate_input_method_manager, delegate_text_input_manager, delegate_virtual_keyboard_manager,
};

use crate::state::{State, mapped_size};

fn candidates_location(
    cursor: Rectangle<i32, Logical>,
    candidates: (i32, i32),
    surface_origin: Point<i32, Logical>,
    output: (i32, i32),
) -> Point<i32, Logical> {
    let (width, height) = candidates;
    let below = cursor.loc.y + cursor.size.h;
    let fits_below = surface_origin.y + below + height <= output.1;
    let fits_above = surface_origin.y + cursor.loc.y - height >= 0;
    let y = if fits_below || !fits_above {
        below
    } else {
        cursor.loc.y - height
    };
    let rightmost = output.0 - width - surface_origin.x;
    let x = cursor.loc.x.min(rightmost).max(-surface_origin.x);
    (x, y).into()
}

impl State {
    fn surface_local_geometry(&self, surface: &WlSurface) -> Rectangle<i32, Logical> {
        if let Some(index) = self.window_index(surface) {
            return self.windows[index].local_geometry();
        }
        let size = mapped_size(surface).unwrap_or_default();
        Rectangle::from_size((size.w, size.h).into())
    }

    pub(crate) fn place_candidates(&self, popup: &PopupSurface) {
        let Some(parent) = popup.get_parent() else {
            return;
        };
        let Some(origin) = self.geometry_origin(&parent.surface) else {
            return;
        };
        let size = mapped_size(popup.wl_surface()).unwrap_or_default();
        let screen = self.screen_rect(self.screen_of_surface(&parent.surface));
        let surface_origin = Point::from((origin.x, origin.y)) - parent.location.loc - screen.loc;
        let output = screen.size;
        popup.set_location(candidates_location(
            popup.text_input_rectangle(),
            (size.w, size.h),
            surface_origin,
            (output.w, output.h),
        ));
    }
}

impl InputMethodHandler for State {
    fn new_popup(&mut self, surface: PopupSurface) {
        self.place_candidates(&surface);
        if let Err(error) = self.popups.track_popup(PopupKind::InputMethod(surface)) {
            tracing::warn!("cannot track the input method candidates: {error}");
        }
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone())
            && let Err(error) =
                PopupManager::dismiss_popup(&parent, &PopupKind::InputMethod(surface))
        {
            tracing::warn!("cannot dismiss the input method candidates: {error}");
        }
        self.needs_redraw = true;
    }

    fn popup_repositioned(&mut self, surface: PopupSurface) {
        self.place_candidates(&surface);
        self.needs_redraw = true;
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        self.surface_local_geometry(parent)
    }
}

delegate_input_method_manager!(State);
delegate_text_input_manager!(State);
delegate_virtual_keyboard_manager!(State);
