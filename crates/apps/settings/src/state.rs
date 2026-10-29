use unbound_bluetooth::Device;
use unbound_network::Network;

#[derive(Debug, Clone, PartialEq)]
pub struct WifiState {
    pub enabled: bool,
    pub networks: Vec<Network>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Display {
    pub connector: String,
    pub description: String,
    pub modes: Vec<Mode>,
    pub current: Option<Mode>,
}

pub const REFRESH_RANGE: (u32, u32) = (60, 240);

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

    pub fn variable_refresh_capable(&self) -> bool {
        self.refresh_rates()
            .last()
            .is_some_and(|fastest| *fastest > REFRESH_RANGE.0)
    }
}
