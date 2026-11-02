use std::path::Path;

use serde_json::Value;

use crate::engine;
use crate::report::Tally;

pub fn run(html5lib: &Path) -> Result<Tally, String> {
    let mut tally = Tally::new("html5lib tokenizer");
    for path in crate::files(&html5lib.join("tokenizer"), "test")? {
        let file = crate::read_json(&path)?;
        let name = crate::file_name(&path);
        for test in file["tests"].as_array().into_iter().flatten() {
            let Some(case) = Case::from_json(test) else {
                tally.skipped += 1;
                continue;
            };
            let states = test["initialStates"]
                .as_array()
                .map(|states| states.iter().filter_map(Value::as_str).collect())
                .unwrap_or_else(|| vec!["Data state"]);
            for state in states {
                let passed = crate::guarded(|| {
                    coalesce(engine::tokenize(&case.input, state, case.last_start_tag))
                        == case.output
                });
                tally.record(passed, || {
                    format!("{name}: {} ({state})", test["description"])
                });
            }
        }
    }
    Ok(tally)
}

struct Case<'a> {
    input: String,
    output: Vec<Value>,
    last_start_tag: Option<&'a str>,
}

impl<'a> Case<'a> {
    fn from_json(test: &'a Value) -> Option<Self> {
        let double = test["doubleEscaped"].as_bool() == Some(true);
        let text = |value: &str| {
            if double {
                unescape(value)
            } else {
                Some(value.to_owned())
            }
        };
        Some(Self {
            input: text(test["input"].as_str()?)?,
            output: coalesce(
                test["output"]
                    .as_array()?
                    .iter()
                    .map(|token| map_strings(token, &text))
                    .collect::<Option<_>>()?,
            ),
            last_start_tag: test["lastStartTag"].as_str(),
        })
    }
}

fn map_strings(value: &Value, text: &impl Fn(&str) -> Option<String>) -> Option<Value> {
    Some(match value {
        Value::String(string) => Value::String(text(string)?),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| map_strings(item, text))
                .collect::<Option<_>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| Some((text(key)?, map_strings(item, text)?)))
                .collect::<Option<_>>()?,
        ),
        other => other.clone(),
    })
}

fn unescape(text: &str) -> Option<String> {
    let mut units = Vec::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find("\\u") {
        units.extend(rest[..index].encode_utf16());
        let hex = rest.get(index + 2..index + 6)?;
        units.push(u16::from_str_radix(hex, 16).ok()?);
        rest = &rest[index + 6..];
    }
    units.extend(rest.encode_utf16());
    String::from_utf16(&units).ok()
}

fn coalesce(tokens: Vec<Value>) -> Vec<Value> {
    let mut merged: Vec<Value> = Vec::with_capacity(tokens.len());
    for token in tokens {
        if let (Some(Value::Array(last)), Value::Array(next)) = (merged.last_mut(), &token)
            && last.first().and_then(Value::as_str) == Some("Character")
            && next.first().and_then(Value::as_str) == Some("Character")
            && let (Some(Value::String(text)), Some(more)) =
                (last.get_mut(1), next.get(1).and_then(Value::as_str))
        {
            text.push_str(more);
            continue;
        }
        merged.push(token);
    }
    merged
}
