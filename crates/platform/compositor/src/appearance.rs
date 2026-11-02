use std::thread;

use bande_bus::portal_settings::{self, APPEARANCE, BANDE_APPEARANCE, PREFER_LIGHT};
use bande_graphics::TextSize;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemAppearance {
    pub reduce_motion: bool,
    pub dark: bool,
    pub text_size: TextSize,
}

impl Default for SystemAppearance {
    fn default() -> Self {
        Self {
            reduce_motion: false,
            dark: true,
            text_size: TextSize::default(),
        }
    }
}

impl SystemAppearance {
    fn apply(&mut self, namespace: &str, key: &str, value: OwnedValue) {
        let value = u32::try_from(value).ok();
        match (namespace, key) {
            (APPEARANCE, "reduced-motion") => {
                self.reduce_motion = value.is_some_and(|reduce| reduce != 0);
            }
            (APPEARANCE, "color-scheme") => self.dark = value != Some(PREFER_LIGHT),
            (BANDE_APPEARANCE, "text-size") => {
                self.text_size = value.map_or(TextSize::DEFAULT, TextSize::from_percent);
            }
            _ => {}
        }
    }
}

pub fn follow_appearance(
    connection: Connection,
    publish: impl FnMut(SystemAppearance) -> bool + Send + 'static,
) {
    thread::spawn(move || {
        let followed = portal_settings::follow(
            &connection,
            &[APPEARANCE, BANDE_APPEARANCE],
            SystemAppearance::default(),
            SystemAppearance::apply,
            publish,
        );
        if let Err(error) = followed {
            tracing::warn!("cannot follow the appearance: {error}");
        }
    });
}
