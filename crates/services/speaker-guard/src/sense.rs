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
pub struct ChannelRange {
    pub min: i16,
    pub max: i16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    pub current: ChannelRange,
    pub voltage: ChannelRange,
    pub current_rms: f64,
    pub voltage_rms: f64,
    pub mean_product: f64,
    pub open_loop_power: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fault {
    InvertedCurrent { product: f64, open_loop: f64 },
    Impedance(f64),
    FrozenVoltage,
    FrozenCurrent,
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvertedCurrent { product, open_loop } => write!(
                f,
                "sensed V·I averages {product:.4} W against {open_loop:.4} W from V alone: the current sense is inverted"
            ),
            Self::Impedance(ohms) => write!(f, "implausible speaker impedance {ohms:.2} ohm"),
            Self::FrozenVoltage => write!(f, "voltage sense stream is frozen"),
            Self::FrozenCurrent => write!(f, "current sense stream is frozen"),
        }
    }
}

const STARTUP_GRACE_FRAMES: usize = 2400;
const MIN_JUDGED_FRAMES: usize = 256;
const POLARITY_WINDOW_SECONDS: f64 = 0.5;
const POLARITY_MIN_POWER: f64 = 0.001;
const POLARITY_RATIO: f64 = -0.25;
const IMPEDANCE_CHECK_VOLTAGE: f64 = 0.2;
const MIN_IMPEDANCE_RATIO: f64 = 0.8;
const MAX_IMPEDANCE_RATIO: f64 = 8.0;

impl ChannelRange {
    fn clipped(&self) -> bool {
        self.min <= -i16::MAX || self.max == i16::MAX
    }
}

