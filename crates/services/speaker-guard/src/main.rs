mod card;
mod model;
mod sense;
mod state;
mod thermal;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use card::{Card, SensePcm};
use model::Model;
use sense::SenseMonitor;
use thermal::{Protection, Verdict};

const MODEL_DIRECTORY: &str = "/usr/share/unbound-speaker-guard";
const RUNTIME_STATE: &str = "/run/unbound-speaker-guard/state";
const PERSISTENT_STATE: &str = "/var/lib/unbound-speaker-guard/state";
const WAIT_MS: u32 = 250;
const RUNTIME_SAVE_SECONDS: f64 = 1.0;
const PERSISTENT_SAVE_SECONDS: f64 = 60.0;
const LOCK_TIMEOUT_SECONDS: f64 = 0.25;
const EINTR: i32 = 4;
const EPIPE: i32 = 32;
const ESTRPIPE: i32 = 86;

fn machine_model() -> Option<String> {
    let compatible = std::fs::read("/proc/device-tree/compatible").ok()?;
    let first = compatible.split(|b| *b == 0).next()?;
    let first = std::str::from_utf8(first).ok()?;
    let (_, model) = first.split_once(',')?;
    Some(
        model
            .trim_end_matches(|c: char| c.is_ascii_alphabetic())
            .to_owned(),
    )
}

struct Guard {
    card: Card,
    pcm: SensePcm,
    model: Model,
    protections: Vec<Protection>,
    monitors: Vec<SenseMonitor>,
    last_powers: Vec<f64>,
    channels: usize,
}

impl Guard {
    fn snapshot(&self, saved_at: f64, clean: bool) -> String {
        let saved: Vec<state::Saved> = self
            .protections
            .iter()
            .zip(&self.last_powers)
            .map(|(p, power)| state::Saved {
                temperatures: p.temperatures,
                gain_db: p.gain_db,
                power: *power,
            })
            .collect();
        state::encode(saved_at, clean, &saved)
    }

    fn apply(&mut self, powers: &[f64], seconds: f64) -> Result<(), String> {
        for (index, (protection, power)) in self.protections.iter_mut().zip(powers).enumerate() {
            match protection.update(*power, seconds) {
                Verdict::Gain(gain_db) => self
                    .card
                    .set_gain(index, gain_db.min(self.model.max_gain_db))
                    .map_err(|e| e.to_string())?,
                Verdict::HardLimit(t) => {
                    return Err(format!(
                        "speaker {index} passed its hard temperature limit (coil {:.1} °C, magnet {:.1} °C)",
                        t.coil, t.magnet
                    ));
                }
            }
        }
        self.last_powers = powers.to_vec();
        Ok(())
    }

