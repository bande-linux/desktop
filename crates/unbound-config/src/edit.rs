use std::time::Duration;

use kdl::{KdlDocument, KdlEntry, KdlEntryFormat, KdlNode, KdlValue};

use crate::{Appearance, Config, ConfigError, Output, PanelEdge};

#[derive(Debug, Clone, PartialEq)]
pub enum Setting {
    Appearance(Appearance),
    PanelEdge(PanelEdge),
    IdleDimAfter(Duration),
    IdleLockAfter(Duration),
    Output(String, Output),
}

impl Setting {
    pub fn apply(&self, source: &str) -> Result<String, ConfigError> {
        Config::parse(source)?;
        let mut document = KdlDocument::parse_v2(source).map_err(|_| ConfigError {
            line: 1,
            column: 1,
            message: "invalid KDL".to_owned(),
        })?;
        match self {
            Self::Appearance(appearance) => set(
                &mut document,
                None,
                "appearance",
                Some(appearance.keyword().into()),
            ),
            Self::PanelEdge(edge) => set(
                &mut document,
                Some("panel"),
                "edge",
                Some(edge.keyword().into()),
            ),
            Self::IdleDimAfter(after) => set(
                &mut document,
                Some("idle"),
                "dim-after",
                Some(duration(*after).into()),
            ),
            Self::IdleLockAfter(after) => set(
                &mut document,
                Some("idle"),
                "lock-after",
                Some(duration(*after).into()),
            ),
            Self::Output(connector, output) => set_output(&mut document, connector, output),
        }
        let edited = document.to_string();
        Config::parse(&edited)?;
        Ok(edited)
    }
}

impl Appearance {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Automatic => "automatic",
        }
    }
}

impl PanelEdge {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Bottom => "bottom",
            Self::Right => "right",
            Self::Top => "top",
        }
    }
}

fn duration(after: Duration) -> String {
    let seconds = after.as_secs();
    match seconds {
        _ if seconds.is_multiple_of(3600) => format!("{}h", seconds / 3600),
        _ if seconds.is_multiple_of(60) => format!("{}m", seconds / 60),
        _ => format!("{seconds}s"),
    }
}

fn number(value: f64) -> KdlValue {
    if value.fract() == 0.0 && value.abs() < f64::from(i32::MAX) {
        KdlValue::Integer(value as i128)
    } else {
        KdlValue::Float(value)
    }
}

fn set_output(document: &mut KdlDocument, connector: &str, output: &Output) {
    let settings = [
        ("scale", output.scale.map(number)),
        ("refresh-rate", output.refresh_rate.map(number)),
        (
            "variable-refresh-rate",
            output.variable_refresh_rate.map(KdlValue::Bool),
        ),
    ];
    let position = document.nodes().iter().position(|node| {
        node.name().value() == "output"
            && node
                .entries()
                .first()
                .and_then(|entry| entry.value().as_string())
                == Some(connector)
    });
    match position {
        Some(position) if *output == Output::default() => {
            remove(document, position);
        }
        Some(position) => {
            let children = document.nodes_mut()[position].ensure_children();
            for (name, value) in settings {
                set_child(children, name, value, CHILD_INDENT);
            }
        }
        None if *output == Output::default() => {}
        None => {
            let body: String = settings
                .into_iter()
                .filter_map(|(name, value)| {
                    Some(format!("{CHILD_INDENT}{name} {}\n", repr(&value?)))
                })
                .collect();
            append(
                document,
                &format!("output {} {{\n{body}}}\n", repr(&connector.into())),
                "",
            );
        }
    }
}

const CHILD_INDENT: &str = "    ";

fn set(document: &mut KdlDocument, section: Option<&str>, name: &str, value: Option<KdlValue>) {
    let Some(section) = section else {
        set_child(document, name, value, "");
        return;
    };
    match document
        .nodes_mut()
        .iter_mut()
        .find(|node| node.name().value() == section)
    {
        Some(node) => set_child(node.ensure_children(), name, value, CHILD_INDENT),
        None => {
            if let Some(value) = value {
                let value = repr(&value);
                append(
                    document,
                    &format!("{section} {{\n{CHILD_INDENT}{name} {value}\n}}\n"),
                    "",
                );
            }
        }
    }
}

fn set_child(document: &mut KdlDocument, name: &str, value: Option<KdlValue>, indent: &str) {
    let position = document
        .nodes()
        .iter()
        .position(|node| node.name().value() == name);
    match (position, value) {
        (Some(position), None) => {
            remove(document, position);
        }
        (None, None) => {}
        (Some(position), Some(value)) => {
            let entries = document.nodes_mut()[position].entries_mut();
            let mut format = entries
                .first()
                .and_then(KdlEntry::format)
                .cloned()
                .unwrap_or_else(|| KdlEntryFormat {
                    leading: " ".to_owned(),
                    ..KdlEntryFormat::default()
                });
            format.value_repr = repr(&value);
            let mut entry = KdlEntry::new(value);
            entry.set_format(format);
            *entries = vec![entry];
        }
        (None, Some(value)) => {
            append(document, &format!("{name} {}\n", repr(&value)), indent);
        }
    }
}

fn remove(document: &mut KdlDocument, position: usize) {
    let removed = document.nodes_mut().remove(position);
    if let (Some(leading), Some(format)) = (
        removed.format().map(|format| format.leading.clone()),
        document
            .nodes_mut()
            .get_mut(position)
            .and_then(KdlNode::format_mut),
    ) {
        format.leading = leading;
    }
}

fn repr(value: &KdlValue) -> String {
    match value {
        KdlValue::String(text)
            if text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c)) =>
        {
            format!("\"{text}\"")
        }
        _ => KdlEntry::new(value.clone()).to_string(),
    }
}

fn append(document: &mut KdlDocument, text: &str, indent: &str) {
    let indent = document
        .nodes()
        .first()
        .and_then(KdlNode::format)
        .map_or(indent, |format| {
            let leading = format.leading.as_str();
            &leading[leading.rfind('\n').map_or(0, |newline| newline + 1)..]
        })
        .to_owned();
    let indented: String = text
        .lines()
        .map(|line| format!("{indent}{line}\n"))
        .collect();
    if let Some(format) = document
        .nodes_mut()
        .last_mut()
        .and_then(KdlNode::format_mut)
        && format.terminator.is_empty()
    {
        format.terminator = "\n".to_owned();
    }
    if let Ok(parsed) = KdlDocument::parse_v2(&indented) {
        document.nodes_mut().extend(parsed);
    }
}
