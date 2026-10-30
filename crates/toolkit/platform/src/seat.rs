use std::time::{Duration, Instant};

use unbound_graphics::{
    Cursor, Event, Key, Modifiers, Point, Pointer, PointerButton, Scroll, ScrollPhase, ScrollSource,
};
use wayland_client::protocol::wl_keyboard::{self, KeymapFormat, WlKeyboard};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_seat::{self, Capability, WlSeat};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1;
use xkbcommon::xkb::{self, Keycode, Keysym, keysyms};

use crate::window::{State, Window, WindowEvent};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;
const BTN_FORWARD: u32 = 0x115;
const BTN_BACK: u32 = 0x116;
const EVDEV_OFFSET: u32 = 8;
pub use unbound_graphics::DOUBLE_CLICK_INTERVAL;
pub const CLICK_SLOP: f32 = 4.0;

struct Keymap {
    keymap: xkb::Keymap,
    state: xkb::State,
}

struct Repeat {
    keycode: Keycode,
    next: Instant,
}

#[derive(Default)]
struct ScrollFrame {
    delta: (f32, f32),
    source: Option<ScrollSource>,
    stopped: bool,
}

#[derive(Default)]
struct Clicks {
    last: Option<(PointerButton, Point, Instant)>,
    count: u8,
}

impl Clicks {
    fn press(&mut self, button: PointerButton, at: Point, now: Instant) -> u8 {
        let continues = self.last.is_some_and(|(last_button, last_at, last_time)| {
            last_button == button
                && now.duration_since(last_time) <= DOUBLE_CLICK_INTERVAL
                && (at.x - last_at.x).hypot(at.y - last_at.y) <= CLICK_SLOP
        });
        self.count = if continues {
            self.count.saturating_add(1)
        } else {
            1
        };
        self.last = Some((button, at, now));
        self.count
    }
}

#[derive(Default)]
pub(crate) struct Input {
    pointer: Option<WlPointer>,
    cursor_device: Option<WpCursorShapeDeviceV1>,
    enter_serial: Option<u32>,
    cursor: Cursor,
    shown_cursor: Option<Cursor>,
    keyboard: Option<WlKeyboard>,
    keymap: Option<Keymap>,
    modifiers: Modifiers,
    caps_lock: bool,
    pointer_at: Point,
    held: Vec<(u32, PointerButton)>,
    pointer_popup: Option<usize>,
    pub(crate) keyboard_initiated: bool,
    keyboard_popup: Option<usize>,
    clicks: Clicks,
    scroll: ScrollFrame,
    scroll_source: ScrollSource,
    scrolling: bool,
    repeat_interval: Option<Duration>,
    repeat_delay: Duration,
    repeat: Option<Repeat>,
    pub(crate) serial: Option<u32>,
}

pub(crate) fn key_for(keysym: Keysym) -> Option<Key> {
    let key = match keysym.raw() {
        keysyms::KEY_Left | keysyms::KEY_KP_Left => Key::Left,
        keysyms::KEY_Right | keysyms::KEY_KP_Right => Key::Right,
        keysyms::KEY_Up | keysyms::KEY_KP_Up => Key::Up,
        keysyms::KEY_Down | keysyms::KEY_KP_Down => Key::Down,
        keysyms::KEY_BackSpace => Key::Backspace,
        keysyms::KEY_Delete | keysyms::KEY_KP_Delete => Key::Delete,
        keysyms::KEY_Return | keysyms::KEY_KP_Enter => Key::Enter,
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => Key::Tab,
        keysyms::KEY_Escape => Key::Escape,
        keysyms::KEY_space => Key::Space,
        keysyms::KEY_Home | keysyms::KEY_KP_Home => Key::Home,
        keysyms::KEY_End | keysyms::KEY_KP_End => Key::End,
        keysyms::KEY_Page_Up | keysyms::KEY_KP_Page_Up => Key::PageUp,
        keysyms::KEY_Page_Down | keysyms::KEY_KP_Page_Down => Key::PageDown,
        keysyms::KEY_Insert | keysyms::KEY_KP_Insert => Key::Insert,
        keysyms::KEY_Menu => Key::Menu,
        raw @ keysyms::KEY_F1..=keysyms::KEY_F12 => {
            Key::Function(u8::try_from(raw - keysyms::KEY_F1 + 1).ok()?)
        }
        _ => {
            let character = char::from_u32(xkb::keysym_to_utf32(keysym))?;
            if character.is_control() || character == '\0' {
                return None;
            }
            Key::Character(character)
        }
    };
    Some(key)
}

