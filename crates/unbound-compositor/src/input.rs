use std::io;

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, ButtonState, Event, InputBackend, InputEvent, KeyState,
    KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::desktop::WindowSurfaceType;
use smithay::desktop::utils::under_from_surface_tree;
use smithay::input::keyboard::{FilterResult, Keycode, keysyms};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use crate::state::{Headless, State};

const EVDEV_OFFSET: u32 = 8;

const SCROLL_PER_DETENT: f64 = 15.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shortcut {
    CloseWindow,
    QuitApp,
    NextApp,
    ShowLauncher,
    MinimizeWindow,
    LockScreen,
    ToggleFullscreen,
}

fn shortcut(keysym: u32, ctrl: bool) -> Option<Shortcut> {
    match keysym {
        keysyms::KEY_f | keysyms::KEY_F if ctrl => Some(Shortcut::ToggleFullscreen),
        keysyms::KEY_q | keysyms::KEY_Q if ctrl => Some(Shortcut::LockScreen),
        keysyms::KEY_m | keysyms::KEY_M => Some(Shortcut::MinimizeWindow),
        keysyms::KEY_w | keysyms::KEY_W => Some(Shortcut::CloseWindow),
        keysyms::KEY_q | keysyms::KEY_Q => Some(Shortcut::QuitApp),
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => Some(Shortcut::NextApp),
        keysyms::KEY_space => Some(Shortcut::ShowLauncher),
        _ => None,
    }
}

impl State {
    fn surface_under(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.stack().into_iter().find_map(|(surface, origin)| {
            let origin: Point<i32, Logical> = (origin.x, origin.y).into();
            under_from_surface_tree(&surface, location, origin, WindowSurfaceType::ALL)
                .map(|(surface, at)| (surface, at.to_f64()))
        })
    }

    fn run(&mut self, shortcut: Shortcut) {
        if shortcut == Shortcut::ShowLauncher {
            self.show_launcher();
            return;
        }
        if shortcut == Shortcut::LockScreen {
            self.lock_screen_requested = !self.is_locked();
            return;
        }
        let Some(focused) = self.focused() else {
            return;
        };
        let owner = focused.client();
        match shortcut {
            Shortcut::CloseWindow => {
                if let Some(window) = self
                    .windows
                    .iter()
                    .find(|window| window.toplevel.wl_surface() == &focused)
                {
                    window.toplevel.send_close();
                }
            }
            Shortcut::QuitApp => self
                .windows
                .iter()
                .filter(|window| window.toplevel.wl_surface().client() == owner)
                .for_each(|window| window.toplevel.send_close()),
            Shortcut::NextApp => {
                let next = self
                    .windows
                    .iter()
                    .rev()
                    .filter(|window| window.visible())
                    .map(|window| window.toplevel.wl_surface())
                    .find(|surface| surface.client() != owner)
                    .cloned();
                if next.is_some() {
                    self.focus(next);
                }
            }
            Shortcut::MinimizeWindow => {
                if let Some(window) = self.toplevel_of(&focused) {
                    self.minimize(&window);
                }
            }
            Shortcut::ToggleFullscreen => {
                if let Some(window) = self.toplevel_of(&focused) {
                    self.toggle_fullscreen(&window);
                }
            }
            Shortcut::ShowLauncher | Shortcut::LockScreen => {}
        }
    }
}

