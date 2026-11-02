const STEPS: f32 = 16.0;
const FINE_STEPS: f32 = 64.0;
const SNAP: f32 = 1e-3;
const METER_FLOOR_DB: f32 = -60.0;
const METER_FALL: f32 = 0.82;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeKey {
    Up,
    Down,
    Mute,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub volume: f32,
    pub muted: bool,
}

impl Level {
    pub fn press(self, key: VolumeKey, fine: bool) -> Self {
        let steps = if fine { FINE_STEPS } else { STEPS };
        match key {
            VolumeKey::Up => Self {
                volume: (((self.volume * steps + SNAP).floor() + 1.0) / steps).min(1.0),
                muted: false,
            },
            VolumeKey::Down => Self {
                volume: (((self.volume * steps - SNAP).ceil() - 1.0) / steps).max(0.0),
                muted: false,
            },
            VolumeKey::Mute => Self {
                muted: !self.muted,
                ..self
            },
        }
    }
}

use pipewire::spa::sys::{SPA_AUDIO_CHANNEL_FL, SPA_AUDIO_CHANNEL_FR};

use crate::graph::Volumes;

fn from_channel(channel: f32) -> f32 {
    channel.max(0.0).cbrt().min(1.0)
}

pub(crate) fn loudest(channels: &[f32]) -> f32 {
    from_channel(channels.iter().copied().fold(0.0, f32::max))
}

pub(crate) fn balance(volumes: &Volumes) -> Option<f32> {
    let ([left, right], [SPA_AUDIO_CHANNEL_FL, SPA_AUDIO_CHANNEL_FR]) = (
        <[f32; 2]>::try_from(volumes.channels.as_slice()).ok()?,
        <[u32; 2]>::try_from(volumes.positions.as_slice()).ok()?,
    ) else {
        return None;
    };
    let (left, right) = (from_channel(left), from_channel(right));
    let loudest = left.max(right);
    Some(if loudest > 0.0 {
        (right - left) / loudest
    } else {
        0.0
    })
}

pub(crate) fn balanced(volumes: &Volumes, balance: f32) -> Option<Vec<f32>> {
    self::balance(volumes)?;
    let volume = loudest(&volumes.channels);
    let balance = balance.clamp(-1.0, 1.0);
    Some(vec![
        to_channel(volume * (1.0 - balance.max(0.0))),
        to_channel(volume * (1.0 + balance.min(0.0))),
    ])
}

pub(crate) fn to_channel(volume: f32) -> f32 {
    volume.clamp(0.0, 1.0).powi(3)
}

pub(crate) fn scaled(channels: &[f32], volume: f32) -> Vec<f32> {
    let target = to_channel(volume);
    let loudest = channels.iter().copied().fold(0.0, f32::max);
    if loudest > 0.0 {
        channels
            .iter()
            .map(|channel| channel * target / loudest)
            .collect()
    } else {
        vec![target; channels.len().max(1)]
    }
}

pub fn meter_level(peak: f32, shown: f32) -> f32 {
    let decibels = 20.0 * peak.max(f32::MIN_POSITIVE).log10();
    let level = (1.0 - decibels / METER_FLOOR_DB).clamp(0.0, 1.0);
    level.max(shown * METER_FALL)
}
