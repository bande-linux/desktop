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

pub(crate) fn from_channel(channel: f32) -> f32 {
    channel.max(0.0).cbrt().min(1.0)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn level(volume: f32) -> Level {
        Level {
            volume,
            muted: false,
        }
    }

    #[test]
    fn keys_move_in_sixteen_steps_snapping_between_and_stopping_at_the_ends() {
        for (from, key, fine, expected) in [
            (0.5, VolumeKey::Up, false, 9.0 / 16.0),
            (0.5, VolumeKey::Down, false, 7.0 / 16.0),
            (0.5, VolumeKey::Up, true, 33.0 / 64.0),
            (0.5, VolumeKey::Down, true, 31.0 / 64.0),
            (0.3, VolumeKey::Up, false, 5.0 / 16.0),
            (0.3, VolumeKey::Down, false, 4.0 / 16.0),
            (0.0, VolumeKey::Down, false, 0.0),
            (1.0, VolumeKey::Up, false, 1.0),
        ] {
            assert_eq!(
                level(from).press(key, fine).volume,
                expected,
                "{from} {key:?} fine={fine}"
            );
        }
    }

    #[test]
    fn a_volume_key_unmutes_and_mute_toggles_keeping_the_level() {
        let muted = level(0.5).press(VolumeKey::Mute, false);
        assert_eq!((muted.volume, muted.muted), (0.5, true));
        assert!(!muted.press(VolumeKey::Mute, false).muted);
        assert!(!muted.press(VolumeKey::Up, false).muted);
        assert!(!muted.press(VolumeKey::Down, false).muted);
    }

    #[test]
    fn levels_map_to_pipewire_volumes_on_the_cubic_curve() {
        assert_eq!(to_channel(0.5), 0.125);
        assert!((from_channel(0.125) - 0.5).abs() < 1e-6);
        assert_eq!(to_channel(1.5), 1.0);
    }

    #[test]
    fn a_new_level_keeps_the_balance_between_channels() {
        assert_eq!(scaled(&[0.5, 0.25], 0.5), [0.125, 0.0625]);
        assert_eq!(scaled(&[0.0, 0.0], 0.5), [0.125, 0.125]);
    }

    #[test]
    fn the_input_level_reads_in_decibels_and_falls_back_smoothly() {
        assert_eq!(meter_level(1.0, 0.0), 1.0);
        assert_eq!(meter_level(0.0, 0.0), 0.0);
        assert!((meter_level(10_f32.powf(-30.0 / 20.0), 0.0) - 0.5).abs() < 1e-4);
        let falling = meter_level(0.0, 1.0);
        assert!(falling > 0.5 && falling < 1.0, "{falling}");
    }
}
