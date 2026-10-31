use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;

use bande_power::backlight::{Backlight, KeyboardBacklight, SYSFS_BACKLIGHT};
use bande_power::logind::own_session;
use rustix::event::{PollFd, PollFlags, poll};
use zbus::blocking::Connection;

pub use bande_power::backlight::SYSFS_LEDS;

const READ_LIMIT: usize = 32;

fn fraction(value: u32, maximum: u32) -> Option<f32> {
    (maximum > 0).then(|| value.min(maximum) as f32 / maximum as f32)
}

fn value(level: f32, maximum: u32, lowest: u32) -> u32 {
    ((level.clamp(0.0, 1.0) * maximum as f32).round() as u32).clamp(lowest.min(maximum), maximum)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayLight {
    backlight: Backlight,
}

impl DisplayLight {
    pub fn find() -> Option<Self> {
        Self::find_in(Path::new(SYSFS_BACKLIGHT))
    }

    pub fn find_in(class: &Path) -> Option<Self> {
        Backlight::find(class).map(|backlight| Self { backlight })
    }

    pub fn level(&self) -> Option<f32> {
        let maximum = self.backlight.max_brightness().ok()?;
        fraction(self.backlight.brightness().ok()?, maximum)
    }

    pub fn set_level(&self, system: &Connection, level: f32) -> zbus::Result<()> {
        let maximum = self
            .backlight
            .max_brightness()
            .map_err(|error| zbus::Error::Failure(error.to_string()))?;
        own_session(system)?.set_brightness(
            "backlight",
            &self.backlight.name,
            value(level, maximum, 1),
        )
    }

    pub fn follow(&self, mut changed: impl FnMut(f32) -> bool) -> io::Result<()> {
        let file = File::open(self.backlight.actual_brightness_file())?;
        let mut text = [0; READ_LIMIT];
        loop {
            file.read_at(&mut text, 0)?;
            if let Some(level) = self.level()
                && !changed(level)
            {
                return Ok(());
            }
            let mut wanted = [PollFd::new(&file, PollFlags::PRI | PollFlags::ERR)];
            poll(&mut wanted, None)?;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardLight {
    backlight: KeyboardBacklight,
}

impl KeyboardLight {
    pub fn find(leds: &Path) -> Option<Self> {
        KeyboardBacklight::find(leds).map(|backlight| Self { backlight })
    }

    pub fn name(&self) -> &str {
        &self.backlight.name
    }

    fn maximum(&self) -> Option<u32> {
        self.backlight
            .max_brightness()
            .ok()
            .filter(|maximum| *maximum > 0)
    }

    pub fn level(&self) -> Option<f32> {
        fraction(self.backlight.brightness().ok()?, self.maximum()?)
    }

    pub fn set_level(&self, system: &Connection, level: f32) -> zbus::Result<()> {
        let maximum = self
            .maximum()
            .ok_or_else(|| zbus::Error::Failure(format!("{} has no maximum", self.name())))?;
        own_session(system)?.set_brightness("leds", self.name(), value(level, maximum, 0))
    }
}
