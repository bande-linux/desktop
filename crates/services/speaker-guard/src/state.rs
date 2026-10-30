use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::thermal::{Protection, Temperatures};

const STALE_HEATING_LIMIT: f64 = 2.0;

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

pub fn encode(saved_at: f64, speakers: &[Saved]) -> String {
    let mut text = format!("{saved_at:.3}\n");
    for s in speakers {
        text.push_str(&format!(
            "{:.4} {:.4} {:.3} {:.5}\n",
            s.temperatures.coil, s.temperatures.magnet, s.gain_db, s.power
        ));
    }
    text
}

pub fn decode(text: &str, speakers: usize) -> Option<(f64, Vec<Saved>)> {
    let mut lines = text.lines();
    let saved_at: f64 = lines.next()?.trim().parse().ok()?;
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
    (saved.len() == speakers).then_some((saved_at, saved))
}

pub fn read_newest(paths: &[&Path], speakers: usize) -> Option<(f64, Vec<Saved>)> {
    paths
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| decode(&text, speakers))
        .max_by(|a, b| a.0.total_cmp(&b.0))
}

pub fn write(path: &Path, text: &str) -> std::io::Result<()> {
    let staging = path.with_extension("new");
    std::fs::write(&staging, text)?;
    std::fs::rename(staging, path)
}

pub fn resume(protection: &mut Protection, saved: Saved, elapsed: f64) {
    let thermal = &protection.thermal;
    if !(0.0..=1.0e9).contains(&elapsed) || saved.temperatures.coil < thermal.ambient - 30.0 {
        protection.temperatures = thermal.worst_case();
        protection.gain_db = 0.0;
        return;
    }
    let heating = elapsed.min(STALE_HEATING_LIMIT);
    let warmed = thermal.advance(saved.temperatures, saved.power.max(0.0), heating);
    protection.temperatures = thermal.advance(warmed, 0.0, elapsed - heating);
    protection.gain_db = saved
        .gain_db
        .clamp(-protection.limiter.max_reduction_db, 0.0);
}
