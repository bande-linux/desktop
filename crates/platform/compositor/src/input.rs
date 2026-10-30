use std::io;

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, ButtonState, Event, GestureBeginEvent, GestureEndEvent,
    GesturePinchUpdateEvent, GestureSwipeUpdateEvent, InputBackend, InputEvent, KeyState,
    KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::desktop::WindowSurfaceType;
use smithay::desktop::utils::under_from_surface_tree;
use smithay::input::keyboard::{FilterResult, Keycode, keysyms};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Clock, Logical, Monotonic, Point, SERIAL_COUNTER};
use unbound_protocols::server::unbound_shell_v1::{BrightnessKey, VolumeKey};

use crate::gesture::Gesture;
use crate::motion::DOCK_NAMESPACE;
use crate::shell::toplevel_names;
use crate::state::{Headless, MENU_BAR_NAMESPACE, State};
use crate::tile::Direction;

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
    ForceQuit,
    FocusShell(&'static str),
    ToggleFullscreen,
    Place(Direction),
    CaptureScreen,
    CaptureArea,
    Volume(VolumeKey, bool),
    Brightness(BrightnessKey, bool),
    HideApp,
    HideOthers,
}

fn volume_key(keysym: u32) -> Option<VolumeKey> {
    match keysym {
        keysyms::KEY_XF86AudioMute => Some(VolumeKey::Mute),
        keysyms::KEY_XF86AudioLowerVolume => Some(VolumeKey::Down),
        keysyms::KEY_XF86AudioRaiseVolume => Some(VolumeKey::Up),
        _ => None,
    }
}

fn brightness_key(keysym: u32) -> Option<BrightnessKey> {
    match keysym {
        keysyms::KEY_XF86MonBrightnessDown => Some(BrightnessKey::DisplayDown),
        keysyms::KEY_XF86MonBrightnessUp => Some(BrightnessKey::DisplayUp),
        keysyms::KEY_XF86KbdBrightnessDown => Some(BrightnessKey::KeyboardDown),
        keysyms::KEY_XF86KbdBrightnessUp => Some(BrightnessKey::KeyboardUp),
        _ => None,
    }
}

fn shortcut(
    keysym: u32,
    unshifted: Option<u32>,
    ctrl: bool,
    alt: bool,
    shift: bool,
) -> Option<Shortcut> {
    match unshifted.filter(|_| shift) {
        Some(keysyms::KEY_3) => return Some(Shortcut::CaptureScreen),
        Some(keysyms::KEY_4) => return Some(Shortcut::CaptureArea),
        _ => {}
    }
    match unshifted {
        Some(keysyms::KEY_h) if alt => return Some(Shortcut::HideOthers),
        Some(keysyms::KEY_h) if !ctrl => return Some(Shortcut::HideApp),
        _ => {}
    }
    match keysym {
        keysyms::KEY_Escape if alt => Some(Shortcut::ForceQuit),
        keysyms::KEY_F2 => Some(Shortcut::FocusShell(MENU_BAR_NAMESPACE)),
        keysyms::KEY_F3 => Some(Shortcut::FocusShell(DOCK_NAMESPACE)),
        keysyms::KEY_f | keysyms::KEY_F if ctrl => Some(Shortcut::ToggleFullscreen),
        keysyms::KEY_Left if ctrl => Some(Shortcut::Place(Direction::Left)),
        keysyms::KEY_Right if ctrl => Some(Shortcut::Place(Direction::Right)),
        keysyms::KEY_Up if ctrl => Some(Shortcut::Place(Direction::Up)),
        keysyms::KEY_Down if ctrl => Some(Shortcut::Place(Direction::Down)),
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

    pub(crate) fn refocus_pointer(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let time = Clock::<Monotonic>::new().now().as_millis();
        if self
            .popup_grab
            .as_ref()
            .is_some_and(|grab| grab.has_ended())
        {
            self.popup_grab = None;
            pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), time);
        }
        let mut under = self.surface_under(self.pointer_location);
        if pointer.is_grabbed() {
            let Some(popup) = self
                .popup_grab
                .as_ref()
                .and_then(|grab| grab.current_grab())
            else {
                return;
            };
            under = under.filter(|(surface, _)| surface.client() == popup.client());
        }
        if pointer.current_focus() == under.as_ref().map(|(surface, _)| surface.clone()) {
            return;
        }
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: self.pointer_location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
    }

    fn run(&mut self, shortcut: Shortcut) {
        if let Shortcut::Volume(key, fine) = shortcut {
            self.volume_key(key, fine);
            return;
        }
        if let Shortcut::Brightness(key, fine) = shortcut {
            self.brightness_key(key, fine);
            return;
        }
        if shortcut == Shortcut::ShowLauncher {
            self.show_launcher();
            return;
        }
        if shortcut == Shortcut::CaptureScreen {
            self.capture_screen();
            return;
        }
        if shortcut == Shortcut::CaptureArea {
            self.capture_area();
            return;
        }
        if shortcut == Shortcut::LockScreen {
            self.lock_screen_requested = !self.is_locked();
            return;
        }
        if let Shortcut::FocusShell(namespace) = shortcut {
            self.focus_shell(namespace);
            return;
        }
        if shortcut == Shortcut::ForceQuit {
            self.force_quit_requested = true;
            self.watchdog.ping_everyone();
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
            Shortcut::QuitApp => self.quit_app(owner),
            Shortcut::HideApp => self.hide_app(&toplevel_names(&focused).1),
            Shortcut::HideOthers => {
                let keep = toplevel_names(&focused).1;
                let others: Vec<String> = self
                    .windows
                    .iter()
                    .map(|window| toplevel_names(window.surface()).1)
                    .filter(|app_id| *app_id != keep)
                    .collect();
                for app_id in others {
                    self.hide_app(&app_id);
                }
            }
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
            Shortcut::Place(direction) => {
                if let Some(window) = self.toplevel_of(&focused) {
                    self.place_with_key(&window, direction);
                }
            }
            Shortcut::ToggleFullscreen => {
                if let Some(window) = self.toplevel_of(&focused) {
                    self.toggle_fullscreen(&window);
                }
            }
            Shortcut::ShowLauncher
            | Shortcut::LockScreen
            | Shortcut::ForceQuit
            | Shortcut::FocusShell(_)
            | Shortcut::CaptureScreen
            | Shortcut::CaptureArea
            | Shortcut::Volume(..)
            | Shortcut::Brightness(..) => {}
        }
    }
}

