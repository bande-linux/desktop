use std::fs::{File, OpenOptions};
use std::io::Write;
use std::thread;
use std::time::Duration;

use rustix::ioctl::{IntegerSetter, NoArg, Opcode, Setter, ioctl, opcode};

pub const EV_SYN: u16 = 0;
pub const EV_KEY: u16 = 1;
pub const EV_REL: u16 = 2;
pub const REL_X: u16 = 0;
pub const REL_Y: u16 = 1;
pub const KEY_3: u16 = 4;
pub const KEY_4: u16 = 5;
pub const KEY_A: u16 = 30;
pub const KEY_ENTER: u16 = 28;
pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_F: u16 = 33;
pub const KEY_M: u16 = 50;
pub const KEY_Q: u16 = 16;
pub const KEY_ESC: u16 = 1;
pub const KEY_F10: u16 = 68;
pub const KEY_SPACE: u16 = 57;
pub const KEY_RIGHT: u16 = 106;
pub const KEY_DOWN: u16 = 108;
pub const KEY_LEFTMETA: u16 = 125;
pub const BTN_LEFT: u16 = 0x110;
const KEY_MAX: u16 = 127;
const BUS_VIRTUAL: u16 = 6;

#[repr(C)]
struct UinputSetup {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
    name: [u8; 80],
    ff_effects_max: u32,
}

const UI_DEV_CREATE: Opcode = opcode::none(b'U', 1);
const UI_DEV_SETUP: Opcode = opcode::write::<UinputSetup>(b'U', 3);
const UI_SET_EVBIT: Opcode = opcode::write::<i32>(b'U', 100);
const UI_SET_KEYBIT: Opcode = opcode::write::<i32>(b'U', 101);
const UI_SET_RELBIT: Opcode = opcode::write::<i32>(b'U', 102);

fn set_bit<const OPCODE: Opcode>(device: &File, value: u16) {
    unsafe {
        ioctl(
            device,
            IntegerSetter::<OPCODE>::new_usize(usize::from(value)),
        )
    }
    .expect("configure the virtual device");
}

pub struct VirtualDevice(File);

impl VirtualDevice {
    pub fn keyboard_and_mouse() -> Self {
        let device = OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .expect("open /dev/uinput");
        for event in [EV_KEY, EV_REL] {
            set_bit::<UI_SET_EVBIT>(&device, event);
        }
        for key in (1..=KEY_MAX).chain([BTN_LEFT]) {
            set_bit::<UI_SET_KEYBIT>(&device, key);
        }
        for axis in [REL_X, REL_Y] {
            set_bit::<UI_SET_RELBIT>(&device, axis);
        }
        let mut name = [0u8; 80];
        name[..19].copy_from_slice(b"unbound-test-uinput");
        let setup = UinputSetup {
            bustype: BUS_VIRTUAL,
            vendor: 1,
            product: 1,
            version: 1,
            name,
            ff_effects_max: 0,
        };
        unsafe { ioctl(&device, Setter::<UI_DEV_SETUP, UinputSetup>::new(setup)) }
            .expect("set up the device");
        unsafe { ioctl(&device, NoArg::<UI_DEV_CREATE>::new()) }.expect("create the device");
        Self(device)
    }

    pub fn emit(&mut self, kind: u16, code: u16, value: i32) {
        let mut event = [0u8; 24];
        event[16..18].copy_from_slice(&kind.to_ne_bytes());
        event[18..20].copy_from_slice(&code.to_ne_bytes());
        event[20..24].copy_from_slice(&value.to_ne_bytes());
        self.0.write_all(&event).expect("write an input event");
    }

    pub fn frame(&mut self, events: &[(u16, u16, i32)]) {
        for &(kind, code, value) in events {
            self.emit(kind, code, value);
        }
        self.emit(EV_SYN, 0, 0);
    }
}

const LETTERS: [u16; 26] = [
    30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45,
    21, 44,
];
const DIGITS: [u16; 10] = [11, 2, 3, 4, 5, 6, 7, 8, 9, 10];

fn us_key(character: char) -> Option<(u16, bool)> {
    let key = match character {
        'a'..='z' => (LETTERS[(character as u8 - b'a') as usize], false),
        'A'..='Z' => (LETTERS[(character as u8 - b'A') as usize], true),
        '0'..='9' => (DIGITS[(character as u8 - b'0') as usize], false),
        ' ' => (KEY_SPACE, false),
        '-' => (12, false),
        '.' => (52, false),
        '/' => (53, false),
        '>' => (52, true),
        '\n' => (KEY_ENTER, false),
        _ => return None,
    };
    Some(key)
}

impl VirtualDevice {
    pub fn tap(&mut self, key: u16) {
        self.frame(&[(EV_KEY, key, 1)]);
        self.frame(&[(EV_KEY, key, 0)]);
        thread::sleep(Duration::from_millis(15));
    }

    pub fn chord(&mut self, modifiers: &[u16], key: u16) {
        for &modifier in modifiers {
            self.frame(&[(EV_KEY, modifier, 1)]);
        }
        self.tap(key);
        for &modifier in modifiers.iter().rev() {
            self.frame(&[(EV_KEY, modifier, 0)]);
        }
        thread::sleep(Duration::from_millis(15));
    }

    pub fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            let (key, shifted) =
                us_key(character).unwrap_or_else(|| panic!("no key for {character:?}"));
            if shifted {
                self.chord(&[KEY_LEFTSHIFT], key);
            } else {
                self.tap(key);
            }
        }
    }
}