impl SenseLayout {
    pub fn measure(&self, frames: &[i16]) -> Measurement {
        let samples = |channel: usize| frames.iter().skip(channel).step_by(self.channels).copied();
        let range = |channel: usize| {
            samples(channel).fold(
                ChannelRange {
                    min: i16::MAX,
                    max: i16::MIN,
                },
                |r, v| ChannelRange {
                    min: r.min.min(v),
                    max: r.max.max(v),
                },
            )
        };
        let volts = |raw: i16| f64::from(raw) / 32768.0 * self.voltage_full_scale;
        let amps = |raw: i16| f64::from(raw) / 32768.0 * self.current_full_scale;
        let (count, voltage_square, current_square, product) = samples(self.voltage_channel)
            .zip(samples(self.current_channel))
            .fold((0usize, 0.0, 0.0, 0.0), |(n, vv, ii, vi), (v, i)| {
                (
                    n + 1,
                    vv + volts(v) * volts(v),
                    ii + amps(i) * amps(i),
                    vi + volts(v) * amps(i),
                )
            });
        let count = count.max(1) as f64;
        let voltage_rms = (voltage_square / count).sqrt();
        Measurement {
            current: range(self.current_channel),
            voltage: range(self.voltage_channel),
            current_rms: (current_square / count).sqrt(),
            voltage_rms,
            mean_product: product / count,
            open_loop_power: voltage_rms * voltage_rms / self.dc_resistance,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SenseMonitor {
    pub layout: SenseLayout,
    pub worst_case_power: f64,
    stream_frames: usize,
    voltage_seen_live: bool,
    current_seen_live: bool,
    window_seconds: f64,
    window_product: f64,
    window_open_loop: f64,
}

impl SenseMonitor {
    pub fn new(layout: SenseLayout, worst_case_power: f64) -> Self {
        Self {
            layout,
            worst_case_power,
            stream_frames: 0,
            voltage_seen_live: false,
            current_seen_live: false,
            window_seconds: 0.0,
            window_product: 0.0,
            window_open_loop: 0.0,
        }
    }

    pub fn stream_paused(&mut self) {
        *self = Self::new(self.layout.clone(), self.worst_case_power);
    }

    pub fn assess(
        &mut self,
        measurement: &Measurement,
        frames: usize,
        seconds: f64,
    ) -> Result<f64, Fault> {
        self.stream_frames += frames;
        let in_grace = self.stream_frames <= STARTUP_GRACE_FRAMES;
        let judged = frames >= MIN_JUDGED_FRAMES;
        let voltage_frozen = judged && measurement.voltage.min == measurement.voltage.max;
        let current_frozen = judged && measurement.current.min == measurement.current.max;
        if voltage_frozen && (self.voltage_seen_live || !in_grace) {
            return Err(Fault::FrozenVoltage);
        }
        if current_frozen && (self.current_seen_live || !in_grace) {
            return Err(Fault::FrozenCurrent);
        }
        self.voltage_seen_live |= !voltage_frozen;
        self.current_seen_live |= !current_frozen;
        if measurement.voltage.clipped() || measurement.current.clipped() {
            return Ok(self.worst_case_power);
        }
        if measurement.voltage_rms > IMPEDANCE_CHECK_VOLTAGE {
            let impedance = measurement.voltage_rms / measurement.current_rms.max(1e-6);
            let resistance = self.layout.dc_resistance;
            if !(MIN_IMPEDANCE_RATIO * resistance..=MAX_IMPEDANCE_RATIO * resistance)
                .contains(&impedance)
            {
                return Err(Fault::Impedance(impedance));
            }
        }
        self.window_seconds += seconds;
        self.window_product += measurement.mean_product * seconds;
        self.window_open_loop += measurement.open_loop_power * seconds;
        if self.window_seconds >= POLARITY_WINDOW_SECONDS {
            let product = self.window_product / self.window_seconds;
            let open_loop = self.window_open_loop / self.window_seconds;
            self.window_seconds = 0.0;
            self.window_product = 0.0;
            self.window_open_loop = 0.0;
            if open_loop > POLARITY_MIN_POWER && product < POLARITY_RATIO * open_loop {
                return Err(Fault::InvertedCurrent { product, open_loop });
            }
        }
        Ok(if voltage_frozen {
            self.worst_case_power
        } else {
            measurement.open_loop_power
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERIOD: f64 = 1024.0 / 48_000.0;

    fn j473() -> SenseMonitor {
        let layout = SenseLayout {
            channels: 2,
            current_channel: 0,
            voltage_channel: 1,
            current_full_scale: 3.75,
            voltage_full_scale: 14.0,
            dc_resistance: 4.6,
        };
        SenseMonitor::new(layout, 15.87 * 15.87 / 4.6)
    }

    fn stream(volts: f64, ohms: f64, current_sign: f64) -> Vec<i16> {
        (0..1024)
            .flat_map(|n| {
                let v = volts * (n as f64 * 0.13).sin();
                let noise = ((n * 7919) % 17) as i16 - 8;
                [
                    (current_sign * v / ohms / 3.75 * 32768.0) as i16 + noise,
                    (v / 14.0 * 32768.0) as i16 + noise,
                ]
            })
            .collect()
    }

    fn assess(monitor: &mut SenseMonitor, frames: &[i16]) -> Result<f64, Fault> {
        let measurement = monitor.layout.measure(frames);
        monitor.assess(&measurement, 1024, PERIOD)
    }

    #[test]
    fn the_silent_start_of_a_stream_is_tolerated_and_costs_worst_case_power() {
        let mut monitor = j473();
        let power = assess(&mut monitor, &vec![0; 2048]);
        assert_eq!(power, Ok(monitor.worst_case_power));
        assert!(assess(&mut monitor, &stream(0.1, 4.6, 1.0)).is_ok());
    }

    #[test]
    fn a_channel_that_never_comes_alive_or_stops_is_a_fault() {
        let mut monitor = j473();
        let mut dead_current = stream(0.1, 4.6, 1.0);
        dead_current.iter_mut().step_by(2).for_each(|s| *s = 0);
        let results: Vec<_> = (0..4)
            .map(|_| assess(&mut monitor, &dead_current))
            .collect();
        assert_eq!(results.last(), Some(&Err(Fault::FrozenCurrent)));

        let mut monitor = j473();
        assert!(assess(&mut monitor, &stream(0.1, 4.6, 1.0)).is_ok());
        assert_eq!(
            assess(&mut monitor, &vec![0; 2048]),
            Err(Fault::FrozenVoltage)
        );
    }

    #[test]
    fn an_inverted_current_sense_is_a_fault_but_a_transient_is_not() {
        let mut monitor = j473();
        let inverted = stream(1.0, 4.6, -1.0);
        let fault = (0..30).find_map(|_| assess(&mut monitor, &inverted).err());
        assert!(
            matches!(fault, Some(Fault::InvertedCurrent { .. })),
            "{fault:?}"
        );

        let mut monitor = j473();
        assert!(assess(&mut monitor, &inverted).is_ok());
        let normal = stream(1.0, 4.6, 1.0);
        assert!((0..30).all(|_| assess(&mut monitor, &normal).is_ok()));
    }

    #[test]
    fn swapped_voltage_and_current_channels_are_a_fault() {
        let mut monitor = j473();
        let swapped: Vec<i16> = stream(1.0, 4.6, 1.0)
            .chunks(2)
            .flat_map(|frame| [frame[1], frame[0]])
            .collect();
        assert!(matches!(
            assess(&mut monitor, &swapped),
            Err(Fault::Impedance(_))
        ));
    }

    #[test]
    fn a_clipped_sense_period_costs_worst_case_power() {
        let mut monitor = j473();
        let mut clipped = stream(1.0, 4.6, 1.0);
        clipped[1] = i16::MAX;
        assert_eq!(assess(&mut monitor, &clipped), Ok(monitor.worst_case_power));
    }

    #[test]
    fn implausible_impedance_is_a_fault() {
        let mut monitor = j473();
        assert!(matches!(
            assess(&mut monitor, &stream(1.0, 0.5, 1.0)),
            Err(Fault::Impedance(_))
        ));
    }

    #[test]
    fn power_never_underestimates_a_driver_above_its_dc_resistance() {
        let monitor = j473();
        let m = monitor.layout.measure(&stream(4.0, 6.0, 1.0));
        assert!(m.open_loop_power >= m.voltage_rms * m.current_rms);
    }
}
