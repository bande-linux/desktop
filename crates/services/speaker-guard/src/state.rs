use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::thermal::{Protection, Temperatures};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Saved {
    pub temperatures: Temperatures,
    pub gain_db: f64,
    pub power: f64,
}

pub fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs_f64()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub saved_at: f64,
    pub clean: bool,
    pub speakers: Vec<Saved>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Heating {
    pub seconds: f64,
    pub power: f64,
}

pub fn boot_seconds() -> Option<f64> {
    std::fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

pub fn encode(saved_at: f64, clean: bool, speakers: &[Saved]) -> String {
    let mut text = format!("{saved_at:.3} {}\n", u8::from(clean));
    for s in speakers {
        text.push_str(&format!(
            "{:.4} {:.4} {:.3} {:.5}\n",
            s.temperatures.coil, s.temperatures.magnet, s.gain_db, s.power
        ));
    }
    text
}

pub fn decode(text: &str, speakers: usize) -> Option<Snapshot> {
    let mut lines = text.lines();
    let (saved_at, clean) = match lines.next()?.split_whitespace().collect::<Vec<_>>()[..] {
        [saved_at, "0"] => (saved_at.parse::<f64>().ok()?, false),
        [saved_at, "1"] => (saved_at.parse::<f64>().ok()?, true),
        _ => return None,
    };
    let saved = lines
        .map(|line| {
            let fields: Vec<f64> = line
                .split_whitespace()
                .map(str::parse)
                .collect::<Result<_, _>>()
                .ok()?;
            match fields[..] {
                [coil, magnet, gain_db, power]
                    if [coil, magnet, gain_db, power].iter().all(|v| v.is_finite()) =>
                {
                    Some(Saved {
                        temperatures: Temperatures { coil, magnet },
                        gain_db,
                        power,
                    })
                }
                _ => None,
            }
        })
        .collect::<Option<Vec<_>>>()?;
    (saved_at.is_finite() && saved.len() == speakers).then_some(Snapshot {
        saved_at,
        clean,
        speakers: saved,
    })
}

pub fn read(path: &Path, speakers: usize) -> Option<Snapshot> {
    decode(&std::fs::read_to_string(path).ok()?, speakers)
}

pub fn write(path: &Path, text: &str) -> std::io::Result<()> {
    let staging = path.with_extension("new");
    std::fs::write(&staging, text)?;
    std::fs::rename(staging, path)
}

pub fn resume(protection: &mut Protection, saved: Saved, elapsed: f64, heating: &[Heating]) {
    let thermal = &protection.thermal;
    let implausible = |t: f64| t < thermal.ambient - 30.0;
    if !(0.0..=1.0e9).contains(&elapsed)
        || implausible(saved.temperatures.coil)
        || implausible(saved.temperatures.magnet)
    {
        protection.temperatures = thermal.worst_case();
        protection.gain_db = 0.0;
        return;
    }
    let mut temperatures = Temperatures {
        coil: saved.temperatures.coil.max(thermal.ambient),
        magnet: saved.temperatures.magnet.max(thermal.ambient),
    };
    let mut remaining = elapsed;
    for stage in heating {
        let seconds = stage.seconds.min(remaining);
        temperatures = thermal.advance(temperatures, stage.power.max(saved.power), seconds);
        remaining -= seconds;
    }
    protection.temperatures = thermal.advance(temperatures, 0.0, remaining);
    protection.gain_db = saved
        .gain_db
        .clamp(-protection.limiter.max_reduction_db, 0.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thermal::{LimiterParams, ThermalParams};

    fn j473() -> Protection {
        Protection {
            thermal: ThermalParams {
                ambient: 46.0,
                coil_resistance: 40.0,
                coil_time_constant: 3.7,
                magnet_resistance: 60.0,
                magnet_time_constant: 250.0,
                limit: 140.0,
                headroom: 40.0,
            },
            limiter: LimiterParams {
                attack: 1.0,
                release: 20.0,
                max_reduction_db: 20.0,
            },
            temperatures: Temperatures {
                coil: 0.0,
                magnet: 0.0,
            },
            gain_db: 0.0,
        }
    }

    fn saved(coil: f64, magnet: f64) -> Saved {
        Saved {
            temperatures: Temperatures { coil, magnet },
            gain_db: 0.0,
            power: 0.0,
        }
    }

    #[test]
    fn a_restart_after_an_unclean_exit_assumes_the_speaker_kept_heating_until_the_kernel_locked_it()
    {
        let mut protection = j473();
        let continuous = protection.thermal.max_continuous_power();
        let heating = [
            Heating {
                seconds: 60.0,
                power: continuous,
            },
            Heating {
                seconds: 0.25,
                power: 54.7,
            },
        ];
        resume(&mut protection, saved(100.0, 80.0), 30.0, &heating);
        let mut expected = j473().thermal.advance(
            Temperatures {
                coil: 100.0,
                magnet: 80.0,
            },
            continuous,
            30.0,
        );
        expected.coil = expected.coil.max(100.0);
        assert!(protection.temperatures.coil >= expected.coil);
        assert!(protection.temperatures.magnet > 80.0);
    }

    #[test]
    fn a_clock_that_went_backwards_or_an_implausible_magnet_restores_the_worst_case() {
        let worst = j473().thermal.worst_case();
        let mut protection = j473();
        resume(&mut protection, saved(60.0, 50.0), -5.0, &[]);
        assert_eq!(protection.temperatures, worst);
        let mut protection = j473();
        resume(&mut protection, saved(60.0, -40.0), 5.0, &[]);
        assert_eq!(protection.temperatures, worst);
    }
}
