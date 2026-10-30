use std::io;

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, ButtonState, Event, GestureBeginEvent, GestureEndEvent,
    GesturePinchUpdateEvent, GestureSwipeUpdateEvent, InputBackend, InputEvent, KeyState,
    KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::input::keyboard::{FilterResult, Keycode, keysyms};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Clock, Logical, Monotonic, Point, SERIAL_COUNTER};
use unbound_protocols::server::unbound_shell_v1::{
    BrightnessKey, LauncherMode, MediaKey, VolumeKey,
};

use crate::frame::Target;
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
    OpenLauncher(LauncherMode),
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
    Media(MediaKey),
    HideApp,
    HideOthers,
    Workspace(bool),
    WorkspaceAt(usize),
    CarryWindow(bool),
    MoveToDisplay(i32, i32),
    Overview,
    AppWindows,
    OverviewKey(u32, Option<char>),
    InputSource { backwards: bool },
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
        Some(digit @ keysyms::KEY_1..=keysyms::KEY_9) if alt && !ctrl => {
            return Some(Shortcut::WorkspaceAt((digit - keysyms::KEY_1) as usize));
        }
        Some(keysyms::KEY_h) if alt => return Some(Shortcut::HideOthers),
        Some(keysyms::KEY_h) if !ctrl => return Some(Shortcut::HideApp),
        Some(keysyms::KEY_v) if shift && !ctrl && !alt => {
            return Some(Shortcut::OpenLauncher(LauncherMode::Clipboard));
        }
        _ => {}
    }
    match keysym {
        keysyms::KEY_Escape if alt => Some(Shortcut::ForceQuit),
        keysyms::KEY_F2 => Some(Shortcut::FocusShell(MENU_BAR_NAMESPACE)),
        keysyms::KEY_F3 => Some(Shortcut::FocusShell(DOCK_NAMESPACE)),
        keysyms::KEY_f | keysyms::KEY_F if ctrl => Some(Shortcut::ToggleFullscreen),
        keysyms::KEY_Left if ctrl && shift => Some(Shortcut::CarryWindow(false)),
        keysyms::KEY_Right if ctrl && shift => Some(Shortcut::CarryWindow(true)),
        keysyms::KEY_Left if alt && shift => Some(Shortcut::MoveToDisplay(-1, 0)),
        keysyms::KEY_Right if alt && shift => Some(Shortcut::MoveToDisplay(1, 0)),
        keysyms::KEY_Up if alt && shift => Some(Shortcut::MoveToDisplay(0, -1)),
        keysyms::KEY_Down if alt && shift => Some(Shortcut::MoveToDisplay(0, 1)),
        keysyms::KEY_Left if alt => Some(Shortcut::Workspace(false)),
        keysyms::KEY_Right if alt => Some(Shortcut::Workspace(true)),
        keysyms::KEY_Up if alt => Some(Shortcut::Overview),
        keysyms::KEY_Down if alt => Some(Shortcut::AppWindows),
        keysyms::KEY_Left if ctrl => Some(Shortcut::Place(Direction::Left)),
        keysyms::KEY_Right if ctrl => Some(Shortcut::Place(Direction::Right)),
        keysyms::KEY_Up if ctrl => Some(Shortcut::Place(Direction::Up)),
        keysyms::KEY_Down if ctrl => Some(Shortcut::Place(Direction::Down)),
        keysyms::KEY_q | keysyms::KEY_Q if ctrl => Some(Shortcut::LockScreen),
        keysyms::KEY_m | keysyms::KEY_M => Some(Shortcut::MinimizeWindow),
        keysyms::KEY_w | keysyms::KEY_W => Some(Shortcut::CloseWindow),
        keysyms::KEY_q | keysyms::KEY_Q => Some(Shortcut::QuitApp),
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => Some(Shortcut::NextApp),
        keysyms::KEY_space if alt && !ctrl => Some(Shortcut::InputSource { backwards: shift }),
        keysyms::KEY_space if ctrl && !alt => Some(Shortcut::OpenLauncher(LauncherMode::Emoji)),
        keysyms::KEY_space => Some(Shortcut::ShowLauncher),
        _ => None,
    }
}

