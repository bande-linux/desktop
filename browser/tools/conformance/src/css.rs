use std::path::Path;

use serde_json::Value;

use crate::engine;
use crate::report::Tally;

pub fn run(css_parsing: &Path) -> Result<Vec<Tally>, String> {
    let mut tallies =
        ["syntax", "colors", "An+B"].map(|group| Tally::new(format!("css-parsing {group}")));
    for path in crate::files(css_parsing, "json")? {
        let entry = crate::file_name(&path).trim_end_matches(".json").to_owned();
        let tally = match entry.as_str() {
            "An+B" => &mut tallies[2],
            color if color.starts_with("color_") => &mut tallies[1],
            _ => &mut tallies[0],
        };
        let file = crate::read_json(&path)?;
        let pairs = file.as_array().map(Vec::as_slice).unwrap_or_default();
        for [input, expected] in pairs.as_chunks().0 {
            let passed = crate::guarded(|| {
                engine::css(&entry, input).is_some_and(|actual| same(&actual, expected))
            });
            tally.record(passed, || format!("{entry} {input}"));
        }
    }
    Ok(tallies.into())
}

fn same(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0),
            _ => a == b,
        },
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, value)| b.get(key).is_some_and(|other| same(value, other)))
        }
        _ => actual == expected,
    }
}
