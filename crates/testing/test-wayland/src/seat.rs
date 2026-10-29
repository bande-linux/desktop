use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};

use crate::{Globals, Headless, TestClient};

#[derive(Default)]
pub(crate) struct Input {
    keyboard_focus: Option<WlSurface>,
    pointer_focus: Option<(WlSurface, f64, f64)>,
    keys: Vec<(u32, bool)>,
    buttons: Vec<(u32, bool)>,
    closed: Vec<XdgToplevel>,
    scrolled: f64,
    motions: usize,
    pub(crate) seat: Option<WlSeat>,
    pub(crate) pointer: Option<WlPointer>,
    pub(crate) pointer_enter_serial: u32,
    pub(crate) button_serial: u32,
}

pub(crate) struct Devices;

impl Input {
    pub(crate) fn keyboard_surface(&self) -> Option<&WlSurface> {
        self.keyboard_focus.as_ref()
    }
}

impl TestClient {
    pub fn use_seat(&mut self, server: &mut Headless) {
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

    pub fn keyboard_focus_on(&self, index: usize) -> bool {
        self.globals.input.keyboard_focus.as_ref() == Some(&self.windows[index].0)
    }

    pub fn pointer_focus(&self) -> Option<(usize, f64, f64)> {
        let (surface, x, y) = self.globals.input.pointer_focus.as_ref()?;
        let index = self
            .windows
            .iter()
            .position(|(window, _, _)| window == surface)?;
        Some((index, *x, *y))
    }

    pub fn keys(&self) -> &[(u32, bool)] {
        &self.globals.input.keys
    }

    pub fn pointer_motions(&self) -> usize {
        self.globals.input.motions
    }

    pub fn scrolled(&self) -> f64 {
        self.globals.input.scrolled
    }

    pub fn buttons(&self) -> &[(u32, bool)] {
        &self.globals.input.buttons
    }

    pub fn close_requested(&self, index: usize) -> bool {
        self.globals.input.closed.contains(&self.windows[index].2)
    }
}

impl Dispatch<WlSeat, Devices> for Globals {
    fn event(
        globals: &mut Self,
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
                globals.input.pointer = Some(seat.get_pointer(qh, Devices));
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
        let input = &mut globals.input;
        match event {
            wl_pointer::Event::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
            } => {
                input.pointer_enter_serial = serial;
                input.pointer_focus = Some((surface, surface_x, surface_y));
            }
            wl_pointer::Event::Leave { .. } => input.pointer_focus = None,
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                input.motions += 1;
                if let Some(focus) = input.pointer_focus.as_mut() {
                    focus.1 = surface_x;
                    focus.2 = surface_y;
                }
            }
            wl_pointer::Event::Axis {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                value,
                ..
            } => input.scrolled += value,
            wl_pointer::Event::Button {
                serial,
                button,
                state,
                ..
            } => {
                input.button_serial = serial;
                input
                    .buttons
                    .push((button, pressed(state, wl_pointer::ButtonState::Pressed)));
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgToplevel, ()> for Globals {
    fn event(
        globals: &mut Self,
        toplevel: &XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Close => globals.input.closed.push(toplevel.clone()),
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            } => globals
                .toplevels
                .configured(toplevel, width, height, &states),
            _ => {}
        }
    }
}
