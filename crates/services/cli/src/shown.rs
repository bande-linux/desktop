use bande_intents::{
    Definition, Dict, Kind, Manifest, Manifests, Performed, Refusal, domain as domain_of, to_json,
};
use bande_l10n::tr;
use serde_json::Value as Json;

const FRAME: [&str; 4] = ["reference", "type", "foreign", "cut"];

fn command(id: &str) -> String {
    let verb = id
        .split_once('.')
        .map_or("", |(_, verb)| verb)
        .replace('_', "-");
    format!("bande {} {verb}", domain_of(id))
}

fn lines(manifests: &Manifests, of: Option<&str>) -> String {
    let mut rows: Vec<(String, String)> = Vec::new();
    for manifest in manifests.iter() {
        let catalogs = manifest.catalogs();
        for (id, definition) in &manifest.intents {
            if of.is_none_or(|of| of == domain_of(id)) {
                let title = &definition.title;
                rows.push((
                    command(id),
                    catalogs.pattern(title.context.as_deref(), &title.en),
                ));
            }
        }
    }
    rows.sort();
    rows.push((
        "bande undo [receipt]".to_owned(),
        tr!("Undo the last change"),
    ));
    let width = rows
        .iter()
        .map(|(command, _)| command.chars().count())
        .max()
        .unwrap_or(0);
    rows.into_iter()
        .map(|(command, title)| format!("{command:width$}  {title}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn everything(manifests: &Manifests) -> String {
    lines(manifests, None)
}

pub fn domain(manifests: &Manifests, of: &str) -> String {
    lines(manifests, Some(of))
}

pub fn help(manifest: &Manifest, id: &str, definition: &Definition) -> String {
    let mut usage = command(id);
    let mut rows = Vec::new();
    for parameter in &definition.parameters {
        let name = match parameter.required {
            true => format!("<{}>", parameter.name.replace('_', "-")),
            false => format!("--{}", parameter.name.replace('_', "-")),
        };
        usage.push(' ');
        usage.push_str(&match (parameter.required, &parameter.kind) {
            (true, Kind::List { .. }) => format!("{name}…"),
            (true, _) => name.clone(),
            (false, _) => format!("[{name} …]"),
        });
        rows.push(format!("  {name}: {}", parameter.kind.accepts().join(", ")));
    }
    let mut text = format!("{usage}\n{}", manifest.label(&definition.title));
    for row in rows {
        text.push('\n');
        text.push_str(&row);
    }
    text
}

fn written(value: &Json) -> String {
    match value {
        Json::String(text) => text.clone(),
        Json::Array(items) => items.iter().map(written).collect::<Vec<_>>().join(", "),
        Json::Object(entries) => entries
            .iter()
            .map(|(name, value)| format!("{name} {}", written(value)))
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    }
}

fn own_text(dict: &Dict, name: &str) -> String {
    dict.get(name)
        .map(|value| written(&to_json(value)))
        .unwrap_or_default()
}

fn printable(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\n' | '\t' => character,
            _ if character.is_control() => '\u{fffd}',
            _ => character,
        })
        .collect()
}

pub fn things(things: &[Dict], whole: bool) -> String {
    let shown: Vec<String> = things
        .iter()
        .map(|thing| {
            let reference = own_text(thing, "reference");
            let token = reference
                .split_once(':')
                .map_or(reference.as_str(), |(_, token)| token);
            let mut names: Vec<&String> = thing
                .keys()
                .filter(|name| !FRAME.contains(&name.as_str()) && *name != "title")
                .collect();
            names.sort();
            let title = own_text(thing, "title");
            match whole {
                true => names
                    .into_iter()
                    .fold(format!("{title} ({token})"), |mut text, name| {
                        text.push_str(&format!("\n  {name}: {}", own_text(thing, name)));
                        text
                    }),
                false => names
                    .into_iter()
                    .fold(format!("{token}\t{title}"), |mut text, name| {
                        text.push('\t');
                        text.push_str(&own_text(thing, name));
                        text
                    }),
            }
        })
        .collect();
    printable(&shown.join("\n"))
}

pub fn performed(summary: &str, performed: &Performed) -> String {
    let mut text = match performed.cancelled {
        true => tr!("Stopped: {what}", what = summary),
        false => summary.to_owned(),
    };
    if let Some(value) = performed.result.get("value") {
        text = written(&to_json(value));
    }
    if let Some(Json::Array(failed)) = performed.result.get("failed").map(|value| to_json(value)) {
        for failure in failed {
            text.push('\n');
            text.push_str(&format!(
                "{}: {}",
                written(&failure["reference"]),
                written(&failure["message"])
            ));
        }
    }
    printable(&text)
}

pub fn error(refusal: &Refusal) -> String {
    let mut text = refusal.message.clone();
    for candidate in &refusal.candidates {
        text.push_str(&format!("\n  {candidate}"));
    }
    printable(&text)
}
