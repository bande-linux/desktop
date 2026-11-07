use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};

use wayland_client::backend::protocol::ProtocolError;
use wayland_client::protocol::wl_keyboard::KeyState;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::security_context::v1::client::wp_security_context_manager_v1::WpSecurityContextManagerV1;
use wayland_protocols::wp::security_context::v1::client::wp_security_context_v1::WpSecurityContextV1;
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::ZwpTextInputManagerV3;
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::{
    ContentHint, ContentPurpose, ZwpTextInputV3,
};
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2::{
    self, ZwpInputMethodKeyboardGrabV2,
};
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::ZwpInputMethodManagerV2;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::{
    self, ZwpInputMethodV2,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

use crate::{Compositor, Globals, TestClient};

const XKB_V1: u32 = 1;
const KEYMAP: &str = "xkb_keymap { xkb_keycodes { include \"evdev\" }; xkb_types { include \"complete\" }; xkb_compat { include \"complete\" }; xkb_symbols { include \"pc+us\" }; };\0";

#[derive(Default)]
pub(crate) struct InputMethod {
    input_method: Option<ZwpInputMethodV2>,
    grabbed_keys: Vec<(u32, bool)>,
    unavailable: bool,
    text_input: Option<ZwpTextInputV3>,
    virtual_keyboard: Option<ZwpVirtualKeyboardV1>,
    sandbox: Option<(tempfile::TempDir, std::io::PipeWriter)>,
}

pub fn grant_consent(server: &mut Compositor, lines: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("consent directory");
    let file = directory.path().join("consent");
    std::fs::write(&file, lines).expect("consent file");
    server.set_consent_file(file);
    directory
}

impl TestClient {
    pub fn connect_sandboxed(server: &mut Compositor, app_id: &str) -> Self {
        let mut flatpak = Self::connect(server);
        let manager: WpSecurityContextManagerV1 = flatpak.bind("wp_security_context_manager_v1", 1);
        let directory = tempfile::tempdir().expect("socket directory");
        let path = directory.path().join("wayland");
        let listener = UnixListener::bind(&path).expect("sandbox socket");
        let (closing, keep_open) = std::io::pipe().expect("close pipe");
        let qh = flatpak.queue.handle();
        let context = manager.create_listener(listener.as_fd(), closing.as_fd(), &qh, ());
        context.set_sandbox_engine("org.flatpak".to_owned());
        context.set_app_id(app_id.to_owned());
        context.set_instance_id("1".to_owned());
        context.commit();
        flatpak.exchange(server);
        let stream = UnixStream::connect(&path).expect("connect to the sandbox socket");
        server.dispatch().expect("accept the sandboxed client");
        let mut client = Self::from_socket(server, stream);
        client.globals.input_method.sandbox = Some((directory, keep_open));
        client
    }

    pub fn disconnected_after_dispatch(&mut self, server: &mut Compositor) -> bool {
        for _ in 0..8 {
            server.dispatch().expect("server dispatch");
            if let Some(input_method) = &self.globals.input_method.input_method {
                input_method.commit(0);
            }
            if self.connection.flush().is_err() {
                return true;
            }
            if let Some(guard) = self.connection.prepare_read() {
                let _ = guard.read();
            }
            if self.queue.dispatch_pending(&mut self.globals).is_err() {
                return true;
            }
        }
        false
    }

    pub fn sees_global(&self, interface: &str) -> bool {
        self.globals
            .advertised
            .iter()
            .any(|(_, advertised)| advertised == interface)
    }

    pub fn input_method_unavailable(&self) -> bool {
        self.globals.input_method.unavailable
    }

    pub fn enable_text_input(&mut self, server: &mut Compositor) {
        let manager: ZwpTextInputManagerV3 = self.bind("zwp_text_input_manager_v3", 1);
        let seat = self.globals.input.seat.as_ref().expect("use_seat first");
        let text_input = manager.get_text_input(seat, &self.queue.handle(), ());
        self.exchange(server);
        text_input.enable();
        text_input.commit();
        self.globals.input_method.text_input = Some(text_input);
        self.exchange(server);
    }

    pub fn mark_text_input_as_password(&mut self, server: &mut Compositor) {
        let text_input = self
            .globals
            .input_method
            .text_input
            .as_ref()
            .expect("enable_text_input first");
        text_input.set_content_type(ContentHint::SensitiveData, ContentPurpose::Password);
        text_input.commit();
        self.exchange(server);
    }

    pub fn commit_text(&mut self, server: &mut Compositor, text: &str) {
        let input_method = self
            .globals
            .input_method
            .input_method
            .as_ref()
            .expect("become_input_method first");
        input_method.commit_string(text.to_owned());
        input_method.commit(0);
        self.exchange(server);
    }

    pub fn create_virtual_keyboard(
        &mut self,
        server: &mut Compositor,
    ) -> Result<(), ProtocolError> {
        let manager: ZwpVirtualKeyboardManagerV1 = self.bind("zwp_virtual_keyboard_manager_v1", 1);
        let seat = self.globals.input.seat.as_ref().expect("use_seat first");
        let keyboard = manager.create_virtual_keyboard(seat, &self.queue.handle(), ());
        let mut keymap = tempfile::tempfile().expect("keymap file");
        keymap.write_all(KEYMAP.as_bytes()).expect("write keymap");
        keyboard.keymap(XKB_V1, keymap.as_fd(), KEYMAP.len() as u32);
        self.globals.input_method.virtual_keyboard = Some(keyboard);
        let _ = self.connection.flush();
        server.dispatch().expect("server dispatch");
        let _ = self.connection.flush();
        if let Some(guard) = self.connection.prepare_read() {
            let _ = guard.read();
        }
        let _ = self.queue.dispatch_pending(&mut self.globals);
        match self.connection.protocol_error() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub fn type_on_virtual_keyboard(&mut self, server: &mut Compositor, key: u32) {
        let keyboard = self
            .globals
            .input_method
            .virtual_keyboard
            .as_ref()
            .expect("create_virtual_keyboard first");
        keyboard.key(0, key, 1);
        keyboard.key(1, key, 0);
        self.exchange(server);
    }

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

impl Dispatch<ZwpInputMethodV2, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, zwp_input_method_v2::Event::Unavailable) {
            globals.input_method.unavailable = true;
        }
    }
}

delegate_noop!(Globals: ZwpInputMethodManagerV2);
delegate_noop!(Globals: ZwpVirtualKeyboardManagerV1);
delegate_noop!(Globals: ZwpVirtualKeyboardV1);
delegate_noop!(Globals: ZwpTextInputManagerV3);
delegate_noop!(Globals: ignore ZwpTextInputV3);
delegate_noop!(Globals: WpSecurityContextManagerV1);
delegate_noop!(Globals: WpSecurityContextV1);
