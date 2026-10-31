use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::ZwpTextInputManagerV3;
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::{self, ZwpTextInputV3};
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2::{
    self, ZwpInputMethodKeyboardGrabV2,
};
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::ZwpInputMethodManagerV2;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::{
    self, ZwpInputMethodV2,
};
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2;

use crate::{Compositor, Globals, ShmImage, TestClient};

#[derive(Default)]
pub(crate) struct TextInput {
    text_input: Option<ZwpTextInputV3>,
    entered: bool,
    pending_commit: Option<String>,
    committed: Vec<String>,
}

#[derive(Default)]
pub(crate) struct InputMethod {
    input_method: Option<ZwpInputMethodV2>,
    pending_active: bool,
    active: bool,
    done: u32,
    grab: Option<ZwpInputMethodKeyboardGrabV2>,
    grabbed_keys: Vec<(u32, bool)>,
    candidates: Option<(WlSurface, ZwpInputPopupSurfaceV2)>,
}

impl TestClient {
    pub fn enable_text_input(&mut self, server: &mut Compositor, cursor: (i32, i32, i32, i32)) {
        let manager: ZwpTextInputManagerV3 = self.bind("zwp_text_input_manager_v3", 1);
        let seat = self.globals.input.seat.as_ref().expect("use_seat first");
        let text_input = manager.get_text_input(seat, &self.queue.handle(), ());
        self.exchange(server);
        assert!(
            self.globals.text_input.entered,
            "the focused window's text input is entered"
        );
        let (x, y, width, height) = cursor;
        text_input.enable();
        text_input.set_cursor_rectangle(x, y, width, height);
        text_input.commit();
        self.globals.text_input.text_input = Some(text_input);
        self.exchange(server);
    }

    pub fn committed_text(&self) -> &[String] {
        &self.globals.text_input.committed
    }

    pub fn become_input_method(&mut self, server: &mut Compositor) {
        let manager: ZwpInputMethodManagerV2 = self.bind("zwp_input_method_manager_v2", 1);
        let seat = self.globals.input.seat.as_ref().expect("use_seat first");
        self.globals.input_method.input_method =
            Some(manager.get_input_method(seat, &self.queue.handle(), ()));
        self.exchange(server);
    }

    pub fn input_method_active(&self) -> bool {
        self.globals.input_method.active
    }

    fn input_method(&self) -> &ZwpInputMethodV2 {
        self.globals
            .input_method
            .input_method
            .as_ref()
            .expect("become_input_method first")
    }

    pub fn commit_text(&mut self, server: &mut Compositor, text: &str) {
        let input_method = self.input_method();
        input_method.commit_string(text.to_owned());
        input_method.commit(self.globals.input_method.done);
        self.exchange(server);
    }

    pub fn set_preedit(&mut self, server: &mut Compositor, text: &str, cursor: (i32, i32)) {
        let input_method = self.input_method();
        input_method.set_preedit_string(text.to_owned(), cursor.0, cursor.1);
        input_method.commit(self.globals.input_method.done);
        self.exchange(server);
    }

    pub fn grab_keyboard(&mut self, server: &mut Compositor) {
        let grab = self.input_method().grab_keyboard(&self.queue.handle(), ());
        self.globals.input_method.grab = Some(grab);
        self.exchange(server);
    }

    pub fn grabbed_keys(&self) -> &[(u32, bool)] {
        &self.globals.input_method.grabbed_keys
    }

    pub fn show_candidates(&mut self, server: &mut Compositor, image: &ShmImage) {
        let qh = self.queue.handle();
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let surface = compositor.create_surface(&qh, ());
        let popup = self
            .input_method()
            .get_input_popup_surface(&surface, &qh, ());
        let buffer = self.shm_buffer(image);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, image.width, image.height);
        surface.commit();
        self.globals.input_method.candidates = Some((surface, popup));
        self.exchange(server);
    }
}

impl Dispatch<ZwpTextInputV3, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let text_input = &mut globals.text_input;
        match event {
            zwp_text_input_v3::Event::Enter { .. } => text_input.entered = true,
            zwp_text_input_v3::Event::Leave { .. } => text_input.entered = false,
            zwp_text_input_v3::Event::CommitString { text } => text_input.pending_commit = text,
            zwp_text_input_v3::Event::Done { .. } => {
                if let Some(text) = text_input.pending_commit.take() {
                    text_input.committed.push(text);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpInputMethodV2, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let input_method = &mut globals.input_method;
        match event {
            zwp_input_method_v2::Event::Activate => input_method.pending_active = true,
            zwp_input_method_v2::Event::Deactivate => input_method.pending_active = false,
            zwp_input_method_v2::Event::Done => {
                input_method.active = input_method.pending_active;
                input_method.done += 1;
            }
            _ => {}
        }
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
            let pressed = matches!(
                state,
                WEnum::Value(wayland_client::protocol::wl_keyboard::KeyState::Pressed)
            );
            globals.input_method.grabbed_keys.push((key, pressed));
        }
    }
}

delegate_noop!(Globals: ZwpTextInputManagerV3);
delegate_noop!(Globals: ZwpInputMethodManagerV2);
delegate_noop!(Globals: ignore ZwpInputPopupSurfaceV2);
