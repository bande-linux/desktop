use std::collections::BTreeMap;
use std::io::{self, Read};

const TIME_MARK: &str = "dcp:iomfb_swap_";
const SWAP_ID: &str = "swap_id=";
const OUTLIER_SHARE: f64 = 0.125;
const FIT_ROUNDS: usize = 4;
const CUTOFF_BIN_MS: f64 = 0.2;
const CUTOFF_TABLE_END_MS: f64 = 4.0;
const SKIP_GAP_REFRESHES: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Swap {
    submitted: f64,
    completed: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Grid {
    origin: f64,
    period: f64,
}

impl Grid {
    fn next_after(&self, time: f64) -> f64 {
        self.origin + self.period * ((time - self.origin) / self.period).ceil()
    }
}

fn parse_line(line: &str) -> Option<(f64, &str, u64)> {
    let (before, after) = line.split_once(TIME_MARK)?;
    let time = before
        .trim_end()
        .strip_suffix(':')?
        .rsplit(char::is_whitespace)
        .next()?
        .parse::<f64>()
        .ok()?;
    let (kind, rest) = after.split_once(':')?;
    let (_, id) = rest.split_once(SWAP_ID)?;
    let id = id
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some((time * 1e3, kind, id))
}

fn swaps_of(text: &str) -> Vec<Swap> {
    let mut submitted = BTreeMap::new();
    let mut swaps = Vec::new();
    for (time, kind, id) in text.lines().filter_map(parse_line) {
        match kind {
            "submit" => {
                submitted.insert(id, time);
            }
            "complete" => {
                if let Some(start) = submitted.remove(&id) {
                    swaps.push(Swap {
                        submitted: start,
                        completed: time,
                    });
                }
            }
            _ => {}
        }
    }
    swaps
}

fn sorted(mut values: Vec<f64>) -> Vec<f64> {
    values.sort_by(f64::total_cmp);
    values
}

fn at(values: &[f64], share: f64) -> f64 {
    let index = (values.len().saturating_sub(1) as f64 * share).round() as usize;
    values.get(index).copied().unwrap_or(0.0)
}

fn shortest_gap(completes: &[f64]) -> Option<f64> {
    let gaps = sorted(completes.windows(2).map(|pair| pair[1] - pair[0]).collect());
    let quick = &gaps[..gaps.len().div_ceil(4)];
    (!quick.is_empty()).then(|| at(quick, 0.5))
}

fn slots_of(completes: &[f64], period: f64) -> Vec<f64> {
    let mut slot = 0.0;
    let mut slots = vec![slot];
    for pair in completes.windows(2) {
        slot += ((pair[1] - pair[0]) / period).round();
        slots.push(slot);
    }
    slots
}

fn line_through(points: &[(f64, f64)]) -> Option<Grid> {
    let count = points.len() as f64;
    let mean_slot = points.iter().map(|point| point.0).sum::<f64>() / count;
    let mean_time = points.iter().map(|point| point.1).sum::<f64>() / count;
    let spread: f64 = points.iter().map(|p| (p.0 - mean_slot).powi(2)).sum();
    if spread == 0.0 {
        return None;
    }
    let period = points
        .iter()
        .map(|p| (p.0 - mean_slot) * (p.1 - mean_time))
        .sum::<f64>()
        / spread;
    Some(Grid {
        origin: mean_time - mean_slot * period,
        period,
    })
}

fn fit_grid(completes: &[f64]) -> Option<Grid> {
    let mut period = shortest_gap(completes)?;
    let mut grid = None;
    for _ in 0..FIT_ROUNDS {
        let slots = slots_of(completes, period);
        let points: Vec<(f64, f64)> = slots
            .iter()
            .copied()
            .zip(completes.iter().copied())
            .collect();
        let kept: Vec<(f64, f64)> = match grid {
            Some(Grid { origin, period }) => points
                .iter()
                .copied()
                .filter(|(slot, time)| {
                    (time - (origin + slot * period)).abs() < period * OUTLIER_SHARE
                })
                .collect(),
            None => points,
        };
        let line = line_through(&kept)?;
        period = line.period;
        grid = Some(line);
    }
    grid
}

struct Report {
    grid: Grid,
    swaps: usize,
    seconds: f64,
    residual_p50: f64,
    residual_p99: f64,
    over_a_refresh: usize,
    missed: usize,
    gaps_of_one: usize,
    gaps_missed: usize,
    gaps_late: usize,
    gaps_idle: usize,
    gaps_long: usize,
    hit_slack_p50: f64,
    hit_slack_mean: f64,
    hit_slack_min: f64,
    miss_slack_p50: f64,
    cutoff_bins: BTreeMap<u64, (usize, usize)>,
    submit_to_complete: BTreeMap<u64, usize>,
}

impl Report {
    fn of(swaps: &[Swap]) -> Option<Self> {
        let completes: Vec<f64> = swaps.iter().map(|swap| swap.completed).collect();
        let grid = fit_grid(&completes)?;
        let residuals = sorted(
            completes
                .iter()
                .map(|time| {
                    let slot = ((time - grid.origin) / grid.period).round();
                    time - (grid.origin + slot * grid.period)
                })
                .collect(),
        );
        let slack = |swap: &Swap| grid.next_after(swap.submitted) - swap.submitted;
        let reached =
            |swap: &Swap| swap.completed - swap.submitted < slack(swap) + grid.period / 2.0;
        let mut hits = Vec::new();
        let mut misses = Vec::new();
        let mut cutoff_bins: BTreeMap<u64, (usize, usize)> = BTreeMap::new();
        let mut submit_to_complete: BTreeMap<u64, usize> = BTreeMap::new();
        for swap in swaps {
            let left = slack(swap);
            let bin = cutoff_bins
                .entry((left / CUTOFF_BIN_MS) as u64)
                .or_default();
            bin.0 += 1;
            if reached(swap) {
                hits.push(left);
            } else {
                bin.1 += 1;
                misses.push(left);
            }
            *submit_to_complete
                .entry((swap.completed - swap.submitted) as u64)
                .or_default() += 1;
        }
        let (mut gaps_of_one, mut gaps_missed, mut gaps_late, mut gaps_idle, mut gaps_long) =
            (0, 0, 0, 0, 0);
        for pair in swaps.windows(2) {
            let refreshes = ((pair[1].completed - pair[0].completed) / grid.period).round() as u64;
            match refreshes {
                0 | 1 => gaps_of_one += 1,
                _ if refreshes > SKIP_GAP_REFRESHES => gaps_idle += 1,
                _ if !reached(&pair[1]) => gaps_missed += 1,
                2 => gaps_late += 1,
                _ => gaps_long += 1,
            }
        }
        let over_a_refresh = swaps
            .iter()
            .filter(|swap| swap.completed - swap.submitted > grid.period)
            .count();
        let hits = sorted(hits);
        let misses = sorted(misses);
        Some(Self {
            grid,
            swaps: swaps.len(),
            seconds: (completes.last()? - completes.first()?) / 1e3,
            residual_p50: at(&residuals, 0.5),
            residual_p99: at(&residuals, 0.99),
            over_a_refresh,
            missed: misses.len(),
            gaps_of_one,
            gaps_missed,
            gaps_late,
            gaps_idle,
            gaps_long,
            hit_slack_p50: at(&hits, 0.5),
            hit_slack_mean: hits.iter().sum::<f64>() / hits.len().max(1) as f64,
            hit_slack_min: hits.first().copied().unwrap_or(0.0),
            miss_slack_p50: at(&misses, 0.5),
            cutoff_bins,
            submit_to_complete,
        })
    }

    fn print(&self) {
        let share = |part: usize, whole: usize| 100.0 * part as f64 / whole.max(1) as f64;
        println!(
            "refresh {:.3} Hz ({:.4} ms), flip callback off the fit: p50 {:.3} ms, p99 {:.3} ms",
            1e3 / self.grid.period,
            self.grid.period,
            self.residual_p50,
            self.residual_p99
        );
        println!(
            "swaps {} in {:.1} s ({:.1}/s)",
            self.swaps,
            self.seconds,
            self.swaps as f64 / self.seconds.max(f64::MIN_POSITIVE)
        );
        println!(
            "over one refresh, submit to complete: {} ({:.1} %)",
            self.over_a_refresh,
            share(self.over_a_refresh, self.swaps)
        );
        println!(
            "missed the next flip: {} ({:.1} %)",
            self.missed,
            share(self.missed, self.swaps)
        );
        let gaps =
            self.gaps_of_one + self.gaps_missed + self.gaps_late + self.gaps_idle + self.gaps_long;
        println!(
            "between flips: one refresh {} ({:.1} %), missed flip {} ({:.1} %), sent a refresh late or nothing to draw {} ({:.1} %), 3-{} refreshes {}, idle {}",
            self.gaps_of_one,
            share(self.gaps_of_one, gaps),
            self.gaps_missed,
            share(self.gaps_missed, gaps),
            self.gaps_late,
            share(self.gaps_late, gaps),
            SKIP_GAP_REFRESHES,
            self.gaps_long,
            self.gaps_idle
        );
        println!(
            "time left before the flip on a hit: p50 {:.2} ms, mean {:.2} ms, least {:.2} ms; on a miss p50 {:.2} ms",
            self.hit_slack_p50, self.hit_slack_mean, self.hit_slack_min, self.miss_slack_p50
        );
        println!("left before the flip: swaps, missed");
        for (bin, (count, missed)) in &self.cutoff_bins {
            let from = *bin as f64 * CUTOFF_BIN_MS;
            if from < CUTOFF_TABLE_END_MS {
                println!(
                    "  {from:.1}-{:.1} ms: {count} {missed}",
                    from + CUTOFF_BIN_MS
                );
            }
        }
        println!("submit to complete, whole ms: swaps");
        for (ms, count) in &self.submit_to_complete {
            println!("  {ms}: {count}");
        }
    }
}

fn main() -> io::Result<()> {
    let mut text = String::new();
    io::stdin().read_to_string(&mut text)?;
    match Report::of(&swaps_of(&text)) {
        Some(report) => {
            report.print();
            Ok(())
        }
        None => Err(io::Error::other(
            "no swaps to fit: was the screen changing during the recording?",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(period: f64) -> Vec<Swap> {
        (0..400)
            .filter(|slot| slot % 7 != 3)
            .map(|slot| {
                let completed = 1000.0 + f64::from(slot) * period + f64::from(slot % 5) * 0.01;
                let submitted = completed - if slot % 4 == 0 { 2.0 + period } else { 3.0 };
                Swap {
                    submitted,
                    completed,
                }
            })
            .collect()
    }

    #[test]
    fn finds_the_refresh_and_the_misses_at_any_rate() {
        for hertz in [60.0, 119.989, 144.001] {
            let period = 1e3 / hertz;
            let report = Report::of(&run(period));
            let Some(report) = report else {
                panic!("no fit at {hertz}");
            };
            assert!((report.grid.period - period).abs() < 0.001, "{hertz}");
            assert!(report.missed > 0, "{hertz}");
            assert!(
                report.hit_slack_p50 > 2.9 && report.hit_slack_p50 < 3.1,
                "{hertz}"
            );
        }
    }

    #[test]
    fn reads_a_perf_script_line() {
        let line = " kworker/u35:2-r  377712 41285.518207:                dcp:iomfb_swap_submit: dcp=ffff800012050080, swap_id=199724";
        assert_eq!(parse_line(line), Some((41285518.207, "submit", 199724)));
    }
}
