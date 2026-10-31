#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use bande_inbox::Inbox;
use bande_status::Update;
use bande_status::devices::{Devices, follow_devices};
use bande_test_bus::SessionBus;
use rustix::event::{PollFd, PollFlags, Timespec, poll};

const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const PROMPTLY: Duration = Duration::from_millis(1500);

struct DisplayDevice {
    percentage: f64,
}

#[zbus::interface(name = "org.freedesktop.UPower.Device")]
impl DisplayDevice {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> u32 {
        2
    }

    #[zbus(property)]
    fn is_present(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn percentage(&self) -> f64 {
        self.percentage
    }

    #[zbus(property)]
    fn state(&self) -> u32 {
        2
    }

    #[zbus(property)]
    fn time_to_empty(&self) -> i64 {
        0
    }

    #[zbus(property)]
    fn time_to_full(&self) -> i64 {
        0
    }
}

fn next_devices(inbox: &mut Inbox<Update>, within: Duration) -> Option<Devices> {
    let deadline = Instant::now() + within;
    loop {
        let found = inbox
            .drain()
            .into_iter()
            .rev()
            .find_map(|update| match update {
                Update::Devices(devices) => Some(devices),
                _ => None,
            });
        if found.is_some() {
            return found;
        }
        let left = deadline.checked_duration_since(Instant::now())?;
        let timeout = Timespec::try_from(left).unwrap();
        poll(
            &mut [PollFd::new(&inbox.fd(), PollFlags::IN)],
            Some(&timeout),
        )
        .unwrap();
    }
}

fn percent(devices: &Devices) -> Option<f64> {
    devices.0.map(|battery| battery.percent)
}

#[test]
fn a_battery_change_is_shown_as_soon_as_upower_announces_it() {
    let bus = SessionBus::start().unwrap();
    let upower = zbus::blocking::connection::Builder::address(bus.address())
        .unwrap()
        .name("org.freedesktop.UPower")
        .unwrap()
        .serve_at(DISPLAY_DEVICE, DisplayDevice { percentage: 80.0 })
        .unwrap()
        .build()
        .unwrap();
    let mut inbox = Inbox::new().unwrap();
    let _controls = follow_devices(&bus.connect().unwrap(), inbox.outbox());
    let first = next_devices(&mut inbox, PROMPTLY).expect("the first reading");
    assert_eq!(percent(&first), Some(80.0));

    let device = upower
        .object_server()
        .interface::<_, DisplayDevice>(DISPLAY_DEVICE)
        .unwrap();
    device.get_mut().percentage = 41.0;
    zbus::block_on(device.get().percentage_changed(device.signal_emitter())).unwrap();

    let changed = next_devices(&mut inbox, PROMPTLY).expect("the change");
    assert_eq!(percent(&changed), Some(41.0));
}
