use std::fs::{File, OpenOptions};
use std::io::Write;
use std::time::{Duration, Instant};

use rustix::ioctl::{IntegerSetter, NoArg, Opcode, Setter, ioctl, opcode};
use unbound_compositor::Headless;
use unbound_test_wayland::{RED, ShmImage, TestClient};
use wayland_client::protocol::wl_shm::Format;

use crate::session::Session;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const KEY_A: u16 = 30;
const BTN_LEFT: u16 = 0x110;
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

struct VirtualDevice(File);

impl VirtualDevice {
    fn keyboard_and_mouse() -> Self {
        let device = OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .expect("open /dev/uinput");
        for event in [EV_KEY, EV_REL] {
            set_bit::<UI_SET_EVBIT>(&device, event);
        }
        for key in [KEY_A, BTN_LEFT] {
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

    fn emit(&mut self, kind: u16, code: u16, value: i32) {
        let mut event = [0u8; 24];
        event[16..18].copy_from_slice(&kind.to_ne_bytes());
        event[18..20].copy_from_slice(&code.to_ne_bytes());
        event[20..24].copy_from_slice(&value.to_ne_bytes());
        self.0.write_all(&event).expect("write an input event");
    }

    fn frame(&mut self, events: &[(u16, u16, i32)]) {
        for &(kind, code, value) in events {
            self.emit(kind, code, value);
        }
        self.emit(EV_SYN, 0, 0);
    }
}

fn window(client: &mut TestClient, server: &mut Headless, width: i32, height: i32) {
    let pixels = RED.repeat((width * height) as usize);
    client.show_window(
        server,
        &ShmImage {
            width,
            height,
            stride: width * 4,
            format: Format::Xrgb8888,
            bytes: &pixels,
        },
    );
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn uinput_events_reach_the_window_under_the_pointer_after_a_click() {
    let mut session = Session::on_vkms();
    let input = session
        .output
        .input_backend()
        .expect("libinput on the seat");
    session
        .events
        .handle()
        .insert_source(input, |event, _, shared| {
            shared.server.handle_input(event).expect("deliver input");
        })
        .expect("libinput events");
    session.client.use_seat(&mut session.shared.server);
    window(&mut session.client, &mut session.shared.server, 400, 300);
    let mut front = TestClient::connect(&mut session.shared.server);
    front.use_seat(&mut session.shared.server);
    window(&mut front, &mut session.shared.server, 32, 32);
    front.exchange(&mut session.shared.server);
    assert!(front.keyboard_focus_on(0));

    let mut device = VirtualDevice::keyboard_and_mouse();
    let start = session.shared.server.pointer_location();
    let deadline = Instant::now() + Duration::from_secs(5);
    while session.shared.server.pointer_location() == start {
        assert!(
            Instant::now() < deadline,
            "libinput never opened the device"
        );
        device.frame(&[(EV_REL, REL_X, 1)]);
        session
            .events
            .dispatch(Some(Duration::from_millis(20)), &mut session.shared)
            .expect("dispatch events");
    }
    device.frame(&[(EV_REL, REL_X, 100)]);
    device.frame(&[(EV_KEY, BTN_LEFT, 1)]);
    device.frame(&[(EV_KEY, BTN_LEFT, 0)]);
    device.frame(&[(EV_KEY, KEY_A, 1)]);
    device.frame(&[(EV_KEY, KEY_A, 0)]);
    session.dispatch_until("the button and key releases", |session| {
        session.client.buttons().len() >= 2 && session.client.keys().len() >= 2
    });
    front.exchange(&mut session.shared.server);

    assert!(session.client.pointer_focus().is_some());
    assert_eq!(
        session.client.buttons(),
        [(u32::from(BTN_LEFT), true), (u32::from(BTN_LEFT), false)]
    );
    assert!(session.client.keyboard_focus_on(0));
    assert_eq!(
        session.client.keys(),
        [(u32::from(KEY_A), true), (u32::from(KEY_A), false)]
    );
    assert!(!front.keyboard_focus_on(0));
    assert!(front.keys().is_empty());
}
