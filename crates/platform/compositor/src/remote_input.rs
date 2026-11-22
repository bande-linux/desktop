use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;

use reis::calloop::{EisRequestSource, EisRequestSourceEvent};
use reis::eis;
use reis::request::{Connection, Device, DeviceCapability, EisRequest, Seat};
use rustix::time::{ClockId, clock_gettime};
use smithay::backend::input::{Axis, AxisSource};
use smithay::input::keyboard::{Keycode, xkb};
use smithay::input::pointer::AxisFrame;
use smithay::reexports::calloop::PostAction;
use smithay::utils::{Logical, Point};

use crate::input::SCROLL_PER_DETENT;
use crate::state::{Compositor, State};

const EVDEV_OFFSET: u32 = 8;
const DISCRETE_PER_DETENT: f64 = 120.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum RemoteInput {
    Pointer(Point<f64, Logical>),
    Button(u32, bool),
    Scroll {
        x: f64,
        y: f64,
        v120: Option<(i32, i32)>,
    },
    Keysym(u32, bool),
    Ended,
}

#[derive(Default)]
pub(crate) struct RemoteKeys {
    held: Vec<(u64, u32, u32)>,
    buttons: Vec<(u64, u32)>,
}

#[derive(Default)]
struct Remote {
    seat: Option<Seat>,
    devices: Vec<Device>,
}

fn now_ms() -> u32 {
    let now = clock_gettime(ClockId::Monotonic);
    let millis = now.tv_sec.wrapping_mul(1000) + now.tv_nsec / 1_000_000;
    millis as u32
}

impl Remote {
    fn bind(&mut self, connection: &Connection, seat: &Seat, wanted: bool, state: &State) {
        if !wanted || !self.devices.is_empty() {
            return;
        }
        let regions: Vec<(u32, u32, u32, u32, f32)> = state
            .screens
            .iter()
            .filter(|screen| screen.mirror.is_none())
            .map(|screen| {
                let rect = screen.rect();
                let unsigned = |value: i32| u32::try_from(value).unwrap_or(0);
                (
                    unsigned(rect.loc.x),
                    unsigned(rect.loc.y),
                    unsigned(rect.size.w),
                    unsigned(rect.size.h),
                    screen.scale() as f32,
                )
            })
            .collect();
        let pointer = seat.add_device(
            Some("remote pointer"),
            eis::device::DeviceType::Virtual,
            DeviceCapability::PointerAbsolute | DeviceCapability::Button | DeviceCapability::Scroll,
            |device| {
                for (x, y, width, height, scale) in &regions {
                    device.device().region(*x, *y, *width, *height, *scale);
                }
            },
        );
        let keys = seat.add_device(
            Some("remote keys"),
            eis::device::DeviceType::Virtual,
            DeviceCapability::Text.into(),
            |_| {},
        );
        for device in [pointer, keys] {
            device.resumed();
            self.devices.push(device);
        }
        if let Err(error) = connection.flush() {
            tracing::debug!("cannot send the remote input devices: {error}");
        }
    }

    fn translate(&mut self, request: EisRequest) -> Option<RemoteInput> {
        match request {
            EisRequest::PointerMotionAbsolute(motion) => Some(RemoteInput::Pointer(Point::from((
                f64::from(motion.dx_absolute),
                f64::from(motion.dy_absolute),
            )))),
            EisRequest::Button(button) => Some(RemoteInput::Button(
                button.button,
                button.state == eis::button::ButtonState::Press,
            )),
            EisRequest::ScrollDelta(scroll) => Some(RemoteInput::Scroll {
                x: f64::from(scroll.dx),
                y: f64::from(scroll.dy),
                v120: None,
            }),
            EisRequest::ScrollDiscrete(scroll) => Some(RemoteInput::Scroll {
                x: f64::from(scroll.discrete_dx) / DISCRETE_PER_DETENT * SCROLL_PER_DETENT,
                y: f64::from(scroll.discrete_dy) / DISCRETE_PER_DETENT * SCROLL_PER_DETENT,
                v120: Some((scroll.discrete_dx, scroll.discrete_dy)),
            }),
            EisRequest::TextKeysym(key) => Some(RemoteInput::Keysym(
                key.keysym,
                key.state == eis::keyboard::KeyState::Press,
            )),
            EisRequest::Disconnect => Some(RemoteInput::Ended),
            _ => None,
        }
    }
}

