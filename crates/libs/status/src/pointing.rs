use std::path::Path;

pub const UDEV_DATA: &str = "/run/udev/data";
const INPUT_DEVICE_PREFIX: &str = "c13:";
const TOUCHPAD: &str = "E:ID_INPUT_TOUCHPAD=1";
const MICE: [&str; 2] = ["E:ID_INPUT_MOUSE=1", "E:ID_INPUT_POINTINGSTICK=1"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PointingDevices {
    pub trackpad: bool,
    pub mouse: bool,
}

pub fn is_input_device(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with(INPUT_DEVICE_PREFIX))
}

impl PointingDevices {
    pub fn scan(udev_data: &Path) -> Self {
        let mut found = Self::default();
        let Ok(entries) = std::fs::read_dir(udev_data) else {
            return found;
        };
        for path in entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_input_device(path))
        {
            let Ok(properties) = std::fs::read_to_string(path) else {
                continue;
            };
            let has = |property: &str| properties.lines().any(|line| line.trim() == property);
            if has(TOUCHPAD) {
                found.trackpad = true;
            } else if MICE.iter().any(|mouse| has(mouse)) {
                found.mouse = true;
            }
        }
        found
    }
}
