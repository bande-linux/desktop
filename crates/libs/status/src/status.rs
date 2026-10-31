use std::time::Duration;

pub use unbound_audio::{Capture, Sense};
use unbound_network::Network;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Power {
    Battery { remaining: Option<Duration> },
    Charging { until_full: Option<Duration> },
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Battery {
    pub percent: f64,
    pub power: Power,
}

impl Battery {
    pub fn on_adapter(&self) -> bool {
        !matches!(self.power, Power::Battery { .. })
    }

    pub fn low(&self) -> bool {
        !self.on_adapter() && self.percent <= 10.0
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Wifi {
    pub enabled: bool,
    pub networks: Vec<Network>,
}

impl Wifi {
    pub fn current(&self) -> Option<&Network> {
        self.networks.iter().find(|network| network.current)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    pub volume: f32,
    pub muted: bool,
    pub outputs: Vec<Output>,
    pub default_output: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gadget {
    pub address: String,
    pub name: String,
    pub connected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bluetooth {
    pub powered: bool,
    pub devices: Vec<Gadget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSource {
    pub name: String,
    pub short: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InputSources {
    pub sources: Vec<InputSource>,
    pub active: usize,
}

impl InputSources {
    pub fn current(&self) -> Option<&InputSource> {
        self.sources
            .get(self.active)
            .filter(|_| self.sources.len() > 1)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub battery: Option<Battery>,
    pub wifi: Option<Wifi>,
    pub sound: Option<Sound>,
    pub bluetooth: Option<Bluetooth>,
    pub input_sources: InputSources,
    pub capturing: Vec<Capture>,
}

pub fn wifi_bars(strength: u8) -> u8 {
    match strength {
        0..34 => 1,
        34..67 => 2,
        _ => 3,
    }
}

pub fn speaker_waves(volume: f32) -> u8 {
    match volume {
        volume if volume <= 0.0 => 0,
        volume if volume < 0.34 => 1,
        volume if volume < 0.67 => 2,
        _ => 3,
    }
}

pub fn hours_and_minutes(duration: Duration) -> String {
    let minutes = duration.as_secs().div_ceil(60);
    format!("{}:{:02}", minutes / 60, minutes % 60)
}