fn button_for(code: u32) -> Option<PointerButton> {
    Some(match code {
        BTN_LEFT => PointerButton::Primary,
        BTN_RIGHT => PointerButton::Secondary,
        BTN_MIDDLE => PointerButton::Middle,
        BTN_SIDE | BTN_BACK => PointerButton::Back,
        BTN_EXTRA | BTN_FORWARD => PointerButton::Forward,
        _ => return None,
    })
}

fn shape_for(cursor: Cursor) -> Shape {
    match cursor {
        Cursor::Default => Shape::Default,
        Cursor::Pointer => Shape::Pointer,
        Cursor::Text => Shape::Text,
        Cursor::Crosshair => Shape::Crosshair,
        Cursor::Move => Shape::Move,
        Cursor::Grab => Shape::Grab,
        Cursor::Grabbing => Shape::Grabbing,
        Cursor::NotAllowed => Shape::NotAllowed,
        Cursor::ContextMenu => Shape::ContextMenu,
        Cursor::Copy => Shape::Copy,
        Cursor::Alias => Shape::Alias,
        Cursor::ColumnResize => Shape::ColResize,
        Cursor::RowResize => Shape::RowResize,
        Cursor::ResizeTop => Shape::NResize,
        Cursor::ResizeBottom => Shape::SResize,
        Cursor::ResizeLeft => Shape::WResize,
        Cursor::ResizeRight => Shape::EResize,
        Cursor::ResizeTopLeft => Shape::NwResize,
        Cursor::ResizeTopRight => Shape::NeResize,
        Cursor::ResizeBottomLeft => Shape::SwResize,
        Cursor::ResizeBottomRight => Shape::SeResize,
    }
}

impl Input {
    fn key_event(&self, keycode: Keycode) -> Option<WindowEvent> {
        let keymap = self.keymap.as_ref()?;
        let key = key_for(keymap.state.key_get_one_sym(keycode))?;
        Some(WindowEvent::Input(Event::Key {
            key,
            modifiers: self.modifiers,
        }))
    }

    fn update_modifiers(&mut self) -> bool {
        let Some(keymap) = &self.keymap else {
            return false;
        };
        let active = |name: &str| {
            keymap
                .state
                .mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE)
        };
        let modifiers = Modifiers {
            shift: active(xkb::MOD_NAME_SHIFT),
            alt: active(xkb::MOD_NAME_ALT),
            control: active(xkb::MOD_NAME_CTRL),
            logo: active(xkb::MOD_NAME_LOGO),
        };
        let changed = modifiers != self.modifiers;
        self.modifiers = modifiers;
        changed
    }

    fn update_caps_lock(&mut self) -> bool {
        let caps_lock = self.keymap.as_ref().is_some_and(|keymap| {
            keymap
                .state
                .mod_name_is_active(xkb::MOD_NAME_CAPS, xkb::STATE_MODS_LOCKED)
        });
        let changed = caps_lock != self.caps_lock;
        self.caps_lock = caps_lock;
        changed
    }

    fn show_cursor(&mut self) {
        let (Some(device), Some(serial)) = (&self.cursor_device, self.enter_serial) else {
            return;
        };
        if self.shown_cursor == Some(self.cursor) {
            return;
        }
        device.set_shape(serial, shape_for(self.cursor));
        self.shown_cursor = Some(self.cursor);
    }

    pub(crate) fn forget_popups(&mut self) {
        self.pointer_popup = None;
        self.keyboard_popup = None;
    }

    pub(crate) fn set_cursor(&mut self, cursor: Cursor) {
        self.cursor = cursor;
        self.show_cursor();
    }

    fn pointer(&self, position: Point) -> Pointer {
        Pointer::at(position).with_modifiers(self.modifiers)
    }

    pub(crate) fn repeat_deadline(&self) -> Option<Instant> {
        self.repeat.as_ref().map(|repeat| repeat.next)
    }
}

impl Window {
    pub fn modifiers(&self) -> Modifiers {
        self.state.input.modifiers
    }

    pub fn set_cursor(&mut self, cursor: Cursor) {
        self.app_cursor = cursor;
        if self.pointer_in_content() {
            self.state.input.set_cursor(cursor);
        }
    }

