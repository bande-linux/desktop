use std::path::Path;

use bande_power::backlight::KeyboardBacklight;
use bande_power::logind::own_session;
use zbus::blocking::Connection;

pub use bande_power::backlight::SYSFS_LEDS;

fn fraction(value: u32, maximum: u32) -> Option<f32> {
    (maximum > 0).then(|| value.min(maximum) as f32 / maximum as f32)
}

fn value(level: f32, maximum: u32) -> u32 {
    ((level.clamp(0.0, 1.0) * maximum as f32).round() as u32).min(maximum)
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
        own_session(system)?.set_brightness("leds", self.name(), value(level, maximum))
    }
}
