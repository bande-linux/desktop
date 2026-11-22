use std::io;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;

use bande_screen_sharing::rfb::Input;
use reis::ei;
use reis::event::{Connection, Device, DeviceCapability, EiEvent};

const SCROLL_UP: u8 = 1 << 3;
const SCROLL_DOWN: u8 = 1 << 4;
const SCROLL_LEFT: u8 = 1 << 5;
const SCROLL_RIGHT: u8 = 1 << 6;
const DETENT: i32 = 120;
const BUTTONS: [(u8, u32); 3] = [(1, 0x110), (1 << 1, 0x112), (1 << 2, 0x111)];

struct Emulated {
    connection: Option<Connection>,
    pointer: Option<Device>,
    keys: Option<Device>,
    sequence: u32,
    buttons: u8,
    region: Option<(f32, f32, f32, f32)>,
    display: (i32, i32),
}

/// Input that reaches the compositor through EIS, the libei protocol.
pub struct RemoteInput {
    emulated: Arc<Mutex<Emulated>>,
    size: (u32, u32),
}

fn lock(emulated: &Mutex<Emulated>) -> MutexGuard<'_, Emulated> {
    emulated.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_us() -> u64 {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    (now.tv_sec as u64).wrapping_mul(1_000_000) + (now.tv_nsec as u64) / 1000
}

impl Emulated {
    fn take(&mut self, event: EiEvent) {
        match event {
            EiEvent::SeatAdded(added) => added.seat.bind_capabilities(
                DeviceCapability::PointerAbsolute
                    | DeviceCapability::Button
                    | DeviceCapability::Scroll
                    | DeviceCapability::Text,
            ),
            EiEvent::DeviceResumed(resumed) => {
                let device = resumed.device;
                self.sequence += 1;
                device
                    .device()
                    .start_emulating(self.serial(), self.sequence);
                if device.has_capability(DeviceCapability::PointerAbsolute) {
                    self.region = device
                        .regions()
                        .iter()
                        .find(|region| {
                            i64::from(region.x) == i64::from(self.display.0)
                                && i64::from(region.y) == i64::from(self.display.1)
                        })
                        .or_else(|| device.regions().first())
                        .map(|region| {
                            (
                                region.x as f32,
                                region.y as f32,
                                region.width as f32,
                                region.height as f32,
                            )
                        });
                    self.pointer = Some(device);
                } else if device.has_capability(DeviceCapability::Text) {
                    self.keys = Some(device);
                }
            }
            EiEvent::DevicePaused(paused) => {
                self.pointer = self
                    .pointer
                    .take()
                    .filter(|device| *device != paused.device);
                self.keys = self.keys.take().filter(|device| *device != paused.device);
            }
            EiEvent::DeviceRemoved(removed) => {
                self.pointer = self
                    .pointer
                    .take()
                    .filter(|device| *device != removed.device);
                self.keys = self.keys.take().filter(|device| *device != removed.device);
            }
            _ => {}
        }
        self.flush();
    }

    fn serial(&self) -> u32 {
        self.connection.as_ref().map_or(0, Connection::serial)
    }

    fn flush(&self) {
        if let Some(connection) = &self.connection
            && let Err(error) = connection.flush()
        {
            tracing::debug!("cannot send input to the compositor: {error}");
        }
    }

    fn frame(&self, device: &Device) {
        device.device().frame(self.serial(), now_us());
        self.flush();
    }
}

fn follow(context: &ei::Context, emulated: &Weak<Mutex<Emulated>>) {
    let (connection, events) = match context
        .handshake_blocking("bande-screen-sharing", ei::handshake::ContextType::Sender)
    {
        Ok(connected) => connected,
        Err(error) => {
            tracing::warn!("the compositor refused remote input: {error}");
            return;
        }
    };
    match emulated.upgrade() {
        Some(emulated) => lock(&emulated).connection = Some(connection),
        None => return,
    }
    for event in events {
        let Some(emulated) = emulated.upgrade() else {
            return;
        };
        match event {
            Ok(event) => lock(&emulated).take(event),
            Err(error) => {
                tracing::warn!("remote input ended: {error}");
                return;
            }
        }
    }
}

impl RemoteInput {
    /// Connects to the compositor's EIS on `socket`, for a screen of `size` pixels that shows the
    /// display at `display` (its top left corner, in logical coordinates).
    pub fn connect(socket: UnixStream, size: (u32, u32), display: (i32, i32)) -> io::Result<Self> {
        let context = ei::Context::new(socket)?;
        let emulated = Arc::new(Mutex::new(Emulated {
            connection: None,
            pointer: None,
            keys: None,
            sequence: 0,
            buttons: 0,
            region: None,
            display,
        }));
        let followed = Arc::downgrade(&emulated);
        thread::Builder::new()
            .name("eis".into())
            .spawn(move || follow(&context, &followed))?;
        Ok(Self { emulated, size })
    }
}

impl Input for RemoteInput {
    fn pointer(&self, x: u32, y: u32, buttons: u8) {
        let mut emulated = lock(&self.emulated);
        let (Some(pointer), Some((left, top, width, height))) =
            (emulated.pointer.clone(), emulated.region)
        else {
            return;
        };
        let (screen_width, screen_height) = (self.size.0.max(1) as f32, self.size.1.max(1) as f32);
        let x = left + (x.min(self.size.0.saturating_sub(1)) as f32 + 0.5) * width / screen_width;
        let y = top + (y.min(self.size.1.saturating_sub(1)) as f32 + 0.5) * height / screen_height;
        if let Some(absolute) = pointer.interface::<ei::PointerAbsolute>() {
            absolute.motion_absolute(x, y);
        }
        let pressed = buttons & !emulated.buttons;
        let released = emulated.buttons & !buttons;
        if let Some(button) = pointer.interface::<ei::Button>() {
            for (bit, code) in BUTTONS {
                if pressed & bit != 0 {
                    button.button(code, ei::button::ButtonState::Press);
                } else if released & bit != 0 {
                    button.button(code, ei::button::ButtonState::Released);
                }
            }
        }
        if let Some(scroll) = pointer.interface::<ei::Scroll>() {
            let steps = [
                (SCROLL_UP, 0, -DETENT),
                (SCROLL_DOWN, 0, DETENT),
                (SCROLL_LEFT, -DETENT, 0),
                (SCROLL_RIGHT, DETENT, 0),
            ];
            for (bit, x, y) in steps {
                if pressed & bit != 0 {
                    scroll.scroll_discrete(x, y);
                }
            }
        }
        emulated.buttons = buttons;
        emulated.frame(&pointer);
    }

    fn key(&self, keysym: u32, down: bool) {
        let emulated = lock(&self.emulated);
        let Some(keys) = emulated.keys.clone() else {
            return;
        };
        if let Some(text) = keys.interface::<ei::Text>() {
            let state = match down {
                true => ei::keyboard::KeyState::Press,
                false => ei::keyboard::KeyState::Released,
            };
            text.keysym(keysym, state);
            emulated.frame(&keys);
        }
    }
}
