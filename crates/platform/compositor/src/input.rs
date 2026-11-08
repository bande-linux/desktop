use std::io;

use bande_graphics::Modifiers;
use bande_keyboard::{Action, Area, Key, Modifiers as KeysHeld, bound};
use bande_protocols::server::bande_shell_v1::{
    BrightnessKey, LauncherMode, MediaKey, Panel, VolumeKey,
};
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, GestureBeginEvent,
    GestureEndEvent, GesturePinchUpdateEvent, GestureSwipeUpdateEvent, InputBackend, InputEvent,
    KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::input::keyboard::{FilterResult, Keycode, ModifiersState, keysyms};
use smithay::input::pointer::{AxisFrame, ButtonEvent, CursorIcon, MotionEvent};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Clock, Logical, Monotonic, Point, SERIAL_COUNTER};
use smithay::wayland::keyboard_shortcuts_inhibit::{
    KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
    KeyboardShortcutsInhibitorSeat,
};

use crate::frame::Target;
use crate::gesture::Gesture;
use crate::keyboard_monitor::KeyEvent;
use crate::state::{Compositor, State};
use crate::tile::Direction;

const EVDEV_OFFSET: u32 = 8;
const ABOVE_TAB: u32 = 41;

const SCROLL_PER_DETENT: f64 = 15.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shortcut {
    Bound(Action, Key),
    SwitcherKey(u32, bool),
    AssistKey(u32, bool),
    Volume(VolumeKey, bool),
    Brightness(BrightnessKey, bool),
    Media(MediaKey),
    OverviewKey(u32, Option<char>),
    CancelGrab,
    MenuKey(u32, Modifiers),
    ForceQuitKey(u32, Modifiers),
    AppKey(u32, Modifiers),
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

fn media_key(keysym: u32) -> Option<MediaKey> {
    match keysym {
        keysyms::KEY_XF86AudioPlay => Some(MediaKey::PlayPause),
        keysyms::KEY_XF86AudioPause => Some(MediaKey::Pause),
        keysyms::KEY_XF86AudioNext => Some(MediaKey::Next),
        keysyms::KEY_XF86AudioPrev => Some(MediaKey::Previous),
        _ => None,
    }
}

fn pressed_key(keysym: u32, unshifted: Option<u32>) -> Option<Key> {
    let base = unshifted.unwrap_or(keysym);
    let key = match base {
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => Key::Tab,
        keysyms::KEY_Escape => Key::Escape,
        keysyms::KEY_Print => Key::Print,
        keysyms::KEY_F1..=keysyms::KEY_F12 => {
            Key::Function(u8::try_from(base - keysyms::KEY_F1 + 1).ok()?)
        }
        keysyms::KEY_Left => Key::Left,
        keysyms::KEY_Right => Key::Right,
        keysyms::KEY_Up => Key::Up,
        keysyms::KEY_Down => Key::Down,
        keysyms::KEY_XF86LaunchA => Key::MissionControl,
        _ => Key::Character(
            char::from_u32(base)
                .filter(char::is_ascii_graphic)?
                .to_ascii_lowercase(),
        ),
    };
    Some(key)
}

fn direction(key: Key) -> Option<Direction> {
    match key {
        Key::Left => Some(Direction::Left),
        Key::Right => Some(Direction::Right),
        Key::Up => Some(Direction::Up),
        Key::Down => Some(Direction::Down),
        _ => None,
    }
}

fn offset(direction: Direction) -> (i32, i32) {
    match direction {
        Direction::Left => (-1, 0),
        Direction::Right => (1, 0),
        Direction::Up => (0, -1),
        Direction::Down => (0, 1),
    }
}

fn allowed(action: Action, locked: bool, inhibited: bool) -> bool {
    (!locked || action.area() == Area::Accessibility)
        && (!inhibited
            || matches!(
                action,
                Action::LockScreen | Action::ForceQuit | Action::RestartDesktop
            ))
        && (action != Action::ToggleScreenReader || crate::assistive::screen_reader_installed())
}

fn keys_held(modifiers: &ModifiersState) -> KeysHeld {
    KeysHeld {
        shift: modifiers.shift,
        ctrl: modifiers.ctrl,
        alt: modifiers.alt,
        logo: modifiers.logo,
    }
}

impl State {
    pub(crate) fn surface_under(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.pointer_target(location)?.surface()
    }

    pub(crate) fn refocus_pointer(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        if self
            .popup_grab
            .as_ref()
            .is_some_and(|grab| grab.has_ended())
        {
            self.return_from_menu();
        }
        if pointer.is_grabbed() && self.popup_grab.is_none() {
            return;
        }
        let under = self
            .surface_under(self.pointer_location)
            .filter(|(surface, _)| self.reachable_beside_menu(surface));
        if pointer.current_focus() == under.as_ref().map(|(surface, _)| surface.clone()) {
            return;
        }
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: self.pointer_location,
                serial: SERIAL_COUNTER.next_serial(),
                time: Clock::<Monotonic>::new().now().as_millis(),
            },
        );
        pointer.frame(self);
    }

    fn run(&mut self, shortcut: Shortcut) {
        match shortcut {
            Shortcut::Bound(action, key) => self.run_action(action, key),
            Shortcut::CancelGrab => self.cancel_grab(),
            Shortcut::MenuKey(keysym, modifiers) => self.window_menu_key(keysym, modifiers),
            Shortcut::ForceQuitKey(keysym, modifiers) => self.force_quit_key(keysym, modifiers),
            Shortcut::AppKey(keysym, modifiers) => self.send_app_key(keysym, modifiers),
            Shortcut::Volume(key, fine) => self.volume_key(key, fine),
            Shortcut::Brightness(key, fine) => self.brightness_key(key, fine),
            Shortcut::Media(key) => self.media_key(key),
            Shortcut::OverviewKey(keysym, text) => {
                self.close_menus();
                self.overview_key(keysym, text);
            }
            Shortcut::AssistKey(keysym, shift) => {
                self.close_menus();
                self.assist_key(keysym, shift);
            }
            Shortcut::SwitcherKey(keysym, shift) => {
                self.close_menus();
                self.dismiss_assist();
                self.switcher_key(keysym, shift);
            }
        }
    }

    fn run_action(&mut self, action: Action, key: Key) {
        bande_log::fault::action(&format!("{action:?}"));
        if !matches!(
            action,
            Action::TextConsole
                | Action::CaptureScreen
                | Action::ToggleZoom
                | Action::ZoomIn
                | Action::ZoomOut
        ) {
            self.close_menus();
        }
        if matches!(action.area(), Area::Windows | Area::Workspaces)
            || matches!(action, Action::NextApp | Action::PreviousApp)
        {
            self.dismiss_assist();
        }
        let focused = self
            .focused()
            .and_then(|focused| self.toplevel_of(&focused));
        match action {
            Action::TextConsole => {
                if let Key::Function(number) = key {
                    self.virtual_terminal = Some(i32::from(number));
                }
            }
            Action::ToggleZoom => self.toggle_magnifier(),
            Action::ZoomIn => self.step_magnifier(true),
            Action::ZoomOut => self.step_magnifier(false),
            Action::ToggleScreenReader => self.toggle_screen_reader(),
            Action::ShowLauncher => self.show_launcher(),
            Action::ShowClipboardHistory => self.open_launcher(LauncherMode::Clipboard),
            Action::ShowEmojiPicker => self.open_launcher(LauncherMode::Emoji),
            Action::OpenTerminal => self.open_terminal(),
            Action::CaptureScreen => self.capture_screen(),
            Action::CaptureArea => self.capture_area(),
            Action::CaptureOptions => self.show_capture_options(),
            Action::LockScreen => self.lock_screen_requested = !self.is_locked(),
            Action::ForceQuit | Action::RestartDesktop => self.open_force_quit(),
            Action::ShowNotificationCenter => self.toggle_panel(Panel::NotificationCenter),
            Action::ShowControlCenter => self.toggle_panel(Panel::ControlCenter),
            Action::NextApp => self.switch_apps(false),
            Action::PreviousApp => self.switch_apps(true),
            Action::Overview => self.toggle_overview(None),
            Action::ShowDesktop => self.toggle_desktop(),
            Action::PreviousWorkspace => self.step_workspace(false),
            Action::NextWorkspace => self.step_workspace(true),
            Action::CloseWindow => {
                if let Some(window) = self
                    .windows
                    .iter()
                    .find(|window| Some(window.toplevel.wl_surface()) == focused.as_ref())
                {
                    window.toplevel.send_close();
                }
            }
            Action::MinimizeWindow => {
                if let Some(focused) = focused {
                    self.minimize(&focused);
                }
            }
            Action::TileWindow => {
                if let (Some(focused), Some(direction)) = (focused, direction(key)) {
                    self.place_with_key(&focused, direction);
                }
            }
            Action::MoveWindowToDisplay => {
                if let (Some(focused), Some(direction)) = (focused, direction(key)) {
                    self.move_to_screen_beside(&focused, offset(direction));
                }
            }
        }
    }
}

