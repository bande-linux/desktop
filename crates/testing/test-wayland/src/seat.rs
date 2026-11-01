use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1;
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1;

use crate::{Compositor, Globals, TestClient};

#[derive(Default)]
pub(crate) struct Input {
    pub(crate) keyboard_focus: Option<WlSurface>,
    keys: Vec<(u32, bool)>,
    buttons: Vec<(u32, bool)>,
    pub(crate) seat: Option<WlSeat>,
    pub(crate) button_serial: u32,
}

pub(crate) struct Devices;

impl TestClient {
    pub fn use_seat(&mut self, server: &mut Compositor) {
        let registry = self.globals.registry.as_ref().expect("registry");
        let (name, _) = self
            .globals
            .advertised
            .iter()
            .find(|(_, interface)| interface == "wl_seat")
            .expect("wl_seat is advertised");
        let seat: WlSeat = registry.bind(*name, 7, &self.queue.handle(), Devices);
        self.globals.input.seat = Some(seat);
        self.exchange(server);
    }

    pub fn inhibit_shortcuts(&mut self, server: &mut Compositor, window: usize) {
        let manager: ZwpKeyboardShortcutsInhibitManagerV1 =
            self.bind("zwp_keyboard_shortcuts_inhibit_manager_v1", 1);
        let seat = self.globals.input.seat.as_ref().expect("use_seat first");
        let _: ZwpKeyboardShortcutsInhibitorV1 =
            manager.inhibit_shortcuts(&self.windows[window].0, seat, &self.queue.handle(), ());
        self.exchange(server);
    }

    pub fn keyboard_focus_on(&self, index: usize) -> bool {
        self.globals.input.keyboard_focus.as_ref() == Some(&self.windows[index].0)
    }

    pub fn keys(&self) -> &[(u32, bool)] {
        &self.globals.input.keys
    }

    pub fn buttons(&self) -> &[(u32, bool)] {
        &self.globals.input.buttons
    }
}

impl Dispatch<WlSeat, Devices> for Globals {
    fn event(
        _: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &Devices,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) {
                seat.get_keyboard(qh, Devices);
            }
            if capabilities.contains(wl_seat::Capability::Pointer) {
                seat.get_pointer(qh, Devices);
            }
        }
    }
}

fn pressed<T: PartialEq>(state: WEnum<T>, down: T) -> bool {
    state.into_result().is_ok_and(|state| state == down)
}

impl Dispatch<WlKeyboard, Devices> for Globals {
    fn event(
        globals: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &Devices,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let input = &mut globals.input;
        match event {
            wl_keyboard::Event::Enter { surface, .. } => input.keyboard_focus = Some(surface),
            wl_keyboard::Event::Leave { .. } => input.keyboard_focus = None,
            wl_keyboard::Event::Key { key, state, .. } => input
                .keys
                .push((key, pressed(state, wl_keyboard::KeyState::Pressed))),
            _ => {}
        }
    }
}

impl Dispatch<WlPointer, Devices> for Globals {
    fn event(
        globals: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &Devices,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button {
            serial,
            button,
            state,
            ..
        } = event
        {
            globals.input.button_serial = serial;
            globals
                .input
                .buttons
                .push((button, pressed(state, wl_pointer::ButtonState::Pressed)));
        }
    }
}

delegate_noop!(Globals: ZwpKeyboardShortcutsInhibitManagerV1);
delegate_noop!(Globals: ignore ZwpKeyboardShortcutsInhibitorV1);