impl Headless {
    pub fn handle_input<B: InputBackend>(&mut self, event: InputEvent<B>) -> io::Result<()> {
        match event {
            InputEvent::DeviceAdded { .. } => self.state.input_devices += 1,
            InputEvent::DeviceRemoved { .. } => {
                self.state.input_devices = self.state.input_devices.saturating_sub(1);
            }
            InputEvent::Keyboard { event } => {
                let pressed = event.state() == KeyState::Pressed;
                let keycode = event.key_code().raw().saturating_sub(EVDEV_OFFSET);
                self.key(keycode, pressed, event.time_msec());
            }
            InputEvent::PointerMotion { event } => {
                let location = self.state.pointer_location + event.delta();
                self.move_pointer_to(location, event.time_msec());
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let size = self.output_size();
                let location = event.position_transformed((size.w, size.h).into());
                self.move_pointer_to(location, event.time_msec());
            }
            InputEvent::PointerButton { event } => {
                let pressed = event.state() == ButtonState::Pressed;
                self.press_button(event.button_code(), pressed, event.time_msec());
            }
            InputEvent::PointerAxis { event } => {
                let mut frame = AxisFrame::new(event.time_msec()).source(event.source());
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    if let Some(amount) = event.amount(axis) {
                        frame = frame.value(axis, amount);
                    } else if let Some(v120) = event.amount_v120(axis) {
                        frame = frame.value(axis, v120 / 120.0 * SCROLL_PER_DETENT);
                    }
                    if let Some(v120) = event.amount_v120(axis) {
                        frame = frame.v120(axis, v120 as i32);
                    }
                }
                self.scroll(frame);
            }
            _ => return Ok(()),
        }
        self.flush()
    }

    pub fn scroll(&mut self, frame: AxisFrame) {
        let state = &mut self.state;
        let Some(pointer) = state.seat.get_pointer() else {
            return;
        };
        pointer.axis(state, frame);
        pointer.frame(state);
    }

    pub fn input_devices(&self) -> usize {
        self.state.input_devices
    }

    pub fn pointer_location(&self) -> Point<f64, Logical> {
        self.state.pointer_location
    }

    pub fn move_pointer_to(&mut self, location: Point<f64, Logical>, time: u32) {
        let size = self.output_size();
        let location: Point<f64, Logical> = (
            location.x.clamp(0.0, f64::from(size.w - 1)),
            location.y.clamp(0.0, f64::from(size.h - 1)),
        )
            .into();
        let state = &mut self.state;
        state.pointer_location = location;
        state.cursor_shown = true;
        state.needs_redraw = true;
        let under = state.surface_under(location);
        let Some(pointer) = state.seat.get_pointer() else {
            return;
        };
        pointer.motion(
            state,
            under,
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(state);
    }

    pub fn press_button(&mut self, button: u32, pressed: bool, time: u32) {
        let state = &mut self.state;
        if !pressed && state.swallowed_button == Some(button) {
            state.swallowed_button = None;
            return;
        }
        if pressed {
            let under = state.surface_under(state.pointer_location);
            if state.dismisses_popups(under.as_ref().map(|(surface, _)| surface)) {
                state.swallowed_button = Some(button);
                return;
            }
            if state.popup_grab.is_none() {
                match under {
                    Some((surface, _)) => {
                        if let Some(window) = state.toplevel_of(&surface) {
                            state.focus(Some(window));
                        }
                    }
                    None => state.focus(None),
                }
            }
        }
        let Some(pointer) = state.seat.get_pointer() else {
            return;
        };
        pointer.button(
            state,
            &ButtonEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time,
                button,
                state: if pressed {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                },
            },
        );
        pointer.frame(state);
    }

    pub fn key(&mut self, evdev_code: u32, pressed: bool, time: u32) {
        let state = &mut self.state;
        let Some(keyboard) = state.seat.get_keyboard() else {
            return;
        };
        let key_state = if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        };
        let action = keyboard.input(
            state,
            Keycode::new(evdev_code + EVDEV_OFFSET),
            key_state,
            SERIAL_COUNTER.next_serial(),
            time,
            |state, modifiers, keysym| {
                if !pressed {
                    return if state.held_shortcut_keys.contains(&evdev_code) {
                        state.held_shortcut_keys.retain(|held| *held != evdev_code);
                        FilterResult::Intercept(None)
                    } else {
                        FilterResult::Forward
                    };
                }
                match shortcut(keysym.modified_sym().raw(), modifiers.ctrl)
                    .filter(|_| modifiers.logo && !state.is_locked())
                {
                    Some(shortcut) => {
                        state.held_shortcut_keys.push(evdev_code);
                        FilterResult::Intercept(Some(shortcut))
                    }
                    None => FilterResult::Forward,
                }
            },
        );
        if let Some(Some(shortcut)) = action {
            state.run(shortcut);
        }
    }
}