impl Compositor {
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
                self.move_pointer_by(event.delta(), event.delta_unaccel(), event.time());
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let bounds = self.state.bounding_rect();
                let location = event.position_transformed(bounds.size) + bounds.loc.to_f64();
                self.move_pointer_to(location, event.time_msec());
            }
            InputEvent::PointerButton { event } => {
                let pressed = event.state() == ButtonState::Pressed;
                self.press_button(event.button_code(), pressed, event.time_msec());
            }
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    if source == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                        frame = frame.stop(axis);
                    } else if let Some(amount) = event.amount(axis) {
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
        if state.force_quit_scroll(&frame) {
            return;
        }
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
        let state = &mut self.state;
        let location = state.nearest_on_screens(location);
        let entered = state.screen_at(location) != state.screen_at(state.pointer_location);
        state.pointer_location = location;
        state.zoom.follow_pointer();
        state.cursor_shown = true;
        state.needs_redraw = true;
        if entered {
            state.reload_cursor();
        }

        if state.overview.is_open() {
            state.overview_pointer(location);
        }
        if state.switcher.is_shown() {
            state.switcher_pointer(location);
        }
        if state.assist.is_open() {
            state.assist_pointer(location);
        }
        if state.window_menu_pointer(location) || state.force_quit_pointer(location) {
            state.point_at_frame(None, false);
            if let Some(pointer) = state.seat.get_pointer() {
                pointer.motion(
                    state,
                    None,
                    &MotionEvent {
                        location,
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                    },
                );
                pointer.frame(state);
            }
            return;
        }
        state.follow_drag_with_window();
        let target = state.pointer_target(location);
        let under = target
            .as_ref()
            .and_then(Target::surface)
            .filter(|(surface, _)| state.reachable_beside_menu(surface));
        let Some(pointer) = state.seat.get_pointer() else {
            return;
        };
        if !pointer.is_grabbed() {
            let pointed = target.as_ref().and_then(Target::frame);
            state.point_at_frame(pointed, under.is_some());
            let under_client = under
                .as_ref()
                .and_then(|(surface, _)| surface.client())
                .map(|client| client.id());
            if state.cursor_owner.is_some() && state.cursor_owner != under_client {
                state.show_cursor(CursorIcon::Default);
            }
        }
        state.drag_over(under.as_ref().map(|(surface, _)| surface));
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
        state.reload_cursor();
    }

    pub fn press_button(&mut self, button: u32, pressed: bool, time: u32) {
        self.state.note_activity();
        self.state.super_tap = false;
        let state = &mut self.state;
        if state.window_menu_button(button, pressed) || state.force_quit_button(button, pressed) {
            return;
        }
        if !pressed && state.release_frame(button) {
            return;
        }
        if !pressed && state.swallowed_button == Some(button) {
            state.swallowed_button = None;
            return;
        }
        if pressed && (state.switcher_click() || state.assist_click()) {
            state.swallowed_button = Some(button);
            return;
        }
        let overview_button = state.overview.is_open() && state.popup_grab.is_none();
        let target = (pressed || overview_button)
            .then(|| state.pointer_target(state.pointer_location))
            .flatten();
        let under = target.as_ref().and_then(Target::surface);
        let overview_release = !pressed && state.overview.is_pressed();
        if overview_release
            || overview_button
                && !under
                    .as_ref()
                    .is_some_and(|(surface, _)| state.is_above_windows(surface))
        {
            state.overview_button(button, pressed);
            return;
        }
        if pressed {
            if state.dismisses_popups(under.as_ref().map(|(surface, _)| surface)) {
                state.swallowed_button = Some(button);
                return;
            }
            state.press_outside(under.as_ref().map(|(surface, _)| surface));
            if state.popup_grab.is_none() {
                match target.as_ref().and_then(Target::frame) {
                    Some(pointed) => {
                        if state.press_frame(pointed, button, time) {
                            return;
                        }
                    }
                    None => match under {
                        Some((surface, _)) => {
                            if let Some(dialog) = state
                                .toplevel_of(&surface)
                                .and_then(|window| state.modal_blocker(&window))
                            {
                                state.focus(Some(dialog.clone()));
                                state.nudge(&dialog);
                                state.swallowed_button = Some(button);
                                return;
                            }
                            let toplevel = state.toplevel_of(&surface);
                            if let Some(focusable) = toplevel
                                .clone()
                                .or_else(|| state.focusable_layer_of(&surface))
                            {
                                state.focus(Some(focusable));
                            }
                            if let Some(toplevel) = toplevel {
                                state.drag_from_anywhere(&toplevel, button);
                            }
                        }
                        None => {
                            state.windowless_app = None;
                            state.focus(None);
                        }
                    },
                }
            }
        }
        let Some(pointer) = state.seat.get_pointer() else {
            return;
        };
        if !pressed {
            state.drop_window_drag_on_nothing();
        }
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
}

