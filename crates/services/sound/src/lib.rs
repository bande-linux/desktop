mod theme;

use std::fs;
use std::io;
use std::path::Path;

pub use theme::{Alert, RATE, Sound, wav};
use zbus::blocking::Connection;
use zbus::message::{Flags, Message};

pub const BUS_NAME: &str = "org.unbound.Sound";
pub const OBJECT_PATH: &str = "/org/unbound/Sound";
pub const THEME_NAME: &str = "Unbound";

const INDEX_THEME: &str = "[Sound Theme]
Name=Unbound
Comment=Dry, short wooden sounds of the Unbound desktop
Inherits=freedesktop
Directories=stereo

[stereo]
OutputProfile=stereo
";

pub fn play(connection: &Connection, name: &str) {
    let message = Message::method_call(OBJECT_PATH, "Play")
        .and_then(|call| call.destination(BUS_NAME))
        .and_then(|call| call.interface(BUS_NAME))
        .and_then(|call| call.with_flags(Flags::NoReplyExpected))
        .and_then(|call| call.build(&(name,)));
    if let Err(error) = message.and_then(|message| connection.send(&message)) {
        tracing::debug!("cannot play {name}: {error}");
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Policy {
    pub effects: bool,
    pub volume_feedback: bool,
    pub alert: Alert,
    pub alert_volume: f32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            effects: true,
            volume_feedback: true,
            alert: Alert::default(),
            alert_volume: 0.75,
        }
    }
}

impl Policy {
    pub fn gain(&self, sound: Sound) -> Option<f32> {
        let allowed = match sound {
            Sound::Alert(_) | Sound::Message => true,
            Sound::VolumeChange => self.volume_feedback,
            Sound::ScreenCapture
            | Sound::TrashEmpty
            | Sound::DeviceAdded
            | Sound::DeviceRemoved
            | Sound::BatteryLow => self.effects,
        };
        Some(self.alert_volume.clamp(0.0, 1.0)).filter(|gain| allowed && *gain > 0.0)
    }
}

pub fn theme_files() -> Vec<(String, Vec<u8>)> {
    let mut files = vec![("index.theme".to_owned(), INDEX_THEME.as_bytes().to_vec())];
    files.extend(Sound::THEME.into_iter().map(|sound| {
        (
            format!("stereo/{}.wav", sound.name()),
            wav(&sound.samples()),
        )
    }));
    files
}

pub fn write_theme(directory: &Path) -> io::Result<()> {
    for (name, bytes) in theme_files() {
        let path = directory.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)?;
    }
    Ok(())
}
