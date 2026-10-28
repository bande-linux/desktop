mod edit;
mod report;
mod service;
mod store;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

pub use edit::Setting;
pub use report::{EDIT_ACTION, ErrorNotifier};
pub use service::{BUS_NAME, ConfigService, OBJECT_PATH, ServiceError};
pub use store::{ConfigStore, ReloadError};

const SCALE_RANGE: (f64, f64) = (1.0, 2.0);
const REFRESH_RATE_RANGE: (f64, f64) = (60.0, 240.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
    Automatic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelEdge {
    Left,
    Bottom,
    Right,
    Top,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Idle {
    pub dim_after: Duration,
    pub lock_after: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Output {
    pub scale: Option<f64>,
    pub refresh_rate: Option<f64>,
    pub variable_refresh_rate: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub appearance: Appearance,
    pub panel_edge: PanelEdge,
    pub idle: Idle,
    pub outputs: BTreeMap<String, Output>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            appearance: Appearance::Automatic,
            panel_edge: PanelEdge::Left,
            idle: Idle {
                dim_after: Duration::from_secs(5 * 60),
                lock_after: Duration::from_secs(10 * 60),
            },
            outputs: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ConfigError {}

pub fn config_path(xdg_config_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let base = match xdg_config_home.filter(|path| path.is_absolute()) {
        Some(config_home) => config_home.to_path_buf(),
        None => home?.join(".config"),
    };
    Some(base.join("unbound").join("config.kdl"))
}

impl Config {
    pub fn parse(source: &str) -> Result<Self, ConfigError> {
        let document = KdlDocument::parse_v2(source).map_err(|error| {
            let diagnostic = error.diagnostics.first();
            let offset = diagnostic.map_or(0, |diagnostic| diagnostic.span.offset());
            let message = diagnostic
                .and_then(|diagnostic| diagnostic.message.clone().or(diagnostic.label.clone()))
                .unwrap_or_else(|| "invalid KDL".to_owned());
            locate(source, offset, message)
        })?;
        Parser { source }.config(&document)
    }
}

fn locate(source: &str, offset: usize, message: String) -> ConfigError {
    let before = source.get(..offset).unwrap_or(source);
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    ConfigError {
        line: before.matches('\n').count() + 1,
        column: before[line_start..].chars().count() + 1,
        message,
    }
}

struct Parser<'a> {
    source: &'a str,
}

impl Parser<'_> {
    fn at_node(&self, node: &KdlNode, message: impl Into<String>) -> ConfigError {
        locate(self.source, node.span().offset(), message.into())
    }

    fn at_entry(&self, entry: &KdlEntry, message: impl Into<String>) -> ConfigError {
        locate(self.source, entry.span().offset(), message.into())
    }

    fn config(&self, document: &KdlDocument) -> Result<Config, ConfigError> {
        let mut config = Config::default();
        let mut seen = Vec::new();
        for node in document.nodes() {
            let name = node.name().value();
            if name != "output" {
                if seen.contains(&name) {
                    return Err(self.at_node(node, format!("`{name}` is set more than once")));
                }
                seen.push(name);
            }
            match name {
                "appearance" => {
                    config.appearance = self.keyword(
                        node,
                        &[
                            ("dark", Appearance::Dark),
                            ("light", Appearance::Light),
                            ("automatic", Appearance::Automatic),
                        ],
                    )?
                }
                "panel" => config.panel_edge = self.panel(node)?,
                "idle" => config.idle = self.idle(node, config.idle)?,
                "output" => {
                    let (connector, output) = self.output(node)?;
                    if config.outputs.insert(connector.clone(), output).is_some() {
                        return Err(self.at_node(
                            node,
                            format!("output \"{connector}\" is configured more than once"),
                        ));
                    }
                }
                _ => return Err(self.at_node(node, format!("unknown setting `{name}`"))),
            }
        }
        Ok(config)
    }

    fn children<'n>(&self, node: &'n KdlNode) -> Result<&'n [KdlNode], ConfigError> {
        let name = node.name().value();
        let children = node.children().map_or(&[][..], KdlDocument::nodes);
        let mut seen = Vec::new();
        for child in children {
            let child_name = child.name().value();
            if seen.contains(&child_name) {
                return Err(self.at_node(
                    child,
                    format!("`{name} {child_name}` is set more than once"),
                ));
            }
            seen.push(child_name);
        }
        Ok(children)
    }

    fn no_arguments(&self, node: &KdlNode) -> Result<(), ConfigError> {
        match node.entries().first() {
            Some(entry) => Err(self.at_entry(
                entry,
                format!("`{}` takes no arguments", node.name().value()),
            )),
            None => Ok(()),
        }
    }

    fn argument<'n>(&self, node: &'n KdlNode) -> Result<&'n KdlEntry, ConfigError> {
        let name = node.name().value();
        let entries = node.entries();
        if let Some(property) = entries.iter().find(|entry| entry.name().is_some()) {
            return Err(self.at_entry(property, format!("`{name}` takes a value, not a property")));
        }
        match entries {
            [entry] => Ok(entry),
            [] => Err(self.at_node(node, format!("`{name}` needs a value"))),
            [_, extra, ..] => {
                Err(self.at_entry(extra, format!("`{name}` takes exactly one value")))
            }
        }
    }

    fn keyword<T: Copy>(&self, node: &KdlNode, choices: &[(&str, T)]) -> Result<T, ConfigError> {
        let entry = self.argument(node)?;
        let names: Vec<_> = choices
            .iter()
            .map(|(name, _)| format!("\"{name}\""))
            .collect();
        let invalid = || {
            self.at_entry(
                entry,
                format!(
                    "`{}` must be one of {}",
                    node.name().value(),
                    names.join(", ")
                ),
            )
        };
        let value = entry.value().as_string().ok_or_else(invalid)?;
        choices
            .iter()
            .find(|(name, _)| *name == value)
            .map(|(_, choice)| *choice)
            .ok_or_else(invalid)
    }

    fn unknown_child(&self, parent: &str, child: &KdlNode) -> ConfigError {
        self.at_node(
            child,
            format!("unknown setting `{parent} {}`", child.name().value()),
        )
    }

    fn panel(&self, node: &KdlNode) -> Result<PanelEdge, ConfigError> {
        self.no_arguments(node)?;
        let mut edge = Config::default().panel_edge;
        for child in self.children(node)? {
            match child.name().value() {
                "edge" => {
                    edge = self.keyword(
                        child,
                        &[
                            ("left", PanelEdge::Left),
                            ("bottom", PanelEdge::Bottom),
                            ("right", PanelEdge::Right),
                            ("top", PanelEdge::Top),
                        ],
                    )?
                }
                _ => return Err(self.unknown_child("panel", child)),
            }
        }
        Ok(edge)
    }

    fn idle(&self, node: &KdlNode, mut idle: Idle) -> Result<Idle, ConfigError> {
        self.no_arguments(node)?;
        let mut offending = node;
        for child in self.children(node)? {
            match child.name().value() {
                "dim-after" => idle.dim_after = self.duration(child)?,
                "lock-after" => {
                    idle.lock_after = self.duration(child)?;
                    offending = child;
                }
                _ => return Err(self.unknown_child("idle", child)),
            }
        }
        if idle.lock_after < idle.dim_after {
            return Err(self.at_node(
                offending,
                "`idle lock-after` must not be shorter than `idle dim-after`",
            ));
        }
        Ok(idle)
    }

    fn duration(&self, node: &KdlNode) -> Result<Duration, ConfigError> {
        let entry = self.argument(node)?;
        let invalid = || {
            self.at_entry(
                entry,
                format!(
                    "`{}` must be a positive duration such as \"30s\", \"5m\" or \"1h\"",
                    node.name().value()
                ),
            )
        };
        let text = entry.value().as_string().ok_or_else(invalid)?;
        let (amount, unit_seconds) = [("s", 1), ("m", 60), ("h", 3600)]
            .into_iter()
            .find_map(|(suffix, seconds)| Some((text.strip_suffix(suffix)?, seconds)))
            .ok_or_else(invalid)?;
        if amount.is_empty() || !amount.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid());
        }
        amount
            .parse::<u64>()
            .ok()
            .and_then(|amount| amount.checked_mul(unit_seconds))
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .ok_or_else(invalid)
    }

    fn output(&self, node: &KdlNode) -> Result<(String, Output), ConfigError> {
        let entry = self.argument(node)?;
        let connector = entry
            .value()
            .as_string()
            .filter(|connector| !connector.is_empty())
            .ok_or_else(|| {
                self.at_entry(entry, "`output` needs a connector name such as \"eDP-1\"")
            })?
            .to_owned();
        let mut output = Output::default();
        for child in self.children(node)? {
            match child.name().value() {
                "scale" => output.scale = Some(self.number_in(child, SCALE_RANGE)?),
                "refresh-rate" => {
                    output.refresh_rate = Some(self.number_in(child, REFRESH_RATE_RANGE)?)
                }
                "variable-refresh-rate" => {
                    let entry = self.argument(child)?;
                    let enabled = entry.value().as_bool().ok_or_else(|| {
                        self.at_entry(entry, "`variable-refresh-rate` must be #true or #false")
                    })?;
                    output.variable_refresh_rate = Some(enabled);
                }
                _ => return Err(self.unknown_child("output", child)),
            }
        }
        Ok((connector, output))
    }

    fn number_in(&self, node: &KdlNode, (min, max): (f64, f64)) -> Result<f64, ConfigError> {
        let entry = self.argument(node)?;
        let value = match entry.value() {
            KdlValue::Integer(value) => i32::try_from(*value).ok().map(f64::from),
            KdlValue::Float(value) => Some(*value),
            _ => None,
        };
        value
            .filter(|value| (min..=max).contains(value))
            .ok_or_else(|| {
                self.at_entry(
                    entry,
                    format!(
                        "`{}` must be a number from {min} to {max}",
                        node.name().value()
                    ),
                )
            })
    }
}
