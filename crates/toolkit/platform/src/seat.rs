use std::time::{Duration, Instant};

use unbound_graphics::{Event, Key, Modifiers, Point};
use wayland_client::protocol::wl_keyboard::{self, KeymapFormat, WlKeyboard};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_seat::{self, Capability, WlSeat};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use xkbcommon::xkb::{self, Keycode, Keysym, keysyms};

use crate::window::{State, Window, WindowEvent};

const BTN_LEFT: u32 = 0x110;
const EVDEV_OFFSET: u32 = 8;

struct Keymap {
    keymap: xkb::Keymap,
    state: xkb::State,
}

struct Repeat {
    keycode: Keycode,
    next: Instant,
}

#[derive(Default)]
pub(crate) struct Input {
    pointer: Option<WlPointer>,
    keyboard: Option<WlKeyboard>,
    keymap: Option<Keymap>,
    modifiers: Modifiers,
    pointer_at: Point,
    scroll: (f32, f32),
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

impl Input {
    fn key_event(&self, keycode: Keycode) -> Option<WindowEvent> {
        let keymap = self.keymap.as_ref()?;
        let key = key_for(keymap.state.key_get_one_sym(keycode))?;
        Some(WindowEvent::Input(Event::Key {
            key,
            modifiers: self.modifiers,
        }))
    }

    fn update_modifiers(&mut self) {
        let Some(keymap) = &self.keymap else {
            return;
        };
        let active = |name: &str| {
            keymap
                .state
                .mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE)
        };
        self.modifiers = Modifiers {
            shift: active(xkb::MOD_NAME_SHIFT),
            alt: active(xkb::MOD_NAME_ALT),
            control: active(xkb::MOD_NAME_CTRL),
            logo: active(xkb::MOD_NAME_LOGO),
        };
    }

    pub(crate) fn repeat_deadline(&self) -> Option<Instant> {
        self.repeat.as_ref().map(|repeat| repeat.next)
    }
}

impl Window {
    pub fn modifiers(&self) -> Modifiers {
        self.state.input.modifiers
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
            self.state.events.push(event);
        }
    }
}

impl State {
    fn pointer_event(&mut self, event: Event) {
        self.events.push(WindowEvent::Input(event));
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
            input.pointer = Some(seat.get_pointer(handle, ()));
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
        match event {
            wl_pointer::Event::Enter {
                surface_x,
                surface_y,
                ..
            }
            | wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                let at = Point::new(surface_x as f32, surface_y as f32);
                state.input.pointer_at = at;
                state.pointer_event(Event::PointerMove(at));
            }
            wl_pointer::Event::Leave { .. } => state.events.push(WindowEvent::PointerLeft),
            wl_pointer::Event::Button {
                serial,
                button: BTN_LEFT,
                state: WEnum::Value(pressed),
                ..
            } => {
                state.input.serial = Some(serial);
                let at = state.input.pointer_at;
                state.pointer_event(match pressed {
                    wl_pointer::ButtonState::Pressed => Event::PointerDown(at),
                    _ => Event::PointerUp(at),
                });
            }
            wl_pointer::Event::Axis {
                axis: WEnum::Value(axis),
                value,
                ..
            } => match axis {
                wl_pointer::Axis::VerticalScroll => state.input.scroll.1 += value as f32,
                wl_pointer::Axis::HorizontalScroll => state.input.scroll.0 += value as f32,
                _ => {}
            },
            wl_pointer::Event::Frame => {
                let (dx, dy) = std::mem::take(&mut state.input.scroll);
                if dx != 0.0 || dy != 0.0 {
                    state.events.push(WindowEvent::Scroll { dx, dy });
                }
            }
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
            wl_keyboard::Event::Enter { serial, .. } => {
                input.serial = Some(serial);
                state.events.push(WindowEvent::Focus(true));
            }
            wl_keyboard::Event::Leave { .. } => {
                input.repeat = None;
                state.events.push(WindowEvent::Focus(false));
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
                input.update_modifiers();
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
                    let repeats = input
                        .keymap
                        .as_ref()
                        .is_some_and(|keymap| keymap.keymap.key_repeats(keycode));
                    input.repeat = (repeats && input.repeat_interval.is_some()).then(|| Repeat {
                        keycode,
                        next: Instant::now() + input.repeat_delay,
                    });
                    if let Some(event) = input.key_event(keycode) {
                        state.events.push(event);
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