    fn run(&mut self, stop: &AtomicBool) -> Result<(), String> {
        let mut buffer = vec![0i16; self.model.period_frames * self.channels];
        let mut runtime_saved = Instant::now();
        let mut persistent_saved = Instant::now();
        let wait = Duration::from_millis(u64::from(WAIT_MS));
        if !cooled_below_hard_limit(&mut self.protections, stop, wait, std::thread::sleep) {
            return Ok(());
        }
        let mut last = Instant::now();
        self.card.heartbeat().map_err(|e| e.to_string())?;
        let gains: Vec<f64> = self.protections.iter().map(|p| p.gain_db).collect();
        for (index, gain) in gains.into_iter().enumerate() {
            self.card
                .set_gain(index, gain.min(self.model.max_gain_db))
                .map_err(|e| e.to_string())?;
        }
        while !stop.load(Ordering::Relaxed) {
            let read = match self.pcm.read(&mut buffer, WAIT_MS) {
                Err(error) if error.errno() == Some(EPIPE) => {
                    self.pcm.restart().map_err(|e| e.to_string())?;
                    self.monitors
                        .iter_mut()
                        .for_each(SenseMonitor::stream_paused);
                    continue;
                }
                Err(error) if error.errno() == Some(ESTRPIPE) => {
                    self.pcm.resume().map_err(|e| e.to_string())?;
                    self.monitors
                        .iter_mut()
                        .for_each(SenseMonitor::stream_paused);
                    continue;
                }
                Err(error) if error.errno() == Some(EINTR) => continue,
                Ok(Some(0)) => continue,
                other => other.map_err(|e| e.to_string())?,
            };
            let now = Instant::now();
            let seconds = now.duration_since(last).as_secs_f64();
            last = now;
            match read {
                None => {
                    self.monitors
                        .iter_mut()
                        .for_each(SenseMonitor::stream_paused);
                    let idle = vec![0.0; self.protections.len()];
                    self.apply(&idle, seconds)?;
                    self.card.heartbeat().map_err(|e| e.to_string())?;
                }
                Some(frames) => {
                    let samples = &buffer[..frames * self.channels];
                    let mut powers = Vec::with_capacity(self.monitors.len());
                    for (index, monitor) in self.monitors.iter_mut().enumerate() {
                        let m = monitor.layout.measure(samples);
                        let applied_db =
                            self.protections[index].gain_db.min(self.model.max_gain_db);
                        let worst_case =
                            monitor.worst_case_power * thermal::db_to_power(applied_db);
                        tracing::debug!(
                            speaker = index,
                            frames,
                            current_min = m.current.min,
                            current_max = m.current.max,
                            voltage_min = m.voltage.min,
                            voltage_max = m.voltage.max,
                            volts_rms = m.voltage_rms,
                            amps_rms = m.current_rms,
                            ohms = m.voltage_rms / m.current_rms.max(1e-6),
                            mean_product_watts = m.mean_product,
                            open_loop_watts = m.open_loop_power,
                            "sense period"
                        );
                        powers.push(
                            monitor
                                .assess(&m, frames, seconds, worst_case)
                                .map_err(|fault| fault.to_string())?,
                        );
                    }
                    self.apply(&powers, seconds)?;
                    for (index, protection) in self.protections.iter().enumerate() {
                        tracing::debug!(
                            speaker = index,
                            watts = powers.get(index).copied().unwrap_or(0.0),
                            coil_celsius = protection.temperatures.coil,
                            magnet_celsius = protection.temperatures.magnet,
                            gain_db = protection.gain_db,
                            "protection"
                        );
                    }
                    self.card.heartbeat().map_err(|e| e.to_string())?;
                }
            }
            if runtime_saved.elapsed().as_secs_f64() >= RUNTIME_SAVE_SECONDS {
                if let Some(now) = state::boot_seconds() {
                    let _ = state::write(Path::new(RUNTIME_STATE), &self.snapshot(now, false));
                }
                runtime_saved = Instant::now();
            }
            if persistent_saved.elapsed().as_secs_f64() >= PERSISTENT_SAVE_SECONDS {
                let _ = state::write(
                    Path::new(PERSISTENT_STATE),
                    &self.snapshot(state::now_seconds(), false),
                );
                persistent_saved = Instant::now();
            }
        }
        Ok(())
    }

    fn save(&self) {
        let snapshots = [
            (RUNTIME_STATE, state::boot_seconds()),
            (PERSISTENT_STATE, Some(state::now_seconds())),
        ];
        for (path, now) in snapshots {
            let Some(now) = now else { continue };
            if let Err(error) = state::write(Path::new(path), &self.snapshot(now, true)) {
                tracing::warn!("cannot save the speaker temperatures to {path}: {error}");
            }
        }
    }
}

fn cooled_below_hard_limit(
    protections: &mut [Protection],
    stop: &AtomicBool,
    wait: Duration,
    mut sleep: impl FnMut(Duration),
) -> bool {
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        if !protections
            .iter()
            .any(|p| p.thermal.exceeds_hard_limit(p.temperatures))
        {
            return true;
        }
        sleep(wait);
        for protection in protections.iter_mut() {
            protection.temperatures =
                protection
                    .thermal
                    .advance(protection.temperatures, 0.0, wait.as_secs_f64());
        }
    }
}

