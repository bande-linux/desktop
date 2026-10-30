mod card;
mod model;
mod sense;
mod state;
mod thermal;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use card::{Card, SensePcm};
use model::Model;
use thermal::{Protection, Verdict};

const MODEL_DIRECTORY: &str = "/usr/share/unbound-speaker-guard";
const RUNTIME_STATE: &str = "/run/unbound-speaker-guard/state";
const PERSISTENT_STATE: &str = "/var/lib/unbound-speaker-guard/state";
const WAIT_MS: u32 = 250;
const RUNTIME_SAVE_SECONDS: f64 = 1.0;
const PERSISTENT_SAVE_SECONDS: f64 = 60.0;
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
    last_powers: Vec<f64>,
    channels: usize,
}

impl Guard {
    fn snapshot(&self) -> String {
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
        state::encode(state::now_seconds(), &saved)
    }

    fn apply(&mut self, powers: &[f64], seconds: f64) -> Result<(), String> {
        for (index, (protection, power)) in self.protections.iter_mut().zip(powers).enumerate() {
            match protection.update(*power, seconds) {
                Verdict::Gain(gain_db) => self
                    .card
                    .set_gain(index, gain_db)
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
        self.cool_below_hard_limit(stop);
        let mut last = Instant::now();
        self.card.heartbeat().map_err(|e| e.to_string())?;
        let gains: Vec<f64> = self.protections.iter().map(|p| p.gain_db).collect();
        for (index, gain) in gains.into_iter().enumerate() {
            self.card.set_gain(index, gain).map_err(|e| e.to_string())?;
        }
        while !stop.load(Ordering::Relaxed) {
            let read = match self.pcm.read(&mut buffer, WAIT_MS) {
                Err(error) if error.errno() == Some(EPIPE) => {
                    self.pcm.restart().map_err(|e| e.to_string())?;
                    continue;
                }
                Err(error) if error.errno() == Some(ESTRPIPE) => {
                    self.pcm =
                        SensePcm::open(&self.model, self.channels).map_err(|e| e.to_string())?;
                    continue;
                }
                other => other.map_err(|e| e.to_string())?,
            };
            let now = Instant::now();
            let seconds = now.duration_since(last).as_secs_f64();
            last = now;
            match read {
                None => {
                    let idle = vec![0.0; self.protections.len()];
                    self.apply(&idle, seconds)?;
                }
                Some(frames) => {
                    let samples = &buffer[..frames * self.channels];
                    let powers = self
                        .model
                        .speakers
                        .iter()
                        .map(|s| s.sense.read(samples).map(|r| r.power))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|fault| fault.to_string())?;
                    self.apply(&powers, seconds)?;
                    self.card.heartbeat().map_err(|e| e.to_string())?;
                }
            }
            if runtime_saved.elapsed().as_secs_f64() >= RUNTIME_SAVE_SECONDS {
                let _ = state::write(Path::new(RUNTIME_STATE), &self.snapshot());
                runtime_saved = Instant::now();
            }
            if persistent_saved.elapsed().as_secs_f64() >= PERSISTENT_SAVE_SECONDS {
                let _ = state::write(Path::new(PERSISTENT_STATE), &self.snapshot());
                persistent_saved = Instant::now();
            }
        }
        Ok(())
    }

    fn cool_below_hard_limit(&mut self, stop: &AtomicBool) {
        let wait = std::time::Duration::from_millis(u64::from(WAIT_MS));
        while !stop.load(Ordering::Relaxed)
            && self
                .protections
                .iter()
                .any(|p| p.thermal.exceeds_hard_limit(p.temperatures))
        {
            std::thread::sleep(wait);
            for protection in &mut self.protections {
                protection.temperatures =
                    protection
                        .thermal
                        .advance(protection.temperatures, 0.0, wait.as_secs_f64());
            }
        }
    }

    fn save(&self) {
        let snapshot = self.snapshot();
        for path in [RUNTIME_STATE, PERSISTENT_STATE] {
            if let Err(error) = state::write(Path::new(path), &snapshot) {
                tracing::warn!("cannot save the speaker temperatures to {path}: {error}");
            }
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
    match state::read_newest(
        &[Path::new(RUNTIME_STATE), Path::new(PERSISTENT_STATE)],
        protections.len(),
    ) {
        Some((saved_at, saved)) => {
            let elapsed = state::now_seconds() - saved_at;
            for (protection, saved) in protections.iter_mut().zip(saved) {
                state::resume(protection, saved, elapsed);
            }
        }
        None => tracing::info!("no saved speaker temperatures, starting from the worst case"),
    }
    let card = Card::open(&model).map_err(|e| e.to_string())?;
    let pcm = SensePcm::open(&model, channels).map_err(|e| e.to_string())?;
    let last_powers = vec![0.0; protections.len()];
    Ok(Guard {
        card,
        pcm,
        model,
        protections,
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
