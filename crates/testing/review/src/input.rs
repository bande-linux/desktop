use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::thread;
use std::time::{Duration, Instant};

use rustix::ioctl::{IntegerSetter, NoArg, Opcode, Setter, ioctl, opcode};

use crate::keys::{Chord, KEY_LEFTSHIFT, KEY_MAX, character_key};
use crate::scenario::{Button, Direction, Point};

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const REL_HWHEEL: u16 = 6;
const REL_WHEEL: u16 = 8;
const REL_WHEEL_HI_RES: u16 = 11;
const REL_HWHEEL_HI_RES: u16 = 12;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;
const BUS_VIRTUAL: u16 = 6;
const ABS_RANGE: i32 = 65536;
const HI_RES_PER_DETENT: i32 = 120;

const KEY_GAP: Duration = Duration::from_millis(40);
const TYPING_GAP: Duration = Duration::from_millis(55);
const CLICK_HOLD: Duration = Duration::from_millis(70);
const DOUBLE_CLICK_GAP: Duration = Duration::from_millis(120);
const SCROLL_GAP: Duration = Duration::from_millis(60);
const GLIDE_STEP: Duration = Duration::from_millis(8);
const DRAG_SETTLE: Duration = Duration::from_millis(150);

#[repr(C)]
struct UinputSetup {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[repr(C)]
struct AbsSetup {
    code: u16,
    info: AbsInfo,
}

const UI_DEV_CREATE: Opcode = opcode::none(b'U', 1);
const UI_DEV_SETUP: Opcode = opcode::write::<UinputSetup>(b'U', 3);
const UI_ABS_SETUP: Opcode = opcode::write::<AbsSetup>(b'U', 4);
const UI_SET_EVBIT: Opcode = opcode::write::<i32>(b'U', 100);
const UI_SET_KEYBIT: Opcode = opcode::write::<i32>(b'U', 101);
const UI_SET_RELBIT: Opcode = opcode::write::<i32>(b'U', 102);
const UI_SET_ABSBIT: Opcode = opcode::write::<i32>(b'U', 103);

struct Device(File);

impl Device {
    fn open() -> io::Result<Self> {
        OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .map(Self)
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("cannot open /dev/uinput ({error}); ci/review/run.sh grants access"),
                )
            })
    }

    fn enable<const OPCODE: Opcode>(&self, value: u16) -> io::Result<()> {
        unsafe {
            ioctl(
                &self.0,
                IntegerSetter::<OPCODE>::new_usize(usize::from(value)),
            )
        }
        .map_err(io::Error::from)
    }

    fn create(&self, name: &str) -> io::Result<()> {
        let mut device_name = [0u8; 80];
        let length = name.len().min(device_name.len() - 1);
        device_name[..length].copy_from_slice(&name.as_bytes()[..length]);
        let setup = UinputSetup {
            bustype: BUS_VIRTUAL,
            vendor: 0x1d6b,
            product: 0x0104,
            version: 1,
            name: device_name,
            ff_effects_max: 0,
        };
        unsafe { ioctl(&self.0, Setter::<UI_DEV_SETUP, UinputSetup>::new(setup)) }?;
        unsafe { ioctl(&self.0, NoArg::<UI_DEV_CREATE>::new()) }?;
        Ok(())
    }

    fn emit(&mut self, events: &[(u16, u16, i32)]) -> io::Result<()> {
        let mut bytes = Vec::with_capacity((events.len() + 1) * 24);
        for &(kind, code, value) in events.iter().chain([&(EV_SYN, 0, 0)]) {
            bytes.extend_from_slice(&[0; 16]);
            bytes.extend_from_slice(&kind.to_ne_bytes());
            bytes.extend_from_slice(&code.to_ne_bytes());
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        self.0.write_all(&bytes)
    }
}

pub struct Keyboard(Device);

impl Keyboard {
    pub fn new() -> io::Result<Self> {
        let device = Device::open()?;
        device.enable::<UI_SET_EVBIT>(EV_KEY)?;
        for key in 1..=KEY_MAX {
            device.enable::<UI_SET_KEYBIT>(key)?;
        }
        device.create("unbound-review keyboard")?;
        Ok(Self(device))
    }

    fn set(&mut self, key: u16, pressed: bool) -> io::Result<()> {
        self.0.emit(&[(EV_KEY, key, i32::from(pressed))])
    }

    pub fn press(&mut self, chord: &Chord) -> io::Result<()> {
        for &modifier in &chord.modifiers {
            self.set(modifier, true)?;
            thread::sleep(KEY_GAP);
        }
        self.set(chord.key, true)?;
        thread::sleep(KEY_GAP);
        self.set(chord.key, false)?;
        for &modifier in chord.modifiers.iter().rev() {
            thread::sleep(KEY_GAP);
            self.set(modifier, false)?;
        }
        thread::sleep(KEY_GAP);
        Ok(())
    }

    pub fn type_text(&mut self, text: &str) -> io::Result<()> {
        for character in text.chars() {
            let Some((key, shifted)) = character_key(character) else {
                continue;
            };
            let modifiers = if shifted {
                vec![KEY_LEFTSHIFT]
            } else {
                Vec::new()
            };
            for &modifier in &modifiers {
                self.set(modifier, true)?;
            }
            self.set(key, true)?;
            self.set(key, false)?;
            for &modifier in &modifiers {
                self.set(modifier, false)?;
            }
            thread::sleep(TYPING_GAP);
        }
        Ok(())
    }
}

