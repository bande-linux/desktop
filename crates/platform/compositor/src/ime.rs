use smithay::desktop::{PopupKind, PopupManager};
use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::zwp_text_input_manager_v3::ZwpTextInputManagerV3;
use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{
    self, ContentPurpose, ZwpTextInputV3,
};
use smithay::reexports::wayland_protocols_misc::zwp_input_method_v2::server::zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2;
use smithay::reexports::wayland_protocols_misc::zwp_input_method_v2::server::zwp_input_method_manager_v2::{
    self, ZwpInputMethodManagerV2,
};
use smithay::reexports::wayland_protocols_misc::zwp_input_method_v2::server::zwp_input_method_v2::{
    self, ZwpInputMethodV2,
};
use smithay::reexports::wayland_protocols_misc::zwp_input_method_v2::server::zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2;
use smithay::reexports::wayland_protocols_misc::zwp_virtual_keyboard_v1::server::zwp_virtual_keyboard_manager_v1::{
    self, ZwpVirtualKeyboardManagerV1,
};
use smithay::reexports::wayland_protocols_misc::zwp_virtual_keyboard_v1::server::zwp_virtual_keyboard_v1::{
    self, ZwpVirtualKeyboardV1,
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, Resource, delegate_dispatch, delegate_global_dispatch,
};
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::input_method::{
    InputMethodHandler, InputMethodKeyboardUserData, InputMethodManagerGlobalData,
    InputMethodManagerState, InputMethodPopupSurfaceUserData, InputMethodUserData, PopupSurface,
};
use smithay::wayland::text_input::{TextInputManagerState, TextInputSeat, TextInputUserData};
use smithay::wayland::virtual_keyboard::{
    VirtualKeyboardManagerGlobalData, VirtualKeyboardManagerState, VirtualKeyboardUserData,
};

use crate::state::{State, mapped_size};
use crate::trust::Access;

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
        if self.is_locked() {
            return;
        }
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

pub(crate) struct Refused;

impl State {
    pub(crate) fn input_method_must_not_see_keys(&self) -> bool {
        if self.is_locked() {
            return true;
        }
        let mut in_text_field = false;
        let mut secret = false;
        self.seat
            .text_input()
            .with_active_text_input(|text_input, _| {
                in_text_field = true;
                secret = self.secret_text_inputs.contains(text_input);
            });
        !in_text_field || secret
    }
}

impl Dispatch<ZwpInputMethodManagerV2, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        manager: &ZwpInputMethodManagerV2,
        request: zwp_input_method_manager_v2::Request,
        data: &(),
        display: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_input_method_manager_v2::Request::GetInputMethod { input_method, .. }
                if !state.may(Access::ReadKeys, client) =>
            {
                init.init(input_method, Refused).unavailable();
            }
            request => <InputMethodManagerState as Dispatch<_, _, Self>>::request(
                state, client, manager, request, data, display, init,
            ),
        }
    }
}

impl Dispatch<ZwpInputMethodV2, InputMethodUserData<State>> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        input_method: &ZwpInputMethodV2,
        request: zwp_input_method_v2::Request,
        data: &InputMethodUserData<State>,
        display: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        let writes_text = matches!(
            request,
            zwp_input_method_v2::Request::CommitString { .. }
                | zwp_input_method_v2::Request::SetPreeditString { .. }
                | zwp_input_method_v2::Request::DeleteSurroundingText { .. }
        );
        if writes_text && state.is_locked() {
            return;
        }
        <InputMethodManagerState as Dispatch<_, _, Self>>::request(
            state,
            client,
            input_method,
            request,
            data,
            display,
            init,
        );
    }

    fn destroyed(
        state: &mut Self,
        client: ClientId,
        input_method: &ZwpInputMethodV2,
        data: &InputMethodUserData<State>,
    ) {
        <InputMethodManagerState as Dispatch<_, _, Self>>::destroyed(
            state,
            client,
            input_method,
            data,
        );
    }
}

