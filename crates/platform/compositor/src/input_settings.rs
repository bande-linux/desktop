use std::path::PathBuf;
use std::thread;

use bande_config::{
    Accessibility, BUS_NAME, Config, OBJECT_PATH, config_path, read_system_input_sources,
};
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

use crate::keymap::KeyboardSettings;
use crate::pointing::PointerSettings;
use crate::state::Compositor;

#[derive(Debug, Clone, PartialEq)]
pub struct InputSettings {
    pub keyboard: KeyboardSettings,
    pub pointer: PointerSettings,
    pub accessibility: Accessibility,
}

impl InputSettings {
    fn of(config: &Config) -> Self {
        Self {
            keyboard: KeyboardSettings::resolve(&config.keyboard, read_system_input_sources),
            pointer: PointerSettings {
                trackpad: config.trackpad,
                mouse: config.mouse,
            },
            accessibility: config.accessibility,
        }
    }

    pub fn system() -> Self {
        Self::of(&Config::default())
    }

    pub fn read() -> Option<Self> {
        user_config().map(|config| Self::of(&config))
    }
}

pub fn user_config() -> Option<Config> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let Some(path) = config_path(config_home.as_deref(), home.as_deref()) else {
        return Some(Config::default());
    };
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(Config::default());
        }
        Err(error) => {
            tracing::warn!(
                "cannot read {}, keeping the settings: {error}",
                path.display()
            );
            return None;
        }
    };
    Config::parse(&source)
        .inspect_err(|error| {
            tracing::warn!(
                "{} is not valid, keeping the settings: {error}",
                path.display()
            );
        })
        .ok()
}

pub fn follow_input_settings(
    connection: &Connection,
    mut publish: impl FnMut(InputSettings) -> bool + Send + 'static,
) -> zbus::Result<()> {
    let changes = PropertiesProxy::builder(connection)
        .destination(BUS_NAME)?
        .path(OBJECT_PATH)?
        .build()?
        .receive_properties_changed()?;
    thread::spawn(move || {
        let mut known = InputSettings::read().unwrap_or_else(InputSettings::system);
        for _ in changes {
            let Some(read) = InputSettings::read() else {
                continue;
            };
            if read != known {
                known = read.clone();
                if !publish(read) {
                    return;
                }
            }
        }
    });
    Ok(())
}

impl Compositor {
    pub fn apply_input_settings(&mut self, settings: InputSettings) {
        self.apply_pointer_settings(settings.pointer);
        self.apply_keyboard_settings(settings.keyboard);
        self.apply_accessibility(settings.accessibility);
    }
}
