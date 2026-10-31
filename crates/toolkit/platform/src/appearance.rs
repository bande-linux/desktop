use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

use unbound_bus::portal_settings::{self, APPEARANCE, PREFER_DARK, UNBOUND_APPEARANCE};
use unbound_graphics::{Accent, Appearance, Color, Contrast, Scheme, TextSize};
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

const REDUCE: u32 = 1;
const HIGH_CONTRAST: u32 = 1;
const FIRST_READING: Duration = Duration::from_millis(500);

pub(crate) struct AppearanceWatch {
    readings: Receiver<Appearance>,
    woken: UnixStream,
    _wake: UnixStream,
    current: Appearance,
}

impl AppearanceWatch {
    pub(crate) fn start() -> io::Result<Self> {
        let (sender, readings) = channel();
        let (wake, woken) = UnixStream::pair()?;
        woken.set_nonblocking(true)?;
        let follower = wake.try_clone()?;
        thread::spawn(move || follow(&sender, &follower));
        Ok(Self {
            readings,
            woken,
            _wake: wake,
            current: Appearance::default(),
        })
    }

    pub(crate) fn wait_for_first_reading(&mut self) {
        if let Ok(appearance) = self.readings.recv_timeout(FIRST_READING) {
            self.current = appearance;
        }
    }

    pub(crate) fn current(&self) -> Appearance {
        self.current
    }

    pub(crate) fn fd(&self) -> BorrowedFd<'_> {
        self.woken.as_fd()
    }

    pub(crate) fn take_change(&mut self) -> Option<Appearance> {
        let mut drained = [0; 64];
        while matches!(self.woken.read(&mut drained), Ok(read) if read > 0) {}
        let latest = self.readings.try_iter().last()?;
        let changed = latest != self.current;
        self.current = latest;
        changed.then_some(latest)
    }
}

fn follow(sender: &Sender<Appearance>, mut wake: &UnixStream) -> zbus::Result<()> {
    portal_settings::follow(
        &Connection::session()?,
        &[APPEARANCE, UNBOUND_APPEARANCE],
        Appearance::default(),
        apply,
        |appearance| sender.send(appearance).is_ok() && wake.write_all(&[0]).is_ok(),
    )
}

fn apply(appearance: &mut Appearance, namespace: &str, key: &str, value: OwnedValue) {
    match (namespace, key) {
        (APPEARANCE, "color-scheme") => {
            appearance.scheme = match u32::try_from(&value) {
                Ok(PREFER_DARK) => Scheme::Dark,
                _ => Scheme::Light,
            };
        }
        (APPEARANCE, "accent-color") => {
            appearance.accent = <(f64, f64, f64)>::try_from(value)
                .ok()
                .and_then(|(red, green, blue)| color(&[red, green, blue]))
                .map_or_else(Accent::default, Accent::nearest);
        }
        (APPEARANCE, "contrast") => {
            appearance.contrast = match u32::try_from(&value) {
                Ok(HIGH_CONTRAST) => Contrast::Increased,
                _ => Contrast::Standard,
            };
        }
        (APPEARANCE, "reduced-motion") => {
            appearance.reduce_motion = u32::try_from(&value) == Ok(REDUCE);
        }
        (UNBOUND_APPEARANCE, "text-size") => {
            appearance.text_size =
                u32::try_from(&value).map_or(TextSize::DEFAULT, TextSize::from_percent);
        }
        (UNBOUND_APPEARANCE, "frame-tint") => {
            appearance.frame_tint = Vec::<f64>::try_from(value)
                .ok()
                .and_then(|channels| color(&channels));
        }
        _ => {}
    }
}

fn color(channels: &[f64]) -> Option<Color> {
    let channel = |value: f64| {
        (0.0..=1.0)
            .contains(&value)
            .then(|| (value * 255.0).round() as u8)
    };
    let [red, green, blue] = channels else {
        return None;
    };
    Some(Color {
        r: channel(*red)?,
        g: channel(*green)?,
        b: channel(*blue)?,
        a: u8::MAX,
    })
}