pub struct Pointer {
    device: Device,
    screen: (u32, u32),
    position: (f64, f64),
}

impl Pointer {
    pub fn new() -> io::Result<Self> {
        let device = Device::open()?;
        for kind in [EV_KEY, EV_ABS, EV_REL] {
            device.enable::<UI_SET_EVBIT>(kind)?;
        }
        for button in [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE] {
            device.enable::<UI_SET_KEYBIT>(button)?;
        }
        for axis in [REL_WHEEL, REL_HWHEEL, REL_WHEEL_HI_RES, REL_HWHEEL_HI_RES] {
            device.enable::<UI_SET_RELBIT>(axis)?;
        }
        for axis in [ABS_X, ABS_Y] {
            device.enable::<UI_SET_ABSBIT>(axis)?;
            let setup = AbsSetup {
                code: axis,
                info: AbsInfo {
                    value: 0,
                    minimum: 0,
                    maximum: ABS_RANGE - 1,
                    fuzz: 0,
                    flat: 0,
                    resolution: 0,
                },
            };
            unsafe { ioctl(&device.0, Setter::<UI_ABS_SETUP, AbsSetup>::new(setup)) }?;
        }
        device.create("unbound-review pointer")?;
        Ok(Self {
            device,
            screen: (1, 1),
            position: (0.0, 0.0),
        })
    }

    pub fn set_screen(&mut self, width: u32, height: u32) -> io::Result<()> {
        self.screen = (width.max(1), height.max(1));
        self.warp((f64::from(width) / 2.0, f64::from(height) / 2.0))
    }

    fn warp(&mut self, (x, y): (f64, f64)) -> io::Result<()> {
        self.position = (x, y);
        let axis = |value: f64, extent: u32| {
            ((value / f64::from(extent)) * f64::from(ABS_RANGE))
                .clamp(0.0, f64::from(ABS_RANGE - 1)) as i32
        };
        self.device.emit(&[
            (EV_ABS, ABS_X, axis(x, self.screen.0)),
            (EV_ABS, ABS_Y, axis(y, self.screen.1)),
        ])
    }

    pub fn glide(&mut self, to: Point) -> io::Result<()> {
        let from = self.position;
        let target = (f64::from(to.x), f64::from(to.y));
        let distance = (target.0 - from.0).hypot(target.1 - from.1);
        let duration = Duration::from_secs_f64((0.18 + distance / 3000.0).min(0.6));
        let started = Instant::now();
        loop {
            let progress = (started.elapsed().as_secs_f64() / duration.as_secs_f64()).min(1.0);
            let eased = ease_in_out(progress);
            self.warp((
                from.0 + (target.0 - from.0) * eased,
                from.1 + (target.1 - from.1) * eased,
            ))?;
            if progress >= 1.0 {
                return Ok(());
            }
            thread::sleep(GLIDE_STEP);
        }
    }

    fn set_button(&mut self, button: Button, pressed: bool) -> io::Result<()> {
        let code = match button {
            Button::Left => BTN_LEFT,
            Button::Right => BTN_RIGHT,
            Button::Middle => BTN_MIDDLE,
        };
        self.device.emit(&[(EV_KEY, code, i32::from(pressed))])
    }

    pub fn click(&mut self, button: Button, count: u8) -> io::Result<()> {
        for index in 0..count {
            if index > 0 {
                thread::sleep(DOUBLE_CLICK_GAP);
            }
            self.set_button(button, true)?;
            thread::sleep(CLICK_HOLD);
            self.set_button(button, false)?;
        }
        Ok(())
    }

    pub fn drag(&mut self, from: Point, to: Point) -> io::Result<()> {
        self.glide(from)?;
        self.set_button(Button::Left, true)?;
        thread::sleep(DRAG_SETTLE);
        self.glide(to)?;
        thread::sleep(DRAG_SETTLE);
        self.set_button(Button::Left, false)
    }

    pub fn scroll(&mut self, direction: Direction, detents: u32) -> io::Result<()> {
        let (wheel, hi_res, sign) = match direction {
            Direction::Up => (REL_WHEEL, REL_WHEEL_HI_RES, 1),
            Direction::Down => (REL_WHEEL, REL_WHEEL_HI_RES, -1),
            Direction::Left => (REL_HWHEEL, REL_HWHEEL_HI_RES, -1),
            Direction::Right => (REL_HWHEEL, REL_HWHEEL_HI_RES, 1),
        };
        for _ in 0..detents {
            self.device.emit(&[
                (EV_REL, wheel, sign),
                (EV_REL, hi_res, sign * HI_RES_PER_DETENT),
            ])?;
            thread::sleep(SCROLL_GAP);
        }
        Ok(())
    }
}

fn ease_in_out(progress: f64) -> f64 {
    if progress < 0.5 {
        4.0 * progress.powi(3)
    } else {
        1.0 - (-2.0 * progress + 2.0).powi(3) / 2.0
    }
}
