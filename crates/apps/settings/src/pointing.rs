use std::path::Path;

pub const UDEV_DATA: &str = "/run/udev/data";
const INPUT_DEVICE_PREFIX: &str = "c13:";
const TOUCHPAD: &str = "E:ID_INPUT_TOUCHPAD=1";
const MOUSE: &str = "E:ID_INPUT_MOUSE=1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PointingDevices {
    pub trackpad: bool,
    pub mouse: bool,
}

impl PointingDevices {
    pub fn scan(udev_data: &Path) -> Self {
        let mut found = Self::default();
        let Ok(entries) = std::fs::read_dir(udev_data) else {
            return found;
        };
        for entry in entries.filter_map(Result::ok) {
            if !entry
                .file_name()
                .to_string_lossy()
                .starts_with(INPUT_DEVICE_PREFIX)
            {
                continue;
            }
            let Ok(properties) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let has = |property: &str| properties.lines().any(|line| line.trim() == property);
            if has(TOUCHPAD) {
                found.trackpad = true;
            } else if has(MOUSE) {
                found.mouse = true;
            }
        }
        found
    }
}