impl State {
    fn surface_under(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        match self.pointer_target(location)? {
            Target::Surface(surface, at) => Some((surface, at)),
            Target::Frame(..) => None,
        }
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
        if let Shortcut::Media(key) = shortcut {
            self.media_key(key);
            return;
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
        if shortcut == Shortcut::LockScreen {
            self.lock_screen_requested = !self.is_locked();
            return;
        }
        if let Shortcut::FocusShell(namespace) = shortcut {
            self.focus_shell(namespace);
            return;
        }
        if let Shortcut::OverviewKey(keysym, text) = shortcut {
            self.overview_key(keysym, text);
            return;
        }
        if shortcut == Shortcut::Overview {
            self.toggle_overview(None);
            return;
        }
        if shortcut == Shortcut::AppWindows {
            self.show_app_windows();
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
            self.force_quit_requested = true;
            self.watchdog.ping_everyone();
            return;
        }
        let Some(focused) = self.focused() else {
            if shortcut == Shortcut::QuitApp
                && let Some((app, app_id)) = self.windowless_app.clone()
            {
                self.quit_app(Some(app));
                if !app_id.is_empty() {
                    self.quit_app_named(&app_id);
                }
            }
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
            Shortcut::QuitApp => {
                let app_id = toplevel_names(&focused).1;
                if app_id.is_empty() {
                    self.quit_app(owner);
                } else {
                    self.quit_app_named(&app_id);
                }
            }
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
            Shortcut::CarryWindow(forward) => {
                if let Some(window) = self.toplevel_of(&focused) {
                    self.move_to_workspace(&window, forward, true);
                }
            }
            Shortcut::MoveToDisplay(dx, dy) => {
                if let Some(window) = self.toplevel_of(&focused) {
                    self.move_to_screen_beside(&window, (dx, dy));
                }
            }
            Shortcut::ToggleFullscreen => {
                if let Some(window) = self.toplevel_of(&focused) {
                    self.toggle_fullscreen(&window);
                }
            }
            Shortcut::ShowLauncher
            | Shortcut::OpenLauncher(_)
            | Shortcut::LockScreen
            | Shortcut::ForceQuit
            | Shortcut::FocusShell(_)
            | Shortcut::CaptureScreen
            | Shortcut::CaptureArea
            | Shortcut::Volume(..)
            | Shortcut::Brightness(..)
            | Shortcut::Media(_)
            | Shortcut::Workspace(_)
            | Shortcut::WorkspaceAt(_)
            | Shortcut::Overview
            | Shortcut::AppWindows
            | Shortcut::OverviewKey(..)
            | Shortcut::InputSource { .. } => {}
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
                let bounds = self.state.bounding_rect();
                let location = event.position_transformed(bounds.size) + bounds.location.to_f64();
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
        let state = &mut self.state;
        let location = state.nearest_on_screens(location);
        let entered = state.screen_at(location) != state.screen_at(state.pointer_location);
        state.pointer_location = location;
        state.cursor_shown = true;
        state.needs_redraw = true;
        if entered {
            state.reload_cursor();
        }

        if state.overview.is_open() {
            state.overview_pointer(location);
        }
        let under = state.surface_under(location);
        let Some(pointer) = state.seat.get_pointer() else {
            return;
        };
        if !pointer.is_grabbed() {
            state.point_at_frame(location, under.is_some());
        }
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
        if !pressed && state.release_frame(button) {
            return;
        }
        if !pressed && state.swallowed_button == Some(button) {
            state.swallowed_button = None;
            return;
        }
        if state.overview.is_open() && state.popup_grab.is_none() {
            let under = state.surface_under(state.pointer_location);
            if !under.is_some_and(|(surface, _)| state.is_above_windows(&surface)) {
                state.overview_button(pressed);
                return;
            }
        }
        if pressed {
            let under = state.surface_under(state.pointer_location);
            if state.dismisses_popups(under.as_ref().map(|(surface, _)| surface)) {
                state.swallowed_button = Some(button);
                return;
            }
            if state.popup_grab.is_none() {
                match state.press_frame(button, time) {
                    Some(true) => return,
                    Some(false) => {}
                    None => match under {
                        Some((surface, _)) => {
                            if let Some(focusable) = state
                                .toplevel_of(&surface)
                                .or_else(|| state.focusable_layer_of(&surface))
                            {
                                state.focus(Some(focusable));
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
                if !(modifiers.logo && modifiers.alt) {
                    state.input_sources.end_cycle();
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
                if sym.raw() == keysyms::KEY_XF86LaunchA && !state.is_locked() {
                    state.held_shortcut_keys.push(evdev_code);
                    return FilterResult::Intercept(Some(Shortcut::Overview));
                }
                if state.overview.is_open() && !modifiers.logo {
                    state.held_shortcut_keys.push(evdev_code);
                    return FilterResult::Intercept(Some(Shortcut::OverviewKey(
                        sym.raw(),
                        sym.key_char(),
                    )));
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
                .filter(|shortcut| {
                    modifiers.logo
                        && (!state.is_locked() || matches!(shortcut, Shortcut::InputSource { .. }))
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
    }
}

#[cfg(test)]
mod tests {
    use smithay::input::keyboard::keysyms;
    use unbound_keyboard::{Action, SHORTCUTS};
    use unbound_protocols::server::unbound_shell_v1::LauncherMode;

    use super::{Shortcut, shortcut};
    use crate::motion::DOCK_NAMESPACE;
    use crate::state::MENU_BAR_NAMESPACE;
    use crate::tile::Direction;

    fn keysym(key: &str) -> u32 {
        match key {
            "Space" => keysyms::KEY_space,
            "Tab" => keysyms::KEY_Tab,
            "Escape" => keysyms::KEY_Escape,
            "Left" => keysyms::KEY_Left,
            "Right" => keysyms::KEY_Right,
            "Up" => keysyms::KEY_Up,
            "Down" => keysyms::KEY_Down,
            "F2" => keysyms::KEY_F2,
            "F3" => keysyms::KEY_F3,
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
            Action::ToggleFullScreen => Shortcut::ToggleFullscreen,
            Action::TileWindow => Shortcut::Place(Direction::Left),
            Action::NextApp => Shortcut::NextApp,
            Action::QuitApp => Shortcut::QuitApp,
            Action::HideApp => Shortcut::HideApp,
            Action::HideOthers => Shortcut::HideOthers,
            Action::ForceQuit => Shortcut::ForceQuit,
            Action::PreviousWorkspace => Shortcut::Workspace(false),
            Action::NextWorkspace => Shortcut::Workspace(true),
            Action::WorkspaceByNumber => Shortcut::WorkspaceAt(0),
            Action::CarryWindowLeft => Shortcut::CarryWindow(false),
            Action::CarryWindowRight => Shortcut::CarryWindow(true),
            Action::Overview => Shortcut::Overview,
            Action::AppWindows => Shortcut::AppWindows,
            Action::NextInputSource => Shortcut::InputSource { backwards: false },
            Action::PreviousInputSource => Shortcut::InputSource { backwards: true },
            Action::CaptureScreen => Shortcut::CaptureScreen,
            Action::CaptureArea => Shortcut::CaptureArea,
            Action::ShowLauncher => Shortcut::ShowLauncher,
            Action::ShowClipboardHistory => Shortcut::OpenLauncher(LauncherMode::Clipboard),
            Action::ShowEmojiPicker => Shortcut::OpenLauncher(LauncherMode::Emoji),
            Action::FocusMenuBar => Shortcut::FocusShell(MENU_BAR_NAMESPACE),
            Action::FocusDock => Shortcut::FocusShell(DOCK_NAMESPACE),
            Action::LockScreen => Shortcut::LockScreen,
        }
    }

    #[test]
    fn every_shortcut_listed_in_settings_runs_its_action() {
        for listed in SHORTCUTS {
            let parts: Vec<&str> = listed.keys.split('+').collect();
            let held = |name: &str| parts.contains(&name);
            assert!(held("Super"), "{} lacks Super", listed.keys);
            let key = keysym(parts.last().copied().unwrap_or_default());
            assert_eq!(
                shortcut(key, Some(key), held("Ctrl"), held("Alt"), held("Shift")),
                Some(expected(listed.action)),
                "{}",
                listed.keys
            );
        }
    }
}
