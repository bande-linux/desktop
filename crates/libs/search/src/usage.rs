use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const DAY: u64 = 24 * 60 * 60;
const KEPT: usize = 1000;
pub const MAX_BOOST: u32 = 150;
const BOOST_PER_POINT: f64 = 60.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Record {
    count: u32,
    last: u64,
}

impl Record {
    fn weight(self, now: u64) -> f64 {
        let age = now.saturating_sub(self.last) / DAY;
        let recency = match age {
            0..4 => 1.0,
            4..14 => 0.7,
            14..31 => 0.5,
            31..90 => 0.3,
            _ => 0.1,
        };
        f64::from(self.count).ln_1p() * recency
    }
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    records: HashMap<String, Record>,
    file: Option<PathBuf>,
}

pub fn usage_path(state_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let state = state_home
        .filter(|path| path.is_absolute())
        .map(Path::to_path_buf)
        .or_else(|| home.map(|home| home.join(".local/state")))?;
    Some(state.join("unbound/launcher-usage"))
}

impl Usage {
    pub fn load(file: PathBuf) -> Self {
        let records = fs::read_to_string(&file)
            .map(|text| parse(&text))
            .unwrap_or_default();
        Self {
            records,
            file: Some(file),
        }
    }

    pub fn boost(&self, key: &str, now: u64) -> u32 {
        self.records.get(key).map_or(0, |record| {
            let boost = (record.weight(now) * BOOST_PER_POINT).round();
            (boost as u32).min(MAX_BOOST)
        })
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.records.keys().map(String::as_str)
    }

    pub fn record(&mut self, key: &str, now: u64) -> io::Result<()> {
        if key.contains('\n') {
            return Ok(());
        }
        let record = self.records.entry(key.to_owned()).or_insert(Record {
            count: 0,
            last: now,
        });
        record.count = record.count.saturating_add(1);
        record.last = now;
        if self.records.len() > KEPT {
            let mut weights: Vec<(f64, String)> = self
                .records
                .iter()
                .map(|(key, record)| (record.weight(now), key.clone()))
                .collect();
            weights.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (_, key) in weights.into_iter().skip(KEPT) {
                self.records.remove(&key);
            }
        }
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut lines: Vec<String> = self
            .records
            .iter()
            .map(|(key, record)| format!("{}\t{}\t{key}\n", record.count, record.last))
            .collect();
        lines.sort();
        let partial = file.with_extension("partial");
        fs::write(&partial, lines.concat())?;
        fs::rename(partial, file)
    }
}

fn parse(text: &str) -> HashMap<String, Record> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let count = fields.next()?.parse().ok()?;
            let last = fields.next()?.parse().ok()?;
            let key = fields.next().filter(|key| !key.is_empty())?;
            Some((key.to_owned(), Record { count, last }))
        })
        .collect()
}