fn start() -> Result<Guard, String> {
    let machine = machine_model().ok_or("cannot read the machine model from the device tree")?;
    let path = PathBuf::from(MODEL_DIRECTORY).join(format!("{machine}.kdl"));
    let model = Model::load(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let channels = model.speakers.first().map_or(0, |s| s.sense.channels);
    let mut protections: Vec<Protection> = model
        .speakers
        .iter()
        .map(|s| Protection {
            thermal: s.thermal.clone(),
            limiter: s.limiter.clone(),
            temperatures: s.thermal.worst_case(),
            gain_db: 0.0,
        })
        .collect();
    let worst_case_power = |s: &model::Speaker| {
        model.amplifier_sensitivity * model.amplifier_sensitivity / s.sense.dc_resistance
    };
    let restored = state::read(Path::new(RUNTIME_STATE), protections.len())
        .zip(state::boot_seconds())
        .map(|(snapshot, now)| (snapshot, now, RUNTIME_SAVE_SECONDS))
        .or_else(|| {
            state::read(Path::new(PERSISTENT_STATE), protections.len())
                .map(|snapshot| (snapshot, state::now_seconds(), PERSISTENT_SAVE_SECONDS))
        });
    match restored {
        Some((snapshot, now, interval)) => {
            let elapsed = now - snapshot.saved_at;
            for ((protection, saved), speaker) in protections
                .iter_mut()
                .zip(snapshot.speakers)
                .zip(&model.speakers)
            {
                let heating = if snapshot.clean {
                    Vec::new()
                } else {
                    vec![
                        state::Heating {
                            seconds: interval,
                            power: speaker.thermal.max_continuous_power(),
                        },
                        state::Heating {
                            seconds: LOCK_TIMEOUT_SECONDS,
                            power: worst_case_power(speaker),
                        },
                    ]
                };
                state::resume(protection, saved, elapsed, &heating);
            }
        }
        None => tracing::info!("no saved speaker temperatures, starting from the worst case"),
    }
    let card = Card::open(&model).map_err(|e| e.to_string())?;
    let pcm = SensePcm::open(&model, channels).map_err(|e| e.to_string())?;
    let monitors = model
        .speakers
        .iter()
        .map(|s| SenseMonitor::new(s.sense.clone(), worst_case_power(s)))
        .collect();
    let last_powers = vec![0.0; protections.len()];
    Ok(Guard {
        card,
        pcm,
        model,
        protections,
        monitors,
        last_powers,
        channels,
    })
}

fn main() -> ExitCode {
    unbound_log::init("unbound-speaker-guard");
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        if let Err(error) = signal_hook::flag::register(signal, Arc::clone(&stop)) {
            tracing::error!("cannot handle signal {signal}: {error}");
            return ExitCode::FAILURE;
        }
    }
    let mut guard = match start() {
        Ok(guard) => guard,
        Err(error) => {
            tracing::error!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let result = guard.run(&stop);
    guard.save();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thermal::{LimiterParams, Temperatures, ThermalParams};

    fn hot() -> Protection {
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
                coil: 200.0,
                magnet: 120.0,
            },
            gain_db: 0.0,
        }
    }

    #[test]
    fn a_hot_speaker_is_cooled_below_its_hard_limit_before_unlocking() {
        let mut protections = vec![hot()];
        let stop = AtomicBool::new(false);
        let mut slept = 0;
        let cooled =
            cooled_below_hard_limit(&mut protections, &stop, Duration::from_millis(250), |_| {
                slept += 1
            });
        assert!(cooled);
        assert!(slept > 0);
        let p = &protections[0];
        assert!(!p.thermal.exceeds_hard_limit(p.temperatures));
    }

    #[test]
    fn a_stop_while_cooling_never_reports_the_speaker_cool() {
        let mut protections = vec![hot()];
        let stop = AtomicBool::new(false);
        let cooled =
            cooled_below_hard_limit(&mut protections, &stop, Duration::from_millis(250), |_| {
                stop.store(true, Ordering::Relaxed)
            });
        assert!(!cooled);
        let p = &protections[0];
        assert!(p.thermal.exceeds_hard_limit(p.temperatures));
    }
}