impl Dispatch<ZwpInputMethodV2, Refused> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwpInputMethodV2,
        request: zwp_input_method_v2::Request,
        _: &Refused,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_input_method_v2::Request::GrabKeyboard { keyboard } => {
                init.init(keyboard, Refused);
            }
            zwp_input_method_v2::Request::GetInputPopupSurface { id, .. } => {
                init.init(id, Refused);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpInputMethodKeyboardGrabV2, Refused> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwpInputMethodKeyboardGrabV2,
        _: <ZwpInputMethodKeyboardGrabV2 as Resource>::Request,
        _: &Refused,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ZwpInputPopupSurfaceV2, Refused> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwpInputPopupSurfaceV2,
        _: <ZwpInputPopupSurfaceV2 as Resource>::Request,
        _: &Refused,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ZwpVirtualKeyboardManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        manager: &ZwpVirtualKeyboardManagerV1,
        request: zwp_virtual_keyboard_manager_v1::Request,
        data: &(),
        display: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if !state.may(Access::TypeKeys, client) {
            manager.post_error(
                zwp_virtual_keyboard_manager_v1::Error::Unauthorized,
                "typing into other programs needs the person's consent in Privacy & Security",
            );
            return;
        }
        <VirtualKeyboardManagerState as Dispatch<_, _, Self>>::request(
            state, client, manager, request, data, display, init,
        );
    }
}

impl Dispatch<ZwpVirtualKeyboardV1, VirtualKeyboardUserData<State>> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        keyboard: &ZwpVirtualKeyboardV1,
        request: zwp_virtual_keyboard_v1::Request,
        data: &VirtualKeyboardUserData<State>,
        display: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        let reaches_focus = matches!(
            request,
            zwp_virtual_keyboard_v1::Request::Key { .. }
                | zwp_virtual_keyboard_v1::Request::Modifiers { .. }
        );
        if reaches_focus && state.is_locked() {
            return;
        }
        <VirtualKeyboardManagerState as Dispatch<_, _, Self>>::request(
            state, client, keyboard, request, data, display, init,
        );
    }

    fn destroyed(
        state: &mut Self,
        client: ClientId,
        keyboard: &ZwpVirtualKeyboardV1,
        data: &VirtualKeyboardUserData<State>,
    ) {
        <VirtualKeyboardManagerState as Dispatch<_, _, Self>>::destroyed(
            state, client, keyboard, data,
        );
    }
}

impl Dispatch<ZwpTextInputV3, TextInputUserData> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        text_input: &ZwpTextInputV3,
        request: zwp_text_input_v3::Request,
        data: &TextInputUserData,
        display: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let zwp_text_input_v3::Request::SetContentType { purpose, .. } = &request {
            state.secret_text_inputs.retain(|known| known != text_input);
            if matches!(
                purpose.into_result(),
                Ok(ContentPurpose::Password | ContentPurpose::Pin)
            ) {
                state.secret_text_inputs.push(text_input.clone());
            }
        }
        <TextInputManagerState as Dispatch<_, _, Self>>::request(
            state, client, text_input, request, data, display, init,
        );
    }

    fn destroyed(
        state: &mut Self,
        client: ClientId,
        text_input: &ZwpTextInputV3,
        data: &TextInputUserData,
    ) {
        state.secret_text_inputs.retain(|known| known != text_input);
        <TextInputManagerState as Dispatch<_, _, Self>>::destroyed(state, client, text_input, data);
    }
}

delegate_global_dispatch!(State: [ZwpInputMethodManagerV2: InputMethodManagerGlobalData] => InputMethodManagerState);
delegate_dispatch!(State: [ZwpInputMethodKeyboardGrabV2: InputMethodKeyboardUserData<State>] => InputMethodManagerState);
delegate_dispatch!(State: [ZwpInputPopupSurfaceV2: InputMethodPopupSurfaceUserData] => InputMethodManagerState);
delegate_global_dispatch!(State: [ZwpVirtualKeyboardManagerV1: VirtualKeyboardManagerGlobalData] => VirtualKeyboardManagerState);
delegate_global_dispatch!(State: [ZwpTextInputManagerV3: ()] => TextInputManagerState);
delegate_dispatch!(State: [ZwpTextInputManagerV3: ()] => TextInputManagerState);
