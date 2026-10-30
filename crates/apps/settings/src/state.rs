use unbound_bluetooth::Device;
use unbound_network::Network;

#[derive(Debug, Clone, PartialEq)]
pub struct WifiState {
    pub enabled: bool,
    pub networks: Vec<Network>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NetworkState {
    pub ethernet: Option<unbound_network::Ethernet>,
    pub vpns: Vec<unbound_network::Vpn>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BluetoothState {
    pub powered: bool,
    pub paired: Vec<Device>,
    pub nearby: Vec<Device>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Joined {
    Joined,
    WrongPassword,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    Confirm { device: String, code: String },
    Show { device: String, code: String },
    Pin { device: String },
    Authorize { device: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Accept,
    Decline,
    Pin(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    pub millihertz: i32,
}

impl Mode {
    pub fn hertz(self) -> u32 {
        u32::try_from((self.millihertz + 500) / 1000).unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Display {
    pub connector: String,
    pub identity: String,
    pub make: String,
    pub model: String,
    pub modes: Vec<Mode>,
    pub current: Option<Mode>,
    pub position: (i32, i32),
    pub scale: f64,
    pub rotation: u32,
}

pub const REFRESH_RANGE: (u32, u32) = (60, 240);
const RESOLUTIONS_OFFERED: usize = 4;

impl Display {
    pub fn refresh_rates(&self) -> Vec<u32> {
        let Some(current) = self.current else {
            return Vec::new();
        };
        let mut rates: Vec<u32> = self
            .modes
            .iter()
            .filter(|mode| mode.width == current.width && mode.height == current.height)
            .map(|mode| mode.hertz())
            .filter(|hertz| (REFRESH_RANGE.0..=REFRESH_RANGE.1).contains(hertz))
            .collect();
        rates.sort_unstable();
        rates.dedup();
        rates
    }

    pub fn name(&self) -> String {
        let make = self.make.trim();
        let model = self.model.trim();
        match (make.is_empty(), model.is_empty()) {
            (_, true) if !self.identity.is_empty() => self.identity.clone(),
            (_, true) => self.connector.clone(),
            (true, false) => model.to_owned(),
            (false, false) if model.to_lowercase().contains(&make.to_lowercase()) => {
                model.to_owned()
            }
            (false, false) => format!("{make} {model}"),
        }
    }

    pub fn logical_size(&self) -> (i32, i32) {
        let (width, height) = self
            .current
            .map_or((1, 1), |mode| (mode.width, mode.height));
        let (width, height) = if self.rotation % 180 == 90 {
            (height, width)
        } else {
            (width, height)
        };
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        (
            (f64::from(width) / scale).round() as i32,
            (f64::from(height) / scale).round() as i32,
        )
    }

    pub fn resolutions(&self) -> Vec<(i32, i32)> {
        let mut sizes: Vec<(i32, i32)> = self
            .modes
            .iter()
            .map(|mode| (mode.width, mode.height))
            .filter(|(width, height)| *width > 0 && *height > 0)
            .collect();
        sizes.sort_by_key(|(width, height)| {
            std::cmp::Reverse(i64::from(*width) * i64::from(*height))
        });
        sizes.dedup();
        let Some(&(native_width, native_height)) = sizes.first() else {
            return sizes;
        };
        let shaped = |(width, height): &(i32, i32)| {
            (f64::from(*width) / f64::from(*height)
                - f64::from(native_width) / f64::from(native_height))
            .abs()
                < 0.02
        };
        let current = self.current.map(|mode| (mode.width, mode.height));
        let mut offered: Vec<(i32, i32)> = sizes
            .iter()
            .copied()
            .filter(shaped)
            .take(RESOLUTIONS_OFFERED)
            .collect();
        if let Some(current) = current
            && !offered.contains(&current)
        {
            offered.push(current);
        }
        offered
    }

    pub fn variable_refresh_capable(&self) -> bool {
        self.refresh_rates()
            .last()
            .is_some_and(|fastest| *fastest > REFRESH_RANGE.0)
    }
}
