use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const SYSFS_BACKLIGHT: &str = "/sys/class/backlight";
pub const SYSFS_LEDS: &str = "/sys/class/leds";
const KEYBOARD_LIGHT: &str = "kbd_backlight";
const DIMMED_SHARE: u32 = 4;
const PREFERRED_TYPES: [&str; 3] = ["firmware", "platform", "raw"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlight {
    pub name: String,
    directory: PathBuf,
}

fn read_number(path: &Path) -> io::Result<u32> {
    fs::read_to_string(path)?
        .trim()
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

impl Backlight {
    pub fn find(class: &Path) -> Option<Self> {
        let devices: Vec<(usize, Self)> = fs::read_dir(class)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let directory = entry.path();
                let kind = fs::read_to_string(directory.join("type")).ok()?;
                let rank = PREFERRED_TYPES
                    .iter()
                    .position(|preferred| *preferred == kind.trim())?;
                let name = entry.file_name().into_string().ok()?;
                Some((rank, Self { name, directory }))
            })
            .collect();
        devices
            .into_iter()
            .min_by(|(rank, one), (other_rank, other)| {
                rank.cmp(other_rank).then_with(|| one.name.cmp(&other.name))
            })
            .map(|(_, backlight)| backlight)
    }

    pub fn brightness(&self) -> io::Result<u32> {
        read_number(&self.directory.join("brightness"))
    }

    pub fn max_brightness(&self) -> io::Result<u32> {
        read_number(&self.directory.join("max_brightness"))
    }

    pub fn actual_brightness_file(&self) -> PathBuf {
        self.directory.join("actual_brightness")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardBacklight {
    pub name: String,
    directory: PathBuf,
}

impl KeyboardBacklight {
    pub fn find(leds: &Path) -> Option<Self> {
        let mut names: Vec<String> = fs::read_dir(leds)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.contains(KEYBOARD_LIGHT))
            .collect();
        names.sort();
        let name = names.into_iter().next()?;
        Some(Self {
            directory: leds.join(&name),
            name,
        })
    }

    pub fn brightness(&self) -> io::Result<u32> {
        read_number(&self.directory.join("brightness"))
    }

    pub fn max_brightness(&self) -> io::Result<u32> {
        read_number(&self.directory.join("max_brightness"))
    }
}

pub fn dimmed(brightness: u32) -> Option<u32> {
    (brightness > 1).then(|| (brightness / DIMMED_SHARE).max(1))
}
