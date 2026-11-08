use std::fs;
use std::path::Path;

use serde_json::{Map, Value, json};

const FILE: &str = "counters.json";
const STAGED: &str = ".counters.json.new";
const MOST_NAMES: usize = 64;

pub fn is_counter_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
}

pub fn count(spool_root: &Path, name: &str) {
    if !is_counter_name(name) {
        return;
    }
    let path = spool_root.join(FILE);
    let mut counters: Map<String, Value> = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if counters.len() >= MOST_NAMES && !counters.contains_key(name) {
        return;
    }
    let now = counters.get(name).and_then(Value::as_u64).unwrap_or(0) + 1;
    counters.insert(name.to_owned(), json!(now));
    let staged = spool_root.join(STAGED);
    let written = serde_json::to_vec(&counters)
        .map_err(std::io::Error::from)
        .and_then(|bytes| fs::write(&staged, bytes))
        .and_then(|()| fs::rename(&staged, &path));
    if let Err(error) = written {
        tracing::debug!("cannot count {name}: {error}");
    }
}
