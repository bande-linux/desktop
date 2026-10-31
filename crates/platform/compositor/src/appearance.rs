use std::thread;

use unbound_bus::portal_settings::{self, APPEARANCE, PREFER_DARK, UNBOUND_APPEARANCE};
use unbound_graphics::TextSize;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SystemAppearance {
    pub reduce_motion: bool,
    pub dark: bool,
    pub text_size: TextSize,
}

impl SystemAppearance {
    fn apply(&mut self, namespace: &str, key: &str, value: OwnedValue) {
        let value = u32::try_from(value).ok();
        match (namespace, key) {
            (APPEARANCE, "reduced-motion") => {
                self.reduce_motion = value.is_some_and(|reduce| reduce != 0);
            }
            (APPEARANCE, "color-scheme") => self.dark = value == Some(PREFER_DARK),
            (UNBOUND_APPEARANCE, "text-size") => {
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
            &[APPEARANCE, UNBOUND_APPEARANCE],
            SystemAppearance::default(),
            SystemAppearance::apply,
            publish,
        );
        if let Err(error) = followed {
            tracing::warn!("cannot follow the appearance: {error}");
        }
    });
}
