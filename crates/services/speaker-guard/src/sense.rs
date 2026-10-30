#[derive(Debug, Clone, PartialEq)]
pub struct SenseLayout {
    pub channels: usize,
    pub current_channel: usize,
    pub voltage_channel: usize,
    pub current_full_scale: f64,
    pub voltage_full_scale: f64,
    pub dc_resistance: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub power: f64,
    pub voltage_rms: f64,
    pub current_rms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fault {
    NegativePower(f64),
    Impedance(f64),
    FrozenVoltage,
    FrozenCurrent,
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NegativePower(watts) => write!(f, "negative sensed power {watts:.3} W"),
            Self::Impedance(ohms) => write!(f, "implausible speaker impedance {ohms:.2} ohm"),
            Self::FrozenVoltage => write!(f, "voltage sense stream is frozen"),
            Self::FrozenCurrent => write!(f, "current sense stream is frozen"),
        }
    }
}

const IMPEDANCE_CHECK_CURRENT: f64 = 0.05;
const MIN_IMPEDANCE_RATIO: f64 = 0.5;
const MAX_IMPEDANCE_RATIO: f64 = 8.0;
const NEGATIVE_POWER_TOLERANCE: f64 = -0.01;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelRange {
    pub min: i16,
    pub max: i16,
}

impl SenseLayout {
    pub fn ranges(&self, frames: &[i16]) -> (ChannelRange, ChannelRange) {
        let range = |channel: usize| {
            frames.iter().skip(channel).step_by(self.channels).fold(
                ChannelRange {
                    min: i16::MAX,
                    max: i16::MIN,
                },
                |r, &v| ChannelRange {
                    min: r.min.min(v),
                    max: r.max.max(v),
                },
            )
        };
        (range(self.current_channel), range(self.voltage_channel))
    }

    pub fn read(&self, frames: &[i16]) -> Result<Reading, Fault> {
        let mut voltage_square = 0.0;
        let mut current_square = 0.0;
        let mut product = 0.0;
        let mut count = 0usize;
        let mut voltages = frames
            .iter()
            .skip(self.voltage_channel)
            .step_by(self.channels);
        let mut currents = frames
            .iter()
            .skip(self.current_channel)
            .step_by(self.channels);
        let first_voltage = voltages.clone().next().copied();
        let first_current = currents.clone().next().copied();
        let mut voltage_moved = false;
        let mut current_moved = false;
        for (raw_voltage, raw_current) in voltages.by_ref().zip(currents.by_ref()) {
            voltage_moved |= Some(*raw_voltage) != first_voltage;
            current_moved |= Some(*raw_current) != first_current;
            let voltage = f64::from(*raw_voltage) / 32768.0 * self.voltage_full_scale;
            let current = f64::from(*raw_current) / 32768.0 * self.current_full_scale;
            voltage_square += voltage * voltage;
            current_square += current * current;
            product += voltage * current;
            count += 1;
        }
        if count > 1 && !voltage_moved {
            return Err(Fault::FrozenVoltage);
        }
        if count > 1 && !current_moved {
            return Err(Fault::FrozenCurrent);
        }
        let count = count.max(1) as f64;
        let mean_product = product / count;
        if mean_product < NEGATIVE_POWER_TOLERANCE {
            return Err(Fault::NegativePower(mean_product));
        }
        let voltage_rms = (voltage_square / count).sqrt();
        let current_rms = (current_square / count).sqrt();
        if current_rms > IMPEDANCE_CHECK_CURRENT {
            let impedance = voltage_rms / current_rms;
            if !(MIN_IMPEDANCE_RATIO * self.dc_resistance
                ..=MAX_IMPEDANCE_RATIO * self.dc_resistance)
                .contains(&impedance)
            {
                return Err(Fault::Impedance(impedance));
            }
        }
        Ok(Reading {
            power: voltage_rms * voltage_rms / self.dc_resistance,
            voltage_rms,
            current_rms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j473() -> SenseLayout {
        SenseLayout {
            channels: 2,
            current_channel: 0,
            voltage_channel: 1,
            current_full_scale: 3.75,
            voltage_full_scale: 14.0,
            dc_resistance: 4.6,
        }
    }

    fn stream(volts: f64, ohms: f64) -> Vec<i16> {
        (0..1024)
            .flat_map(|n| {
                let v = volts * (n as f64 * 0.13).sin();
                let i = v / ohms;
                [(i / 3.75 * 32768.0) as i16, (v / 14.0 * 32768.0) as i16]
            })
            .collect()
    }

    #[test]
    fn a_dead_sense_channel_is_a_fault_not_a_cold_speaker() {
        let layout = j473();
        let mut dead_current = stream(4.0, 4.6);
        dead_current.iter_mut().step_by(2).for_each(|s| *s = 0);
        assert_eq!(layout.read(&dead_current), Err(Fault::FrozenCurrent));
        assert_eq!(layout.read(&vec![0; 2048]), Err(Fault::FrozenVoltage));
    }

    #[test]
    fn implausible_impedance_or_inverted_current_is_a_fault() {
        let layout = j473();
        assert!(matches!(
            layout.read(&stream(4.0, 0.5)),
            Err(Fault::Impedance(_))
        ));
        let inverted: Vec<i16> = stream(4.0, 4.6)
            .chunks(2)
            .flat_map(|f| [f[0].saturating_neg(), f[1]])
            .collect();
        assert!(matches!(
            layout.read(&inverted),
            Err(Fault::NegativePower(_))
        ));
    }

    #[test]
    fn power_never_underestimates_a_driver_above_its_dc_resistance() {
        let reading = j473().read(&stream(4.0, 6.0));
        assert!(
            reading.is_ok_and(|r| r.power >= r.voltage_rms * r.current_rms),
            "{reading:?}"
        );
    }
}
