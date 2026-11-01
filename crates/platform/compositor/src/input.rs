use std::io;

use bande_graphics::Modifiers;
use bande_protocols::server::bande_shell_v1::{
    BrightnessKey, LauncherMode, MediaKey, Panel, VolumeKey,
};
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, GestureBeginEvent,
    GestureEndEvent, GesturePinchUpdateEvent, GestureSwipeUpdateEvent, InputBackend, InputEvent,
    KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::input::keyboard::{FilterResult, Keycode, keysyms};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Clock, Logical, Monotonic, Point, SERIAL_COUNTER};
use smithay::wayland::keyboard_shortcuts_inhibit::{
    KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
    KeyboardShortcutsInhibitorSeat,
};

use crate::frame::Target;
use crate::gesture::Gesture;
use crate::keyboard_monitor::KeyEvent;
use crate::motion::TASKBAR_NAMESPACE;
use crate::state::{Compositor, MENU_BAR_NAMESPACE, MENU_SEARCH_NAMESPACE, State};
use crate::tile::Direction;

const EVDEV_OFFSET: u32 = 8;
const ABOVE_TAB: u32 = 41;

const SCROLL_PER_DETENT: f64 = 15.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shortcut {
    CloseWindow,
    NextApp { backwards: bool },
    NextWindow { backwards: bool },
    SwitcherKey(u32, bool),
    AssistKey(u32, bool),
    ShowLauncher,
    OpenLauncher(LauncherMode),
    MinimizeWindow,
    LockScreen,
    ForceQuit,
    FocusShell(&'static str),
    ToggleFullScreen,
    Place(Direction),
    CaptureScreen,
    CaptureArea,
    CaptureOptions,
    Volume(VolumeKey, bool),
    Brightness(BrightnessKey, bool),
    Media(MediaKey),
    Workspace(bool),
    WorkspaceAt(usize),
    CarryWindow(bool),
    MoveToDisplay(i32, i32),
    Overview,
    AppWindows,
    OverviewKey(u32, Option<char>),
    InputSource { backwards: bool },
    ToggleZoom,
    Zoom { closer: bool },
    ToggleScreenReader,
    ShowDesktop,
    TogglePanel(Panel),
    CancelGrab,
    MenuKey(u32, Modifiers),
    ForceQuitKey(u32, Modifiers),
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

fn without_super(keysym: u32, ctrl: bool, alt: bool, shift: bool) -> Option<Shortcut> {
    match keysym {
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab if alt && !ctrl => {
            Some(Shortcut::NextApp { backwards: shift })
        }
        keysyms::KEY_XF86LaunchA => Some(Shortcut::Overview),
        keysyms::KEY_Print if shift => Some(Shortcut::CaptureScreen),
        keysyms::KEY_Print => Some(Shortcut::CaptureOptions),
        keysyms::KEY_F4 if alt => Some(Shortcut::CloseWindow),
        keysyms::KEY_Escape if ctrl && alt => Some(Shortcut::ForceQuit),
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
    match unshifted {
        Some(keysyms::KEY_8) if alt && !ctrl => return Some(Shortcut::ToggleZoom),
        Some(keysyms::KEY_equal | keysyms::KEY_plus) if alt && !ctrl => {
            return Some(Shortcut::Zoom { closer: true });
        }
        Some(keysyms::KEY_minus) if alt && !ctrl => {
            return Some(Shortcut::Zoom { closer: false });
        }
        Some(keysyms::KEY_s) if alt && !ctrl => return Some(Shortcut::ToggleScreenReader),
        Some(digit @ keysyms::KEY_1..=keysyms::KEY_9) if ctrl && !alt => {
            return Some(Shortcut::WorkspaceAt((digit - keysyms::KEY_1) as usize));
        }
        Some(keysyms::KEY_s) if shift && !ctrl && !alt => return Some(Shortcut::CaptureArea),
        Some(keysyms::KEY_slash) if shift && !ctrl && !alt => {
            return Some(Shortcut::FocusShell(MENU_SEARCH_NAMESPACE));
        }
        Some(keysyms::KEY_grave) if !ctrl && !alt => {
            return Some(Shortcut::NextWindow { backwards: shift });
        }
        _ if ctrl || alt || shift => {}
        Some(keysyms::KEY_a) => return Some(Shortcut::ShowLauncher),
        Some(keysyms::KEY_d) => return Some(Shortcut::ShowDesktop),
        Some(keysyms::KEY_h) => return Some(Shortcut::MinimizeWindow),
        Some(keysyms::KEY_l) => return Some(Shortcut::LockScreen),
        Some(keysyms::KEY_n) => return Some(Shortcut::TogglePanel(Panel::NotificationCenter)),
        Some(keysyms::KEY_s) => return Some(Shortcut::TogglePanel(Panel::ControlCenter)),
        Some(keysyms::KEY_v) => return Some(Shortcut::OpenLauncher(LauncherMode::Clipboard)),
        Some(keysyms::KEY_w) => return Some(Shortcut::Overview),
        Some(keysyms::KEY_period) => return Some(Shortcut::OpenLauncher(LauncherMode::Emoji)),
        _ => {}
    }
    match keysym {
        keysyms::KEY_F2 => Some(Shortcut::FocusShell(MENU_BAR_NAMESPACE)),
        keysyms::KEY_F3 => Some(Shortcut::FocusShell(TASKBAR_NAMESPACE)),
        keysyms::KEY_F11 => Some(Shortcut::ToggleFullScreen),
        keysyms::KEY_XF86LaunchA => Some(Shortcut::ShowDesktop),
        keysyms::KEY_Left if ctrl && shift => Some(Shortcut::CarryWindow(false)),
        keysyms::KEY_Right if ctrl && shift => Some(Shortcut::CarryWindow(true)),
        keysyms::KEY_Left if ctrl && !alt => Some(Shortcut::Workspace(false)),
        keysyms::KEY_Right if ctrl && !alt => Some(Shortcut::Workspace(true)),
        keysyms::KEY_Left if shift && !alt => Some(Shortcut::MoveToDisplay(-1, 0)),
        keysyms::KEY_Right if shift && !alt => Some(Shortcut::MoveToDisplay(1, 0)),
        keysyms::KEY_Up if shift && !alt => Some(Shortcut::MoveToDisplay(0, -1)),
        keysyms::KEY_Down if shift && !alt => Some(Shortcut::MoveToDisplay(0, 1)),
        keysyms::KEY_Down if alt => Some(Shortcut::AppWindows),
        keysyms::KEY_Left if !alt => Some(Shortcut::Place(Direction::Left)),
        keysyms::KEY_Right if !alt => Some(Shortcut::Place(Direction::Right)),
        keysyms::KEY_Up if !alt => Some(Shortcut::Place(Direction::Up)),
        keysyms::KEY_Down if !alt => Some(Shortcut::Place(Direction::Down)),
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab if !ctrl && !alt => {
            Some(Shortcut::NextApp { backwards: shift })
        }
        keysyms::KEY_space if alt && !ctrl => Some(Shortcut::InputSource { backwards: shift }),
        keysyms::KEY_space if !alt => Some(Shortcut::ShowLauncher),
        _ => None,
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
        if shortcut == Shortcut::CancelGrab {
            self.cancel_grab();
            return;
        }
        if let Shortcut::MenuKey(keysym, modifiers) = shortcut {
            self.window_menu_key(keysym, modifiers);
            return;
        }
        if let Shortcut::ForceQuitKey(keysym, modifiers) = shortcut {
            self.force_quit_key(keysym, modifiers);
            return;
        }
        if let Shortcut::Volume(key, fine) = shortcut {
            self.volume_key(key, fine);
            return;
        }
        if let Shortcut::Brightness(key, fine) = shortcut {
            self.brightness_key(key, fine);
            return;
        }
        if let Shortcut::Media(key) = shortcut {
            self.media_key(key);
            return;
        }
        if !matches!(
            shortcut,
            Shortcut::InputSource { .. }
                | Shortcut::CaptureScreen
                | Shortcut::ToggleZoom
                | Shortcut::Zoom { .. }
        ) {
            self.close_menus();
        }
        match shortcut {
            Shortcut::ToggleZoom => return self.toggle_magnifier(),
            Shortcut::Zoom { closer } => return self.step_magnifier(closer),
            Shortcut::ToggleScreenReader => return self.toggle_screen_reader(),
            _ => {}
        }
        if shortcut == Shortcut::ShowLauncher {
            self.show_launcher();
            return;
        }
        if let Shortcut::OpenLauncher(mode) = shortcut {
            self.open_launcher(mode);
            return;
        }
        if let Shortcut::InputSource { backwards } = shortcut {
            self.next_input_source(backwards);
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
        if shortcut == Shortcut::CaptureOptions {
            self.show_capture_options();
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
        if let Shortcut::TogglePanel(panel) = shortcut {
            self.toggle_panel(panel);
            return;
        }
        if let Shortcut::OverviewKey(keysym, text) = shortcut {
            self.overview_key(keysym, text);
            return;
        }
        if let Shortcut::AssistKey(keysym, shift) = shortcut {
            self.assist_key(keysym, shift);
            return;
        }
        self.dismiss_assist();
        match shortcut {
            Shortcut::NextApp { backwards } => return self.switch_apps(backwards),
            Shortcut::NextWindow { backwards } => return self.cycle_windows(backwards),
            Shortcut::SwitcherKey(keysym, shift) => return self.switcher_key(keysym, shift),
            _ => {}
        }
        if shortcut == Shortcut::Overview {
            self.toggle_overview(None);
            return;
        }
        if shortcut == Shortcut::AppWindows {
            self.show_app_windows();
            return;
        }
        if shortcut == Shortcut::ShowDesktop {
            self.toggle_desktop();
            return;
        }
        if let Shortcut::Workspace(forward) = shortcut {
            self.step_workspace(forward);
            return;
        }
        if let Shortcut::WorkspaceAt(index) = shortcut {
            let screen = self.pointer_screen();
            if index < self.screens[screen].workspaces.count() {
                self.switch_workspace(screen, index);
            }
            return;
        }
        if shortcut == Shortcut::ForceQuit {
            self.open_force_quit();
            return;
        }
        let Some(focused) = self
            .focused()
            .and_then(|focused| self.toplevel_of(&focused))
        else {
            return;
        };
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
            Shortcut::MinimizeWindow => self.minimize(&focused),
            Shortcut::Place(direction) => self.place_with_key(&focused, direction),
            Shortcut::CarryWindow(forward) => self.move_to_workspace(&focused, forward, true),
            Shortcut::MoveToDisplay(dx, dy) => self.move_to_screen_beside(&focused, (dx, dy)),
            Shortcut::ToggleFullScreen => self.toggle_fullscreen(&focused),
            Shortcut::ShowLauncher
            | Shortcut::OpenLauncher(_)
            | Shortcut::LockScreen
            | Shortcut::ForceQuit
            | Shortcut::FocusShell(_)
            | Shortcut::CaptureScreen
            | Shortcut::CaptureArea
            | Shortcut::CaptureOptions
            | Shortcut::Volume(..)
            | Shortcut::Brightness(..)
            | Shortcut::Media(_)
            | Shortcut::Workspace(_)
            | Shortcut::WorkspaceAt(_)
            | Shortcut::Overview
            | Shortcut::AppWindows
            | Shortcut::OverviewKey(..)
            | Shortcut::ToggleZoom
            | Shortcut::Zoom { .. }
            | Shortcut::ToggleScreenReader
            | Shortcut::ShowDesktop
            | Shortcut::TogglePanel(_)
            | Shortcut::NextApp { .. }
            | Shortcut::NextWindow { .. }
            | Shortcut::SwitcherKey(..)
            | Shortcut::AssistKey(..)
            | Shortcut::InputSource { .. }
            | Shortcut::CancelGrab
            | Shortcut::MenuKey(..)
            | Shortcut::ForceQuitKey(..) => {}
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
            state.point_at_frame(location, pointed, under.is_some());
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
        if overview_button
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
            if let Some(Target::Scrim(owner)) = &target {
                state.scrim_pressed(owner);
                state.swallowed_button = Some(button);
                return;
            }
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
                        None => state.focus(None),
                    },
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
        if state.is_locked() && keyboard.is_grabbed() {
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
                if !(modifiers.logo && modifiers.alt) {
                    state.input_sources.end_cycle();
                }
                let super_key = matches!(
                    keysym.modified_sym().raw(),
                    keysyms::KEY_Super_L | keysyms::KEY_Super_R
                );
                if pressed {
                    state.super_tap =
                        super_key && !(modifiers.ctrl || modifiers.alt || modifiers.shift);
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
                if state.window_menus.is_open() && !modifiers.logo && !sym.is_modifier_key() {
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
                if state.force_quit.is_open()
                    && !modifiers.logo
                    && !sym.is_modifier_key()
                    && !(sym.raw() == keysyms::KEY_Escape && modifiers.ctrl && modifiers.alt)
                {
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
                let inhibited = state.shortcuts_inhibited();
                if let Some(shortcut) =
                    without_super(sym.raw(), modifiers.ctrl, modifiers.alt, modifiers.shift).filter(
                        |shortcut| {
                            !modifiers.logo
                                && !state.is_locked()
                                && (!inhibited || *shortcut == Shortcut::ForceQuit)
                        },
                    )
                {
                    state.held_shortcut_keys.push(evdev_code);
                    return FilterResult::Intercept(Some(shortcut));
                }
                if state.overview.is_open() && !modifiers.logo {
                    state.held_shortcut_keys.push(evdev_code);
                    return FilterResult::Intercept(Some(Shortcut::OverviewKey(
                        sym.raw(),
                        sym.key_char(),
                    )));
                }
                let unshifted = if evdev_code == ABOVE_TAB {
                    Some(keysyms::KEY_grave)
                } else {
                    keysym
                        .raw_latin_sym_or_raw_current_sym()
                        .map(|sym| sym.raw())
                };
                match shortcut(
                    keysym.modified_sym().raw(),
                    unshifted,
                    modifiers.ctrl,
                    modifiers.alt,
                    modifiers.shift,
                )
                .filter(|shortcut| {
                    modifiers.logo
                        && (!inhibited
                            || matches!(shortcut, Shortcut::ForceQuit | Shortcut::LockScreen))
                        && (*shortcut != Shortcut::ToggleScreenReader
                            || crate::assistive::screen_reader_installed())
                        && (!state.is_locked()
                            || matches!(
                                shortcut,
                                Shortcut::InputSource { .. }
                                    | Shortcut::ToggleZoom
                                    | Shortcut::Zoom { .. }
                                    | Shortcut::ToggleScreenReader
                            ))
                }) {
                    Some(shortcut) => {
                        state.held_shortcut_keys.push(evdev_code);
                        FilterResult::Intercept(Some(shortcut))
                    }
                    None => {
                        if !keysym.modified_sym().is_modifier_key() {
                            state.note_typing();
                        }
                        FilterResult::Forward
                    }
                }
            },
        );
        if let Some(Some(shortcut)) = action {
            state.run(shortcut);
        }
        if !pressed
            && std::mem::take(&mut state.super_tap)
            && !state.is_locked()
            && !state.shortcuts_inhibited()
        {
            state.run(Shortcut::ShowLauncher);
        }
        let after = keyboard.modifier_state();
        if after.alt != before.alt && state.has_server_side_frames() {
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

#[cfg(test)]
mod tests {
    use bande_keyboard::{Action, SHORTCUTS};
    use bande_protocols::server::bande_shell_v1::{LauncherMode, Panel};
    use smithay::input::keyboard::keysyms;

    use super::{Shortcut, shortcut, without_super};
    use crate::motion::TASKBAR_NAMESPACE;
    use crate::state::{MENU_BAR_NAMESPACE, MENU_SEARCH_NAMESPACE};
    use crate::tile::Direction;

    fn keysym(key: &str) -> u32 {
        match key {
            "Space" => keysyms::KEY_space,
            "Tab" => keysyms::KEY_Tab,
            "Escape" => keysyms::KEY_Escape,
            "." => keysyms::KEY_period,
            "Left" => keysyms::KEY_Left,
            "Right" => keysyms::KEY_Right,
            "Up" => keysyms::KEY_Up,
            "Down" => keysyms::KEY_Down,
            "F2" => keysyms::KEY_F2,
            "F3" => keysyms::KEY_F3,
            "F4" => keysyms::KEY_F4,
            "Print" => keysyms::KEY_Print,
            "F11" => keysyms::KEY_F11,
            "Mission Control" => keysyms::KEY_XF86LaunchA,
            single => single
                .chars()
                .next()
                .map_or(0, |character| character.to_ascii_lowercase() as u32),
        }
    }

    fn expected(action: Action) -> Shortcut {
        match action {
            Action::CloseWindow => Shortcut::CloseWindow,
            Action::MinimizeWindow => Shortcut::MinimizeWindow,
            Action::ToggleFullScreen => Shortcut::ToggleFullScreen,
            Action::TileWindow => Shortcut::Place(Direction::Left),
            Action::NextApp => Shortcut::NextApp { backwards: false },
            Action::NextWindow => Shortcut::NextWindow { backwards: false },
            Action::PreviousWindow => Shortcut::NextWindow { backwards: true },
            Action::ForceQuit => Shortcut::ForceQuit,
            Action::PreviousWorkspace => Shortcut::Workspace(false),
            Action::NextWorkspace => Shortcut::Workspace(true),
            Action::WorkspaceByNumber => Shortcut::WorkspaceAt(0),
            Action::CarryWindowLeft => Shortcut::CarryWindow(false),
            Action::CarryWindowRight => Shortcut::CarryWindow(true),
            Action::MoveWindowToDisplay => Shortcut::MoveToDisplay(1, 0),
            Action::Overview => Shortcut::Overview,
            Action::AppWindows => Shortcut::AppWindows,
            Action::NextInputSource => Shortcut::InputSource { backwards: false },
            Action::PreviousInputSource => Shortcut::InputSource { backwards: true },
            Action::CaptureScreen => Shortcut::CaptureScreen,
            Action::CaptureArea => Shortcut::CaptureArea,
            Action::CaptureOptions => Shortcut::CaptureOptions,
            Action::ShowDesktop => Shortcut::ShowDesktop,
            Action::ShowNotificationCenter => Shortcut::TogglePanel(Panel::NotificationCenter),
            Action::ShowControlCenter => Shortcut::TogglePanel(Panel::ControlCenter),
            Action::ShowLauncher => Shortcut::ShowLauncher,
            Action::ShowClipboardHistory => Shortcut::OpenLauncher(LauncherMode::Clipboard),
            Action::ShowEmojiPicker => Shortcut::OpenLauncher(LauncherMode::Emoji),
            Action::FocusMenuBar => Shortcut::FocusShell(MENU_BAR_NAMESPACE),
            Action::FocusTaskbar => Shortcut::FocusShell(TASKBAR_NAMESPACE),
            Action::SearchMenus => Shortcut::FocusShell(MENU_SEARCH_NAMESPACE),
            Action::LockScreen => Shortcut::LockScreen,
            Action::ToggleZoom => Shortcut::ToggleZoom,
            Action::ZoomIn => Shortcut::Zoom { closer: true },
            Action::ZoomOut => Shortcut::Zoom { closer: false },
            Action::ToggleScreenReader => Shortcut::ToggleScreenReader,
        }
    }

    #[test]
    fn every_shortcut_listed_in_settings_runs_its_action() {
        for listed in SHORTCUTS {
            let parts: Vec<&str> = listed.keys.split('+').collect();
            let held = |name: &str| parts.contains(&name);
            let key = keysym(parts.last().copied().unwrap_or_default());
            let runs = if held("Super") {
                shortcut(key, Some(key), held("Ctrl"), held("Alt"), held("Shift"))
            } else {
                without_super(key, held("Ctrl"), held("Alt"), held("Shift"))
            };
            assert_eq!(runs, Some(expected(listed.action)), "{}", listed.keys);
        }
    }
}