impl KeyboardShortcutsInhibitHandler for State {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.globals.shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        inhibitor.activate();
    }
}

smithay::delegate_keyboard_shortcuts_inhibit!(State);

impl State {
    fn shortcuts_inhibited(&self) -> bool {
        self.seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .and_then(|focus| self.seat.keyboard_shortcuts_inhibitor_for_surface(&focus))
            .is_some_and(|inhibitor| inhibitor.is_active())
    }

    pub(crate) fn deliver_key(&mut self, evdev_code: u32, pressed: bool, time: u32) {
        let state = self;
        let Some(keyboard) = state.seat.get_keyboard() else {
            return;
        };
        let key_state = if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        };
        let held_by_input_method = keyboard
            .grab_start_data()
            .is_some_and(|grab| grab.focus.is_none());
        if keyboard.is_grabbed()
            && (state.is_locked()
                || (held_by_input_method && state.input_method_must_not_see_keys()))
        {
            keyboard.unset_grab(state);
        }
        let before = keyboard.modifier_state();
        let action = keyboard.input(
            state,
            Keycode::new(evdev_code + EVDEV_OFFSET),
            key_state,
            SERIAL_COUNTER.next_serial(),
            time,
            |state, modifiers, keysym| {
                let serialized = modifiers.serialized;
                let monitored = KeyEvent {
                    released: !pressed,
                    state: serialized.depressed | serialized.latched | serialized.locked,
                    keysym: keysym.modified_sym().raw(),
                    unichar: keysym.modified_sym().key_char().map_or(0, u32::from),
                    keycode: u16::try_from(evdev_code + EVDEV_OFFSET).unwrap_or_default(),
                };
                if state.monitor_key(monitored) {
                    return FilterResult::Intercept(None);
                }
                let super_key = matches!(
                    keysym.modified_sym().raw(),
                    keysyms::KEY_Super_L | keysyms::KEY_Super_R
                );
                if pressed {
                    state.super_tap = super_key
                        && !(modifiers.ctrl || modifiers.alt || modifiers.shift)
                        && keyboard.pressed_keys().len() == 1;
                } else if !super_key {
                    state.super_tap = false;
                }
                if !pressed {
                    return if state.held_shortcut_keys.contains(&evdev_code) {
                        state.held_shortcut_keys.retain(|held| *held != evdev_code);
                        FilterResult::Intercept(None)
                    } else {
                        FilterResult::Forward
                    };
                }
                let binding = pressed_key(
                    keysym.modified_sym().raw(),
                    keysym
                        .raw_latin_sym_or_raw_current_sym()
                        .map(|sym| sym.raw()),
                )
                .and_then(|key| Some((bound(keys_held(modifiers), key)?, key)));
                if let Some((Action::TextConsole, key)) = binding {
                    state.held_shortcut_keys.push(evdev_code);
                    return FilterResult::Intercept(Some(Shortcut::Bound(
                        Action::TextConsole,
                        key,
                    )));
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
                if let Some(key) = media_key(keysym.modified_sym().raw()) {
                    state.held_shortcut_keys.push(evdev_code);
                    return FilterResult::Intercept(Some(Shortcut::Media(key)));
                }
                let sym = keysym.modified_sym();
                if sym.raw() == keysyms::KEY_Escape && state.holds_cancellable_grab() {
                    state.held_shortcut_keys.push(evdev_code);
                    return FilterResult::Intercept(Some(Shortcut::CancelGrab));
                }
                if state.window_menus.takes_key(sym.raw())
                    && !modifiers.logo
                    && !sym.is_modifier_key()
                {
                    state.held_shortcut_keys.push(evdev_code);
                    return FilterResult::Intercept(Some(Shortcut::MenuKey(
                        sym.raw(),
                        Modifiers {
                            shift: modifiers.shift,
                            alt: modifiers.alt,
                            control: modifiers.ctrl,
                            logo: modifiers.logo,
                        },
                    )));
                }
                if state.force_quit.is_open() && !modifiers.logo && !sym.is_modifier_key() {
                    state.held_shortcut_keys.push(evdev_code);
                    return FilterResult::Intercept(Some(Shortcut::ForceQuitKey(
                        sym.raw(),
                        Modifiers {
                            shift: modifiers.shift,
                            alt: modifiers.alt,
                            control: modifiers.ctrl,
                            logo: modifiers.logo,
                        },
                    )));
                }
                if state.switcher.is_active() && !sym.is_modifier_key() {
                    state.held_shortcut_keys.push(evdev_code);
                    let pressed = if evdev_code == ABOVE_TAB {
                        keysyms::KEY_grave
                    } else {
                        sym.raw()
                    };
                    return FilterResult::Intercept(Some(Shortcut::SwitcherKey(
                        pressed,
                        modifiers.shift,
                    )));
                }
                if state.assist.is_open() && !modifiers.logo && !sym.is_modifier_key() {
                    state.held_shortcut_keys.push(evdev_code);
                    return FilterResult::Intercept(Some(Shortcut::AssistKey(
                        sym.raw(),
                        modifiers.shift,
                    )));
                }
                if let Some((action, key)) = binding.filter(|(action, _)| {
                    allowed(*action, state.is_locked(), state.shortcuts_inhibited())
                }) {
                    state.held_shortcut_keys.push(evdev_code);
                    return FilterResult::Intercept(Some(Shortcut::Bound(action, key)));
                }
                if state.overview.is_open() && !modifiers.logo {
                    state.held_shortcut_keys.push(evdev_code);
                    return FilterResult::Intercept(Some(Shortcut::OverviewKey(
                        sym.raw(),
                        sym.key_char(),
                    )));
                }
                if !sym.is_modifier_key() && state.focused().is_none() && state.kept_app_is_active()
                {
                    state.held_shortcut_keys.push(evdev_code);
                    return FilterResult::Intercept(Some(Shortcut::AppKey(
                        sym.raw(),
                        Modifiers {
                            shift: modifiers.shift,
                            alt: modifiers.alt,
                            control: modifiers.ctrl,
                            logo: modifiers.logo,
                        },
                    )));
                }
                if !sym.is_modifier_key() {
                    state.note_typing();
                }
                FilterResult::Forward
            },
        );
        if let Some(Some(shortcut)) = action {
            state.run(shortcut);
        }
        if !pressed
            && std::mem::take(&mut state.super_tap)
            && let Some(action) = bound(
                KeysHeld {
                    logo: true,
                    ..KeysHeld::default()
                },
                Key::Tap,
            )
            .filter(|action| allowed(*action, state.is_locked(), state.shortcuts_inhibited()))
        {
            state.run(Shortcut::Bound(action, Key::Tap));
        }
        let after = keyboard.modifier_state();
        if after.alt != before.alt && state.has_compositor_controls() {
            state.needs_redraw = true;
        }
        if (after.ctrl, after.shift) != (before.ctrl, before.shift) {
            state.renegotiate_drag();
        }
        let held = keyboard.modifier_state();
        if state.switcher.is_active() && !held.logo && !held.alt {
            state.commit_switcher();
        }
    }
}
