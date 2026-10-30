use std::path::PathBuf;
use std::thread;

use unbound_config::{
    Accessibility, BUS_NAME, Config, OBJECT_PATH, config_path, read_system_input_sources,
};
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

use crate::keymap::KeyboardSettings;
use crate::pointing::PointerSettings;
use crate::state::Headless;

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

    pub fn read() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        let config = config_path(config_home.as_deref(), home.as_deref())
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|source| Config::parse(&source).ok())
            .unwrap_or_default();
        Self::of(&config)
    }
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
        let mut known = InputSettings::read();
        for _ in changes {
            let read = InputSettings::read();
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

impl Headless {
    pub fn apply_input_settings(&mut self, settings: InputSettings) {
        self.apply_pointer_settings(settings.pointer);
        self.apply_keyboard_settings(settings.keyboard);
        self.apply_accessibility(settings.accessibility);
    }
}