    pub(crate) fn fire_repeats(&mut self, now: Instant) {
        let input = &mut self.state.input;
        let (Some(repeat), Some(interval)) = (&mut input.repeat, input.repeat_interval) else {
            return;
        };
        if repeat.next > now {
            return;
        }
        repeat.next = now + interval;
        let keycode = repeat.keycode;
        if let Some(event) = input.key_event(keycode) {
            self.state.key_event(event);
        }
    }
}

impl State {
    fn pointer_event(&mut self, event: Event) {
        match self.input.pointer_popup {
            Some(level) => self.popup_input.push((level, event)),
            None => self.events.push(WindowEvent::Input(event)),
        }
    }

    fn key_event(&mut self, event: WindowEvent) {
        match (self.input.keyboard_popup, event) {
            (Some(level), WindowEvent::Input(input)) => self.popup_input.push((level, input)),
            (_, event) => self.events.push(event),
        }
    }

    fn popup_level(&self, surface: &WlSurface) -> Option<usize> {
        self.popup_surfaces
            .iter()
            .position(|popup| popup == surface)
    }

    fn press(&mut self, serial: u32, code: u32, pressed: bool, now: Instant) {
        let input = &mut self.input;
        input.serial = Some(serial);
        let position = input.pointer_at;
        if !pressed {
            let Some(index) = input.held.iter().position(|(held, _)| *held == code) else {
                return;
            };
            let (_, button) = input.held.remove(index);
            let pointer = input
                .pointer(position)
                .with_button(button)
                .with_count(input.clicks.count);
            self.pointer_event(Event::PointerUp(pointer));
            return;
        }
        let Some(mut button) = button_for(code) else {
            return;
        };
        if button == PointerButton::Primary && input.modifiers.control {
            button = PointerButton::Secondary;
        }
        input.keyboard_initiated = false;
        input.held.retain(|(held, _)| *held != code);
        input.held.push((code, button));
        let count = input.clicks.press(button, position, now);
        let pointer = input
            .pointer(position)
            .with_button(button)
            .with_count(count);
        self.pointer_event(Event::PointerDown(pointer));
    }