impl Headless {
    pub fn handle_input<B: InputBackend>(&mut self, event: InputEvent<B>) -> io::Result<()> {
        self.state.note_activity();
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
                let location = event.position_transformed(self.logical_output_size());
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
            InputEvent::GestureSwipeBegin { event } => self.gesture(
                Gesture::SwipeBegin {
                    fingers: event.fingers(),
                },
                event.time_msec(),
            ),
            InputEvent::GestureSwipeUpdate { event } => self.gesture(
                Gesture::SwipeUpdate {
                    delta: event.delta(),
                },
                event.time_msec(),
            ),
            InputEvent::GestureSwipeEnd { event } => self.gesture(
                Gesture::SwipeEnd {
                    cancelled: event.cancelled(),
                },
                event.time_msec(),
            ),
            InputEvent::GesturePinchBegin { event } => self.gesture(
                Gesture::PinchBegin {
                    fingers: event.fingers(),
                },
                event.time_msec(),
            ),
            InputEvent::GesturePinchUpdate { event } => self.gesture(
                Gesture::PinchUpdate {
                    delta: event.delta(),
                    scale: event.scale(),
                    rotation: event.rotation(),
                },
                event.time_msec(),
            ),
            InputEvent::GesturePinchEnd { event } => self.gesture(
                Gesture::PinchEnd {
                    cancelled: event.cancelled(),
                },
                event.time_msec(),
            ),
            InputEvent::GestureHoldBegin { event } => self.gesture(
                Gesture::HoldBegin {
                    fingers: event.fingers(),
                },
                event.time_msec(),
            ),
            InputEvent::GestureHoldEnd { event } => self.gesture(
                Gesture::HoldEnd {
                    cancelled: event.cancelled(),
                },
                event.time_msec(),
            ),
            _ => return Ok(()),
        }
        self.flush()
    }

    pub fn scroll(&mut self, frame: AxisFrame) {
        self.state.note_activity();
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
        self.state.note_activity();
        let size = self.logical_output_size();
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
        self.state.note_activity();
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
                        if let Some(focusable) = state
                            .toplevel_of(&surface)
                            .or_else(|| state.focusable_layer_of(&surface))
                        {
                            state.focus(Some(focusable));
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
        self.state.note_activity();
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
                if let Some(key) = volume_key(keysym.modified_sym().raw()) {
                    state.held_shortcut_keys.push(evdev_code);
                    let fine = modifiers.alt && modifiers.shift;
                    return FilterResult::Intercept(Some(Shortcut::Volume(key, fine)));
                }
                if let Some(key) = brightness_key(keysym.modified_sym().raw()) {
                    state.held_shortcut_keys.push(evdev_code);
                    let fine = modifiers.alt && modifiers.shift;
                    return FilterResult::Intercept(Some(Shortcut::Brightness(key, fine)));
                }
                let unshifted = keysym
                    .raw_latin_sym_or_raw_current_sym()
                    .map(|sym| sym.raw());
                match shortcut(
                    keysym.modified_sym().raw(),
                    unshifted,
                    modifiers.ctrl,
                    modifiers.alt,
                    modifiers.shift,
                )
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
