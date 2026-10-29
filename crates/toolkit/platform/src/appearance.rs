use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

use unbound_graphics::{Appearance, Color, Scheme};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const PREFER_DARK: u32 = 1;
const REDUCE: u32 = 1;
const FIRST_READING: Duration = Duration::from_millis(500);

pub(crate) struct AppearanceWatch {
    readings: Receiver<Appearance>,
    woken: UnixStream,
    current: Appearance,
}

impl AppearanceWatch {
    pub(crate) fn start() -> io::Result<Self> {
        let (sender, readings) = channel();
        let (wake, woken) = UnixStream::pair()?;
        woken.set_nonblocking(true)?;
        thread::spawn(move || follow(&sender, &wake));
        Ok(Self {
            readings,
            woken,
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

fn follow(sender: &Sender<Appearance>, wake: &UnixStream) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let settings = Proxy::new(&connection, PORTAL, PORTAL_PATH, SETTINGS)?;
    let changes = settings.receive_signal("SettingChanged")?;
    let mut all: HashMap<String, HashMap<String, OwnedValue>> =
        settings.call("ReadAll", &(&[NAMESPACE][..],))?;
    let mut appearance = Appearance::default();
    for (key, value) in all.remove(NAMESPACE).into_iter().flatten() {
        apply(&mut appearance, &key, value);
    }
    if !publish(sender, wake, appearance) {
        return Ok(());
    }
    for change in changes {
        let (namespace, key, value): (String, String, OwnedValue) = change.body().deserialize()?;
        if namespace == NAMESPACE
            && apply(&mut appearance, &key, value)
            && !publish(sender, wake, appearance)
        {
            break;
        }
    }
    Ok(())
}

fn publish(sender: &Sender<Appearance>, mut wake: &UnixStream, appearance: Appearance) -> bool {
    sender.send(appearance).is_ok() && wake.write_all(&[0]).is_ok()
}

fn apply(appearance: &mut Appearance, key: &str, value: OwnedValue) -> bool {
    let before = *appearance;
    match key {
        "color-scheme" => {
            appearance.scheme = match u32::try_from(&value) {
                Ok(PREFER_DARK) => Scheme::Dark,
                _ => Scheme::Light,
            };
        }
        "accent-color" => appearance.accent = accent(value),
        "reduced-motion" => appearance.reduce_motion = u32::try_from(&value) == Ok(REDUCE),
        _ => {}
    }
    *appearance != before
}

fn accent(value: OwnedValue) -> Option<Color> {
    let (red, green, blue) = <(f64, f64, f64)>::try_from(value).ok()?;
    let channel = |value: f64| {
        (0.0..=1.0)
            .contains(&value)
            .then(|| (value * 255.0).round() as u8)
    };
    Some(Color {
        r: channel(red)?,
        g: channel(green)?,
        b: channel(blue)?,
        a: u8::MAX,
    })
}