    fn scroll_frame(&mut self) {
        let input = &mut self.input;
        let frame = std::mem::take(&mut input.scroll);
        let source = frame.source.unwrap_or(input.scroll_source);
        let moved = frame.delta != (0.0, 0.0);
        let continuous = source != ScrollSource::Wheel;
        let phase = if continuous && frame.stopped {
            if !input.scrolling {
                return;
            }
            input.scrolling = false;
            ScrollPhase::Ended
        } else if !moved {
            return;
        } else if continuous && !input.scrolling {
            input.scrolling = true;
            ScrollPhase::Began
        } else {
            ScrollPhase::Changed
        };
        input.scroll_source = source;
        let scroll = Scroll {
            position: input.pointer_at,
            delta: Point::new(frame.delta.0, frame.delta.1),
            source,
            phase,
            modifiers: input.modifiers,
        };
        self.pointer_event(Event::Scroll(scroll));
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        else {
            return;
        };
        let input = &mut state.input;
        if capabilities.contains(Capability::Pointer) && input.pointer.is_none() {
            let pointer = seat.get_pointer(handle, ());
            input.cursor_device = state
                .cursor_shape_manager
                .as_ref()
                .map(|manager| manager.get_pointer(&pointer, handle, ()));
            input.pointer = Some(pointer);
        }
        if capabilities.contains(Capability::Keyboard) && input.keyboard.is_none() {
            input.keyboard = Some(seat.get_keyboard(handle, ()));
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let input = &mut state.input;
        match event {
            wl_pointer::Event::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
            } => {
                let popup = state.popup_level(&surface);
                let input = &mut state.input;
                input.pointer_popup = popup;
                let at = Point::new(surface_x as f32, surface_y as f32);
                input.pointer_at = at;
                input.enter_serial = Some(serial);
                input.shown_cursor = None;
                input.show_cursor();
                let pointer = input.pointer(at);
                state.pointer_event(Event::PointerEnter(pointer));
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                let at = Point::new(surface_x as f32, surface_y as f32);
                input.pointer_at = at;
                let pointer = input.pointer(at);
                state.pointer_event(Event::PointerMove(pointer));
            }
            wl_pointer::Event::Leave { .. } => {
                input.enter_serial = None;
                state.pointer_event(Event::PointerLeave);
                state.input.pointer_popup = None;
            }
            wl_pointer::Event::Button {
                serial,
                button,
                state: WEnum::Value(pressed),
                ..
            } => state.press(
                serial,
                button,
                pressed == wl_pointer::ButtonState::Pressed,
                Instant::now(),
            ),
            wl_pointer::Event::Axis {
                axis: WEnum::Value(axis),
                value,
                ..
            } => match axis {
                wl_pointer::Axis::VerticalScroll => input.scroll.delta.1 += value as f32,
                wl_pointer::Axis::HorizontalScroll => input.scroll.delta.0 += value as f32,
                _ => {}
            },
            wl_pointer::Event::AxisSource {
                axis_source: WEnum::Value(source),
            } => {
                input.scroll.source = Some(match source {
                    wl_pointer::AxisSource::Finger => ScrollSource::Finger,
                    wl_pointer::AxisSource::Continuous => ScrollSource::Continuous,
                    _ => ScrollSource::Wheel,
                });
            }
            wl_pointer::Event::AxisStop { .. } => input.scroll.stopped = true,
            wl_pointer::Event::Frame => state.scroll_frame(),
            _ => {}
        }
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let input = &mut state.input;
        match event {
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(KeymapFormat::XkbV1),
                fd,
                size,
            } => {
                let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
                let keymap = unsafe {
                    xkb::Keymap::new_from_fd(
                        &context,
                        fd,
                        size as usize,
                        xkb::KEYMAP_FORMAT_TEXT_V1,
                        xkb::KEYMAP_COMPILE_NO_FLAGS,
                    )
                };
                if let Ok(Some(keymap)) = keymap {
                    let xkb_state = xkb::State::new(&keymap);
                    input.keymap = Some(Keymap {
                        keymap,
                        state: xkb_state,
                    });
                }
            }
            wl_keyboard::Event::Enter {
                serial, surface, ..
            } => {
                let popup = state.popup_level(&surface);
                let input = &mut state.input;
                input.serial = Some(serial);
                input.keyboard_popup = popup;
                if popup.is_none() {
                    state.events.push(WindowEvent::Focus(true));
                }
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                input.repeat = None;
                let menu_open = !state.popup_surfaces.is_empty();
                if state.popup_level(&surface).is_none() && !menu_open {
                    state.events.push(WindowEvent::Focus(false));
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(keymap) = &mut input.keymap {
                    keymap.state.update_mask(
                        mods_depressed,
                        mods_latched,
                        mods_locked,
                        0,
                        0,
                        group,
                    );
                }
                if input.update_caps_lock() {
                    state.events.push(WindowEvent::CapsLock(input.caps_lock));
                }
                if input.update_modifiers() {
                    let modifiers = input.modifiers;
                    if let Some(level) = input.keyboard_popup {
                        state
                            .popup_input
                            .push((level, Event::ModifiersChanged(modifiers)));
                    }
                    state
                        .events
                        .push(WindowEvent::Input(Event::ModifiersChanged(modifiers)));
                }
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                input.repeat_interval = u64::try_from(rate)
                    .ok()
                    .filter(|rate| *rate > 0)
                    .map(|rate| Duration::from_micros(1_000_000 / rate));
                input.repeat_delay = Duration::from_millis(u64::try_from(delay).unwrap_or(0));
            }
            wl_keyboard::Event::Key {
                serial,
                key,
                state: WEnum::Value(key_state),
                ..
            } => {
                input.serial = Some(serial);
                let keycode = Keycode::new(key + EVDEV_OFFSET);
                if key_state == wl_keyboard::KeyState::Pressed {
                    input.keyboard_initiated = true;
                    let repeats = input
                        .keymap
                        .as_ref()
                        .is_some_and(|keymap| keymap.keymap.key_repeats(keycode));
                    input.repeat = (repeats && input.repeat_interval.is_some()).then(|| Repeat {
                        keycode,
                        next: Instant::now() + input.repeat_delay,
                    });
                    if let Some(event) = input.key_event(keycode) {
                        state.key_event(event);
                    }
                } else if input
                    .repeat
                    .as_ref()
                    .is_some_and(|repeat| repeat.keycode == keycode)
                {
                    input.repeat = None;
                }
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore WpCursorShapeManagerV1);
delegate_noop!(State: ignore WpCursorShapeDeviceV1);
