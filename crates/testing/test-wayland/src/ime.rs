use wayland_client::protocol::wl_keyboard::KeyState;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2::{
    self, ZwpInputMethodKeyboardGrabV2,
};
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::ZwpInputMethodManagerV2;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::ZwpInputMethodV2;

use crate::{Compositor, Globals, TestClient};

#[derive(Default)]
pub(crate) struct InputMethod {
    input_method: Option<ZwpInputMethodV2>,
    grabbed_keys: Vec<(u32, bool)>,
}

impl TestClient {
    pub fn become_input_method(&mut self, server: &mut Compositor) {
        let manager: ZwpInputMethodManagerV2 = self.bind("zwp_input_method_manager_v2", 1);
        let seat = self.globals.input.seat.as_ref().expect("use_seat first");
        self.globals.input_method.input_method =
            Some(manager.get_input_method(seat, &self.queue.handle(), ()));
        self.exchange(server);
    }

    pub fn grab_keyboard(&mut self, server: &mut Compositor) {
        let _: ZwpInputMethodKeyboardGrabV2 = self
            .globals
            .input_method
            .input_method
            .as_ref()
            .expect("become_input_method first")
            .grab_keyboard(&self.queue.handle(), ());
        self.exchange(server);
    }

    pub fn grabbed_keys(&self) -> &[(u32, bool)] {
        &self.globals.input_method.grabbed_keys
    }
}

impl Dispatch<ZwpInputMethodKeyboardGrabV2, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZwpInputMethodKeyboardGrabV2,
        event: zwp_input_method_keyboard_grab_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_input_method_keyboard_grab_v2::Event::Key { key, state, .. } = event {
            let pressed = matches!(state, WEnum::Value(KeyState::Pressed));
            globals.input_method.grabbed_keys.push((key, pressed));
        }
    }
}

delegate_noop!(Globals: ZwpInputMethodManagerV2);
delegate_noop!(Globals: ignore ZwpInputMethodV2);
