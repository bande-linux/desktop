use std::collections::BTreeMap;

use bande_intents::{Definition, Dict, Kind, Parameter, Refusal, to_json};
use serde_json::Value as Json;

const TEXT: &str = "text";
const ON: &str = "on";

/// The arguments of one command line.
pub struct Given {
    pub arguments: Dict,
}

impl Given {
    pub fn take(&mut self, name: &str) -> Vec<String> {
        match self.arguments.remove(name).map(|value| to_json(&value)) {
            Some(Json::Array(items)) => items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
            Some(Json::String(one)) => vec![one],
            _ => Vec::new(),
        }
    }

    pub fn take_references(&mut self, definition: &Definition) -> Vec<String> {
        match definition.parameters.first() {
            Some(first) if first.kind.names_things() => self.take(&first.name),
            _ => Vec::new(),
        }
    }
}

fn unknown(definition: &Definition, word: &str) -> Refusal {
    Refusal::invalid(
        word,
        definition
            .parameters
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect(),
    )
}

/// Reads `words`: the required parameters in the order they are declared, the last of them taking
/// the rest if it is a list, and the others as `--name value`. Words that are left are the `text`.
pub fn given(definition: &Definition, words: &[String]) -> Result<Given, Refusal> {
    let mut gathered: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut bare = Vec::new();
    let mut words = words.iter().peekable();
    while let Some(word) = words.next() {
        if word == "--" {
            bare.extend(words.by_ref().cloned());
            break;
        }
        let Some(option) = word.strip_prefix("--") else {
            bare.push(word.clone());
            continue;
        };
        let (name, value) = match option.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (option, None),
        };
        let parameter = definition
            .parameter(&name.replace('-', "_"))
            .ok_or_else(|| unknown(definition, name))?;
        let value = value
            .or_else(|| match &parameter.kind {
                Kind::Bool => {
                    let said = words
                        .next_if(|word| parameter.kind.from_words(&[(*word).clone()]).is_some());
                    Some(said.map_or(ON, String::as_str).to_owned())
                }
                _ => words.next().cloned(),
            })
            .ok_or_else(|| Refusal::invalid(&parameter.name, parameter.kind.accepts()))?;
        gathered.entry(&parameter.name).or_default().push(value);
    }
    let mut bare = bare.into_iter().peekable();
    let required: Vec<&Parameter> = definition
        .parameters
        .iter()
        .filter(|parameter| parameter.required)
        .collect();
    for parameter in required {
        if gathered.contains_key(parameter.name.as_str()) {
            continue;
        }
        let values: Vec<String> = match parameter.kind {
            Kind::List { .. } => bare.by_ref().collect(),
            _ => bare.next().into_iter().collect(),
        };
        if values.is_empty() {
            return Err(Refusal::invalid(&parameter.name, parameter.kind.accepts()));
        }
        gathered.insert(&parameter.name, values);
    }
    let rest: Vec<String> = bare.collect();
    if let Some(word) = rest.first() {
        if definition.parameter(TEXT).is_none() || gathered.contains_key(TEXT) {
            return Err(unknown(definition, word));
        }
        gathered.insert(TEXT, vec![rest.join(" ")]);
    }
    let arguments = gathered
        .into_iter()
        .filter_map(|(name, values)| Some((definition.parameter(name)?, values)))
        .map(|(parameter, values)| {
            let value = parameter
                .kind
                .from_words(&values)
                .ok_or_else(|| Refusal::invalid(&parameter.name, parameter.kind.accepts()))?;
            Ok((parameter.name.clone(), value))
        })
        .collect::<Result<Dict, Refusal>>()?;
    Ok(Given { arguments })
}