impl State {
    pub(crate) fn serve_remote_input(&mut self, socket: OwnedFd) {
        let context = match eis::Context::new(UnixStream::from(socket)) {
            Ok(context) => context,
            Err(error) => {
                tracing::warn!("cannot serve remote input: {error}");
                return;
            }
        };
        let id = self.next_remote_input;
        self.next_remote_input += 1;
        let mut remote = Remote::default();
        let inserted = self.blockers.insert_source(
            EisRequestSource::new(context, 1),
            move |event, connection, state| {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        tracing::info!("remote input ended: {error}");
                        state.remote_input.push((id, RemoteInput::Ended));
                        return Ok(PostAction::Remove);
                    }
                };
                match event {
                    EisRequestSourceEvent::Connected => {
                        if connection.context_type() != eis::handshake::ContextType::Sender {
                            tracing::warn!("remote input refused a receiver context");
                            return Ok(PostAction::Remove);
                        }
                        remote.seat = Some(connection.add_seat(
                            Some("remote"),
                            DeviceCapability::PointerAbsolute
                                | DeviceCapability::Button
                                | DeviceCapability::Scroll
                                | DeviceCapability::Text,
                        ));
                    }
                    EisRequestSourceEvent::Request(EisRequest::Bind(bind)) => {
                        let wanted = remote.seat.is_some() && !bind.capabilities.is_empty();
                        remote.bind(connection, &bind.seat, wanted, state);
                    }
                    EisRequestSourceEvent::Request(request) => {
                        if let Some(input) = remote.translate(request) {
                            let ended = input == RemoteInput::Ended;
                            state.remote_input.push((id, input));
                            if ended {
                                return Ok(PostAction::Remove);
                            }
                        }
                    }
                }
                if let Err(error) = connection.flush() {
                    tracing::info!("remote input ended: {error}");
                    state.remote_input.push((id, RemoteInput::Ended));
                    return Ok(PostAction::Remove);
                }
                Ok(PostAction::Continue)
            },
        );
        if let Err(error) = inserted {
            tracing::warn!("cannot serve remote input: {error}");
        }
    }

    fn keycode_for(&mut self, keysym: u32) -> Option<u32> {
        let keyboard = self.seat.get_keyboard()?;
        keyboard.with_xkb_state(self, |context| {
            let xkb = context.xkb().lock().ok()?;
            let layout = xkb.active_layout();
            // SAFETY: the keymap is only read while the lock is held and no reference to it
            // is kept, so it cannot outlive the Xkb.
            let keymap = unsafe { xkb.keymap() };
            let wanted = xkb::Keysym::new(keysym);
            let mut found = None;
            for code in keymap.min_keycode().raw()..=keymap.max_keycode().raw() {
                let key = Keycode::new(code);
                let levels = keymap.num_levels_for_key(key, layout.0);
                for level in 0..levels {
                    if keymap
                        .key_get_syms_by_level(key, layout.0, level)
                        .contains(&wanted)
                        && found.is_none_or(|(_, best)| level < best)
                    {
                        found = Some((code, level));
                    }
                }
            }
            found.map(|(code, _)| code.saturating_sub(EVDEV_OFFSET))
        })
    }
}

impl Compositor {
    pub(crate) fn apply_remote_input(&mut self) {
        for (id, input) in std::mem::take(&mut self.state.remote_input) {
            let time = now_ms();
            match input {
                RemoteInput::Pointer(location) => self.move_pointer_to(location, time),
                RemoteInput::Button(button, pressed) => {
                    let buttons = &mut self.state.remote_keys.buttons;
                    buttons.retain(|held| *held != (id, button));
                    if pressed {
                        buttons.push((id, button));
                    }
                    self.press_button(button, pressed, time);
                }
                RemoteInput::Scroll { x, y, v120 } => {
                    let source = match v120 {
                        Some(_) => AxisSource::Wheel,
                        None => AxisSource::Continuous,
                    };
                    let mut frame = AxisFrame::new(time).source(source);
                    for (axis, amount, discrete) in [
                        (Axis::Horizontal, x, v120.map(|(x, _)| x)),
                        (Axis::Vertical, y, v120.map(|(_, y)| y)),
                    ] {
                        if amount != 0.0 {
                            frame = frame.value(axis, amount);
                        }
                        if let Some(discrete) = discrete.filter(|discrete| *discrete != 0) {
                            frame = frame.v120(axis, discrete);
                        }
                    }
                    self.scroll(frame);
                }
                RemoteInput::Keysym(keysym, pressed) => self.remote_key(id, keysym, pressed, time),
                RemoteInput::Ended => self.end_remote_input(id, time),
            }
        }
    }

    fn remote_key(&mut self, id: u64, keysym: u32, pressed: bool, time: u32) {
        let held = &mut self.state.remote_keys.held;
        if !pressed {
            if let Some(index) = held
                .iter()
                .position(|(owner, sym, _)| *owner == id && *sym == keysym)
            {
                let (_, _, code) = held.remove(index);
                self.key(code, false, time);
            }
            return;
        }
        if held
            .iter()
            .any(|(owner, sym, _)| *owner == id && *sym == keysym)
        {
            return;
        }
        let Some(code) = self.state.keycode_for(keysym) else {
            tracing::debug!("no key on this keyboard layout types keysym {keysym:#x}");
            return;
        };
        self.state.remote_keys.held.push((id, keysym, code));
        self.key(code, true, time);
    }

    fn end_remote_input(&mut self, id: u64, time: u32) {
        let keys = &mut self.state.remote_keys;
        let (released, kept) = std::mem::take(&mut keys.held)
            .into_iter()
            .partition(|(owner, _, _)| *owner == id);
        keys.held = kept;
        let (buttons, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut keys.buttons)
            .into_iter()
            .partition(|(owner, _)| *owner == id);
        keys.buttons = kept;
        let released: Vec<(u64, u32, u32)> = released;
        for (_, _, code) in released {
            self.key(code, false, time);
        }
        for (_, button) in buttons {
            self.press_button(button, false, time);
        }
    }
}
