mod accounts;
mod connectors;
mod edit;
mod report;
mod service;
mod store;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};
use unbound_l10n::Localizer;

pub use accounts::{login_picture, share_login_picture};
pub use connectors::{DRM_CLASS, connected_displays};
pub use edit::{Setting, migrate};
pub use report::{EDIT_ACTION, ErrorNotifier};
pub use service::{
    ArrangedDisplay, BUS_NAME, ConfigService, OBJECT_PATH, ServiceError, migrate_file,
};
pub use store::{ConfigStore, Loaded, ReloadError};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-config.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

const SCALE_RANGE: (f64, f64) = (1.0, 3.0);
const REFRESH_RATE_RANGE: (f64, f64) = (60.0, 240.0);
pub const KEY_REPEAT_RATE_RANGE: (u32, u32) = (1, 60);
pub const KEY_REPEAT_DELAY_RANGE: (u32, u32) = (150, 2000);
pub const DOCK_ICON_SIZE_RANGE: (u32, u32) = (24, 96);
pub const DEFAULT_DOCK_ICON_SIZE: u32 = 48;
pub const MAX_INPUT_SOURCES: usize = 4;
pub const POINTER_SPEED_RANGE: (f64, f64) = (-1.0, 1.0);
pub const ALERT_VOLUME_RANGE: (f64, f64) = (0.0, 1.0);
pub const SYSTEM_KEYBOARD: &str = "/etc/default/keyboard";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
    Automatic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockEdge {
    Left,
    Bottom,
    Right,
    Top,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuBarPlacement {
    #[default]
    Separate,
    Combined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bar {
    pub menu_bar: MenuBarPlacement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suspend {
    WithDisplayOff,
    After(Duration),
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Idle {
    pub dim_after: Duration,
    pub lock_after: Duration,
    pub suspend: Suspend,
}

impl Idle {
    pub fn suspend_after(&self) -> Option<Duration> {
        match self.suspend {
            Suspend::WithDisplayOff => Some(self.lock_after),
            Suspend::After(after) => Some(after),
            Suspend::Never => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    Upright,
    Quarter,
    Half,
    ThreeQuarters,
}

impl Rotation {
    pub const ALL: [Self; 4] = [
        Self::Upright,
        Self::Quarter,
        Self::Half,
        Self::ThreeQuarters,
    ];

    pub fn degrees(self) -> u32 {
        match self {
            Self::Upright => 0,
            Self::Quarter => 90,
            Self::Half => 180,
            Self::ThreeQuarters => 270,
        }
    }

    pub fn from_degrees(degrees: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|rotation| rotation.degrees() == degrees)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DisplaySettings {
    pub resolution: Option<(u32, u32)>,
    pub refresh_rate: Option<f64>,
    pub scale: Option<f64>,
    pub rotation: Option<Rotation>,
    pub variable_refresh_rate: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub display: String,
    pub x: i32,
    pub y: i32,
    pub main: bool,
    pub mirror: Option<String>,
}

pub fn same_displays(placements: &[Placement], displays: &[String]) -> bool {
    placements.len() == displays.len()
        && displays.iter().all(|display| {
            placements
                .iter()
                .any(|placement| &placement.display == display)
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InputSource {
    pub layout: String,
    pub variant: Option<String>,
}

fn is_xkb_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
}

impl InputSource {
    pub fn parse(code: &str) -> Option<Self> {
        let (layout, variant) = match code.split_once('(') {
            Some((layout, rest)) => (layout, Some(rest.strip_suffix(')')?)),
            None => (code, None),
        };
        (is_xkb_name(layout) && variant.is_none_or(is_xkb_name)).then(|| Self {
            layout: layout.to_owned(),
            variant: variant.map(str::to_owned),
        })
    }

    pub fn code(&self) -> String {
        match &self.variant {
            Some(variant) => format!("{}({variant})", self.layout),
            None => self.layout.clone(),
        }
    }
}

pub fn system_input_sources(default_keyboard: &str) -> Vec<InputSource> {
    let field = |name: &str| {
        default_keyboard.lines().find_map(|line| {
            let value = line.trim().strip_prefix(name)?.strip_prefix('=')?;
            Some(value.trim().trim_matches('"').to_owned())
        })
    };
    let layouts = field("XKBLAYOUT").unwrap_or_default();
    let variants = field("XKBVARIANT").unwrap_or_default();
    let mut variants = variants.split(',');
    let mut sources: Vec<InputSource> = Vec::new();
    for layout in layouts.split(',') {
        let variant = variants
            .next()
            .map(str::trim)
            .filter(|variant| !variant.is_empty());
        let source = InputSource {
            layout: layout.trim().to_owned(),
            variant: variant.map(str::to_owned),
        };
        if is_xkb_name(&source.layout)
            && source.variant.as_deref().is_none_or(is_xkb_name)
            && !sources.contains(&source)
            && sources.len() < MAX_INPUT_SOURCES
        {
            sources.push(source);
        }
    }
    if sources.is_empty() {
        sources.push(InputSource {
            layout: "us".to_owned(),
            variant: None,
        });
    }
    sources
}

pub fn read_system_input_sources() -> Vec<InputSource> {
    system_input_sources(&std::fs::read_to_string(SYSTEM_KEYBOARD).unwrap_or_default())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CapsLock {
    #[default]
    CapsLock,
    Escape,
    Control,
    Compose,
    Off,
}

impl CapsLock {
    pub const ALL: [Self; 5] = [
        Self::CapsLock,
        Self::Escape,
        Self::Control,
        Self::Compose,
        Self::Off,
    ];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::CapsLock => "caps-lock",
            Self::Escape => "escape",
            Self::Control => "control",
            Self::Compose => "compose",
            Self::Off => "off",
        }
    }

    pub fn xkb_option(self) -> Option<&'static str> {
        match self {
            Self::CapsLock => None,
            Self::Escape => Some("caps:escape"),
            Self::Control => Some("ctrl:nocaps"),
            Self::Compose => Some("compose:caps"),
            Self::Off => Some("caps:none"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keyboard {
    pub repeat_rate: u32,
    pub repeat_delay: Duration,
    pub input_sources: Vec<InputSource>,
    pub caps_lock: CapsLock,
}

impl Keyboard {
    pub fn input_sources_or(&self, system: impl FnOnce() -> Vec<InputSource>) -> Vec<InputSource> {
        if self.input_sources.is_empty() {
            system()
        } else {
            self.input_sources.clone()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecondaryClick {
    #[default]
    TwoFingers,
    Corner,
}

impl SecondaryClick {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::TwoFingers => "two-fingers",
            Self::Corner => "corner",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trackpad {
    pub tap_to_click: bool,
    pub natural_scrolling: bool,
    pub speed: f64,
    pub secondary_click: SecondaryClick,
    pub disable_while_typing: bool,
}

impl Default for Trackpad {
    fn default() -> Self {
        Self {
            tap_to_click: true,
            natural_scrolling: true,
            speed: 0.0,
            secondary_click: SecondaryClick::TwoFingers,
            disable_while_typing: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Mouse {
    pub natural_scrolling: bool,
    pub speed: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlertSound {
    #[default]
    Mallet,
    Knock,
    Low,
    Tick,
}

impl AlertSound {
    pub const ALL: [Self; 4] = [Self::Mallet, Self::Knock, Self::Low, Self::Tick];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::Mallet => "mallet",
            Self::Knock => "knock",
            Self::Low => "low",
            Self::Tick => "tick",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sound {
    pub effects: bool,
    pub volume_feedback: bool,
    pub alert: AlertSound,
    pub alert_volume: f64,
}

impl Default for Sound {
    fn default() -> Self {
        Self {
            effects: true,
            volume_feedback: true,
            alert: AlertSound::Mallet,
            alert_volume: 0.75,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallpaperVariant {
    Light,
    Dark,
}

impl WallpaperVariant {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WallpaperFit {
    #[default]
    Fill,
    Fit,
}

impl WallpaperFit {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Fit => "fit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Wallpaper {
    pub light: Option<PathBuf>,
    pub dark: Option<PathBuf>,
    pub fit: WallpaperFit,
}

impl Wallpaper {
    pub fn path(&self, variant: WallpaperVariant) -> Option<&Path> {
        match variant {
            WallpaperVariant::Light => self.light.as_deref(),
            WallpaperVariant::Dark => self.dark.as_deref(),
        }
    }
}

pub const DEFAULT_DOCK_APPS: [&str; 3] = [
    "org.unbound.Files",
    "org.unbound.Terminal",
    "org.unbound.Settings",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dock {
    pub edge: DockEdge,
    pub apps: Vec<String>,
    pub auto_hide: bool,
    pub icon_size: u32,
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            edge: DockEdge::Left,
            apps: DEFAULT_DOCK_APPS.map(str::to_owned).to_vec(),
            auto_hide: false,
            icon_size: DEFAULT_DOCK_ICON_SIZE,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub appearance: Appearance,
    pub reduce_motion: bool,
    pub dock: Dock,
    pub bar: Bar,
    pub idle: Idle,
    pub keyboard: Keyboard,
    pub trackpad: Trackpad,
    pub mouse: Mouse,
    pub sound: Sound,
    pub wallpaper: Wallpaper,
    pub displays: BTreeMap<String, DisplaySettings>,
    pub outputs: BTreeMap<String, DisplaySettings>,
    pub arrangements: Vec<Vec<Placement>>,
}

impl Config {
    pub fn arrangement(&self, displays: &[String]) -> Option<&[Placement]> {
        self.arrangements
            .iter()
            .find(|placements| same_displays(placements, displays))
            .map(Vec::as_slice)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            appearance: Appearance::Automatic,
            reduce_motion: false,
            dock: Dock::default(),
            bar: Bar::default(),
            idle: Idle {
                dim_after: Duration::from_secs(5 * 60),
                lock_after: Duration::from_secs(10 * 60),
                suspend: Suspend::WithDisplayOff,
            },
            keyboard: Keyboard {
                repeat_rate: 25,
                repeat_delay: Duration::from_millis(600),
                input_sources: Vec::new(),
                caps_lock: CapsLock::CapsLock,
            },
            trackpad: Trackpad::default(),
            mouse: Mouse::default(),
            sound: Sound::default(),
            wallpaper: Wallpaper::default(),
            displays: BTreeMap::new(),
            outputs: BTreeMap::new(),
            arrangements: Vec::new(),
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

pub const RENAMED_SECTIONS: [(&str, &str); 1] = [("panel", "dock")];

pub const LEGACY_DISPLAY: &str = "output";

impl Config {
    pub fn parse(source: &str) -> Result<Self, ConfigError> {
        Self::load(source).map(|(config, _)| config)
    }

    pub fn load(source: &str) -> Result<(Self, Vec<ConfigError>), ConfigError> {
        let document = KdlDocument::parse_v2(source).map_err(|error| {
            let diagnostic = error.diagnostics.first();
            let offset = diagnostic.map_or(0, |diagnostic| diagnostic.span.offset());
            let message = diagnostic
                .and_then(|diagnostic| diagnostic.message.clone().or(diagnostic.label.clone()))
                .unwrap_or_else(|| "invalid KDL".to_owned());
            locate(source, offset, message)
        })?;
        let parser = Parser {
            source,
            problems: RefCell::new(Vec::new()),
        };
        let config = parser.config(&document);
        let mut problems = parser.problems.into_inner();
        problems.sort_by_key(|problem| (problem.line, problem.column));
        Ok((config, problems))
    }

    pub fn display(&self, identity: &str, connector: &str) -> DisplaySettings {
        self.displays
            .get(identity)
            .or_else(|| self.outputs.get(connector))
            .copied()
            .unwrap_or_default()
    }
}

pub fn section_name(written: &str) -> &str {
    RENAMED_SECTIONS
        .iter()
        .find(|(old, _)| *old == written)
        .map_or(written, |(_, new)| new)
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

pub fn is_wallpaper_path(path: &str) -> bool {
    Path::new(path).is_absolute() || path.strip_prefix("~/").is_some_and(|rest| !rest.is_empty())
}

struct Parser<'a> {
    source: &'a str,
    problems: RefCell<Vec<ConfigError>>,
}

impl Parser<'_> {
    fn at_node(&self, node: &KdlNode, message: impl Into<String>) -> ConfigError {
        locate(self.source, node.span().offset(), message.into())
    }

    fn at_entry(&self, entry: &KdlEntry, message: impl Into<String>) -> ConfigError {
        locate(self.source, entry.span().offset(), message.into())
    }

    fn settle(&self, result: Result<(), ConfigError>) {
        if let Err(problem) = result {
            self.problems.borrow_mut().push(problem);
        }
    }

    fn config(&self, document: &KdlDocument) -> Config {
        let mut config = Config::default();
        let mut seen = Vec::new();
        for node in document.nodes() {
            let written = node.name().value();
            let name = section_name(written);
            if !matches!(name, "display" | "arrangement" | LEGACY_DISPLAY) {
                if seen.contains(&name) {
                    self.settle(Err(
                        self.at_node(node, format!("`{written}` is set more than once"))
                    ));
                    continue;
                }
                seen.push(name);
            }
            let result = match name {
                "appearance" => self
                    .keyword(
                        node,
                        &[
                            ("dark", Appearance::Dark),
                            ("light", Appearance::Light),
                            ("automatic", Appearance::Automatic),
                        ],
                    )
                    .map(|appearance| config.appearance = appearance),
                "accessibility" => {
                    config.reduce_motion = self.accessibility(node);
                    Ok(())
                }
                "dock" => {
                    config.dock = self.dock(node);
                    Ok(())
                }
                "bar" => {
                    config.bar = self.bar(node);
                    Ok(())
                }
                "idle" => {
                    config.idle = self.idle(node, config.idle);
                    Ok(())
                }
                "keyboard" => {
                    self.keyboard(node, &mut config.keyboard);
                    Ok(())
                }
                "trackpad" => {
                    config.trackpad = self.trackpad(node);
                    Ok(())
                }
                "mouse" => {
                    config.mouse = self.mouse(node);
                    Ok(())
                }
                "sound" => {
                    config.sound = self.sound(node);
                    Ok(())
                }
                "wallpaper" => {
                    config.wallpaper = self.wallpaper(node);
                    Ok(())
                }
                "display" => self.display(node, &mut config.displays),
                LEGACY_DISPLAY => self.display(node, &mut config.outputs),
                "arrangement" => self.arrangement(node).and_then(|placements| {
                    let displays: Vec<String> = placements
                        .iter()
                        .map(|placement| placement.display.clone())
                        .collect();
                    if config.arrangement(&displays).is_some() {
                        return Err(
                            self.at_node(node, "these displays are arranged more than once")
                        );
                    }
                    config.arrangements.push(placements);
                    Ok(())
                }),
                _ => Err(self.at_node(node, format!("unknown setting `{written}`"))),
            };
            self.settle(result);
        }
        config
    }

    fn children<'n>(&self, node: &'n KdlNode) -> Vec<&'n KdlNode> {
        let name = node.name().value();
        let mut seen = Vec::new();
        let mut children = Vec::new();
        for child in node.children().map_or(&[][..], KdlDocument::nodes) {
            let child_name = child.name().value();
            if seen.contains(&child_name) {
                self.settle(Err(self.at_node(
                    child,
                    format!("`{name} {child_name}` is set more than once"),
                )));
                continue;
            }
            seen.push(child_name);
            children.push(child);
        }
        children
    }

    fn section<'n>(
        &self,
        node: &'n KdlNode,
        mut apply: impl FnMut(&'n KdlNode) -> Result<(), ConfigError>,
    ) {
        if let Some(entry) = node.entries().first() {
            self.settle(Err(self.at_entry(
                entry,
                format!("`{}` takes no arguments", node.name().value()),
            )));
        }
        self.each_child(node, &mut apply);
    }

    fn each_child<'n>(
        &self,
        node: &'n KdlNode,
        apply: &mut impl FnMut(&'n KdlNode) -> Result<(), ConfigError>,
    ) {
        for child in self.children(node) {
            self.settle(apply(child));
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

    fn accessibility(&self, node: &KdlNode) -> bool {
        let mut reduce_motion = false;
        self.section(node, |child| {
            match child.name().value() {
                "reduce-motion" => reduce_motion = self.flag(child)?,
                _ => return Err(self.unknown_child("accessibility", child)),
            }
            Ok(())
        });
        reduce_motion
    }

    fn flag(&self, node: &KdlNode) -> Result<bool, ConfigError> {
        let entry = self.argument(node)?;
        entry.value().as_bool().ok_or_else(|| {
            self.at_entry(
                entry,
                format!("`{}` must be #true or #false", node.name().value()),
            )
        })
    }

    fn bar(&self, node: &KdlNode) -> Bar {
        let mut bar = Bar::default();
        self.section(node, |child| {
            match child.name().value() {
                "menu-bar" => {
                    bar.menu_bar = self.keyword(
                        child,
                        &[
                            ("separate", MenuBarPlacement::Separate),
                            ("combined", MenuBarPlacement::Combined),
                        ],
                    )?
                }
                _ => return Err(self.unknown_child("bar", child)),
            }
            Ok(())
        });
        bar
    }

    fn dock(&self, node: &KdlNode) -> Dock {
        let mut dock = Dock::default();
        self.section(node, |child| {
            match child.name().value() {
                "edge" => {
                    dock.edge = self.keyword(
                        child,
                        &[
                            ("left", DockEdge::Left),
                            ("bottom", DockEdge::Bottom),
                            ("right", DockEdge::Right),
                            ("top", DockEdge::Top),
                        ],
                    )?
                }
                "apps" => dock.apps = self.app_ids(child)?,
                "auto-hide" => dock.auto_hide = self.flag(child)?,
                "size" => dock.icon_size = self.integer_in(child, DOCK_ICON_SIZE_RANGE)?,
                _ => return Err(self.unknown_child("dock", child)),
            }
            Ok(())
        });
        dock
    }

    fn app_ids(&self, node: &KdlNode) -> Result<Vec<String>, ConfigError> {
        let mut apps: Vec<String> = Vec::new();
        for entry in node.entries() {
            let app = entry
                .value()
                .as_string()
                .filter(|app| entry.name().is_none() && !app.is_empty())
                .ok_or_else(|| {
                    self.at_entry(
                        entry,
                        "`dock apps` takes app IDs such as \"org.unbound.Files\"",
                    )
                })?;
            if apps.iter().any(|known| known == app) {
                return Err(self.at_entry(entry, format!("\"{app}\" is in the dock twice")));
            }
            apps.push(app.to_owned());
        }
        Ok(apps)
    }

    fn idle(&self, node: &KdlNode, mut idle: Idle) -> Idle {
        let mut lock_node = node;
        let mut suspend_node = node;
        self.section(node, |child| {
            match child.name().value() {
                "dim-after" => idle.dim_after = self.duration(child)?,
                "lock-after" => {
                    idle.lock_after = self.duration(child)?;
                    lock_node = child;
                }
                "suspend-after" => {
                    idle.suspend = match self.argument(child)?.value().as_string() {
                        Some("never") => Suspend::Never,
                        _ => Suspend::After(self.duration(child).map_err(|error| ConfigError {
                            message: format!("{} or \"never\"", error.message),
                            ..error
                        })?),
                    };
                    suspend_node = child;
                }
                _ => return Err(self.unknown_child("idle", child)),
            }
            Ok(())
        });
        if idle.lock_after < idle.dim_after {
            self.settle(Err(self.at_node(
                lock_node,
                "`idle lock-after` must not be shorter than `idle dim-after`",
            )));
            idle.lock_after = idle.dim_after;
        }
        if let Suspend::After(after) = idle.suspend
            && after < idle.lock_after
        {
            self.settle(Err(self.at_node(
                suspend_node,
                "`idle suspend-after` must not be shorter than `idle lock-after`",
            )));
            idle.suspend = Suspend::After(idle.lock_after);
        }
        idle
    }

    fn keyboard(&self, node: &KdlNode, keyboard: &mut Keyboard) {
        self.section(node, |child| {
            match child.name().value() {
                "repeat-rate" => {
                    keyboard.repeat_rate = self.integer_in(child, KEY_REPEAT_RATE_RANGE)?
                }
                "repeat-delay" => {
                    keyboard.repeat_delay = Duration::from_millis(u64::from(
                        self.integer_in(child, KEY_REPEAT_DELAY_RANGE)?,
                    ))
                }
                "input-sources" => keyboard.input_sources = self.input_sources(child)?,
                "caps-lock" => {
                    keyboard.caps_lock = self.keyword(
                        child,
                        &CapsLock::ALL.map(|choice| (choice.keyword(), choice)),
                    )?
                }
                _ => return Err(self.unknown_child("keyboard", child)),
            }
            Ok(())
        });
    }

    fn input_sources(&self, node: &KdlNode) -> Result<Vec<InputSource>, ConfigError> {
        let mut sources: Vec<InputSource> = Vec::new();
        for entry in node.entries() {
            let source = entry
                .value()
                .as_string()
                .filter(|_| entry.name().is_none())
                .and_then(InputSource::parse)
                .ok_or_else(|| {
                    self.at_entry(
                        entry,
                        "`keyboard input-sources` takes keyboard layouts such as \"us\" or \"de(nodeadkeys)\"",
                    )
                })?;
            if sources.contains(&source) {
                return Err(self.at_entry(
                    entry,
                    format!("\"{}\" is an input source twice", source.code()),
                ));
            }
            if sources.len() == MAX_INPUT_SOURCES {
                return Err(self.at_entry(
                    entry,
                    format!("`keyboard input-sources` takes at most {MAX_INPUT_SOURCES} layouts"),
                ));
            }
            sources.push(source);
        }
        Ok(sources)
    }

    fn trackpad(&self, node: &KdlNode) -> Trackpad {
        let mut trackpad = Trackpad::default();
        self.section(node, |child| {
            match child.name().value() {
                "tap-to-click" => trackpad.tap_to_click = self.flag(child)?,
                "natural-scrolling" => trackpad.natural_scrolling = self.flag(child)?,
                "speed" => trackpad.speed = self.number_in(child, POINTER_SPEED_RANGE)?,
                "secondary-click" => {
                    trackpad.secondary_click = self.keyword(
                        child,
                        &[SecondaryClick::TwoFingers, SecondaryClick::Corner]
                            .map(|choice| (choice.keyword(), choice)),
                    )?
                }
                "disable-while-typing" => trackpad.disable_while_typing = self.flag(child)?,
                _ => return Err(self.unknown_child("trackpad", child)),
            }
            Ok(())
        });
        trackpad
    }

    fn mouse(&self, node: &KdlNode) -> Mouse {
        let mut mouse = Mouse::default();
        self.section(node, |child| {
            match child.name().value() {
                "natural-scrolling" => mouse.natural_scrolling = self.flag(child)?,
                "speed" => mouse.speed = self.number_in(child, POINTER_SPEED_RANGE)?,
                _ => return Err(self.unknown_child("mouse", child)),
            }
            Ok(())
        });
        mouse
    }

    fn sound(&self, node: &KdlNode) -> Sound {
        let mut sound = Sound::default();
        self.section(node, |child| {
            match child.name().value() {
                "effects" => sound.effects = self.flag(child)?,
                "volume-feedback" => sound.volume_feedback = self.flag(child)?,
                "alert" => {
                    let choices = AlertSound::ALL.map(|alert| (alert.keyword(), alert));
                    sound.alert = self.keyword(child, &choices)?;
                }
                "alert-volume" => sound.alert_volume = self.number_in(child, ALERT_VOLUME_RANGE)?,
                _ => return Err(self.unknown_child("sound", child)),
            }
            Ok(())
        });
        sound
    }

    fn wallpaper(&self, node: &KdlNode) -> Wallpaper {
        let mut wallpaper = Wallpaper::default();
        self.section(node, |child| {
            match child.name().value() {
                "light" => wallpaper.light = Some(self.image_path(child)?),
                "dark" => wallpaper.dark = Some(self.image_path(child)?),
                "fit" => {
                    wallpaper.fit = self.keyword(
                        child,
                        &[("fill", WallpaperFit::Fill), ("fit", WallpaperFit::Fit)],
                    )?
                }
                _ => return Err(self.unknown_child("wallpaper", child)),
            }
            Ok(())
        });
        wallpaper
    }

    fn image_path(&self, node: &KdlNode) -> Result<PathBuf, ConfigError> {
        let entry = self.argument(node)?;
        entry
            .value()
            .as_string()
            .filter(|path| is_wallpaper_path(path))
            .map(PathBuf::from)
            .ok_or_else(|| {
                self.at_entry(
                    entry,
                    format!(
                        "`wallpaper {}` must be an absolute path or start with \"~/\"",
                        node.name().value()
                    ),
                )
            })
    }

    fn integer_in(&self, node: &KdlNode, (min, max): (u32, u32)) -> Result<u32, ConfigError> {
        let entry = self.argument(node)?;
        match entry.value() {
            KdlValue::Integer(value) => u32::try_from(*value).ok(),
            _ => None,
        }
        .filter(|value| (min..=max).contains(value))
        .ok_or_else(|| {
            self.at_entry(
                entry,
                format!(
                    "`{}` must be a whole number from {min} to {max}",
                    node.name().value()
                ),
            )
        })
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

    fn identity(&self, node: &KdlNode, entry: &KdlEntry) -> Result<String, ConfigError> {
        let example = if node.name().value() == LEGACY_DISPLAY {
            "connector such as \"eDP-1\""
        } else {
            "name such as \"Dell Inc. DELL U2720Q 7ZD5K53\""
        };
        entry
            .value()
            .as_string()
            .filter(|identity| entry.name().is_none() && !identity.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                self.at_entry(
                    entry,
                    format!("`{}` needs the display's {example}", node.name().value()),
                )
            })
    }

    fn display(
        &self,
        node: &KdlNode,
        displays: &mut BTreeMap<String, DisplaySettings>,
    ) -> Result<(), ConfigError> {
        let identity = self.identity(node, self.argument(node)?)?;
        if displays.contains_key(&identity) {
            return Err(self.at_node(
                node,
                format!(
                    "{} \"{identity}\" is configured more than once",
                    node.name().value()
                ),
            ));
        }
        let mut display = DisplaySettings::default();
        self.each_child(node, &mut |child| {
            match child.name().value() {
                "resolution" => display.resolution = Some(self.resolution(child)?),
                "scale" => display.scale = Some(self.number_in(child, SCALE_RANGE)?),
                "refresh-rate" => {
                    display.refresh_rate = Some(self.number_in(child, REFRESH_RATE_RANGE)?)
                }
                "rotation" => display.rotation = Some(self.rotation(child)?),
                "variable-refresh-rate" => display.variable_refresh_rate = Some(self.flag(child)?),
                _ => return Err(self.unknown_child(node.name().value(), child)),
            }
            Ok(())
        });
        displays.insert(identity, display);
        Ok(())
    }

    fn resolution(&self, node: &KdlNode) -> Result<(u32, u32), ConfigError> {
        let pixels = |entry: &KdlEntry| match entry.value() {
            KdlValue::Integer(value) if entry.name().is_none() => {
                u32::try_from(*value).ok().filter(|value| *value > 0)
            }
            _ => None,
        };
        match node.entries() {
            [width, height] => pixels(width).zip(pixels(height)),
            _ => None,
        }
        .ok_or_else(|| {
            self.at_node(
                node,
                "`resolution` needs a width and a height in pixels such as 3840 2160",
            )
        })
    }

    fn rotation(&self, node: &KdlNode) -> Result<Rotation, ConfigError> {
        let entry = self.argument(node)?;
        match entry.value() {
            KdlValue::Integer(value) => u32::try_from(*value).ok(),
            _ => None,
        }
        .and_then(Rotation::from_degrees)
        .ok_or_else(|| self.at_entry(entry, "`rotation` must be 0, 90, 180 or 270"))
    }

    fn arrangement(&self, node: &KdlNode) -> Result<Vec<Placement>, ConfigError> {
        if let Some(entry) = node.entries().first() {
            return Err(self.at_entry(entry, "`arrangement` takes no arguments"));
        }
        let mut placements: Vec<Placement> = Vec::new();
        for child in node.children().map_or(&[][..], KdlDocument::nodes) {
            if child.name().value() != "display" {
                return Err(self.unknown_child("arrangement", child));
            }
            let mut entries = child.entries().iter();
            let identity = match entries.next() {
                Some(entry) => self.identity(child, entry)?,
                None => return Err(self.at_node(child, "`display` needs the display's name")),
            };
            if placements.iter().any(|known| known.display == identity) {
                return Err(self.at_node(child, format!("\"{identity}\" is placed twice")));
            }
            let mut placement = Placement {
                display: identity,
                x: 0,
                y: 0,
                main: false,
                mirror: None,
            };
            for entry in entries {
                let invalid = || {
                    self.at_entry(
                        entry,
                        "`display` takes x=, y=, main=#true and mirror=\"display\"",
                    )
                };
                let coordinate = || match entry.value() {
                    KdlValue::Integer(value) => i32::try_from(*value).ok(),
                    _ => None,
                };
                match entry.name().map(|name| name.value()) {
                    Some("x") => placement.x = coordinate().ok_or_else(invalid)?,
                    Some("y") => placement.y = coordinate().ok_or_else(invalid)?,
                    Some("main") => placement.main = entry.value().as_bool().ok_or_else(invalid)?,
                    Some("mirror") => {
                        placement.mirror = Some(
                            entry
                                .value()
                                .as_string()
                                .filter(|display| !display.is_empty())
                                .ok_or_else(invalid)?
                                .to_owned(),
                        )
                    }
                    _ => return Err(invalid()),
                }
            }
            placements.push(placement);
        }
        if placements.is_empty() {
            return Err(self.at_node(node, "`arrangement` needs at least one display"));
        }
        let mirrors_elsewhere = |placement: &Placement| {
            placement.mirror.as_ref().is_some_and(|mirror| {
                *mirror == placement.display
                    || !placements.iter().any(|other| other.display == *mirror)
            })
        };
        if placements.iter().any(mirrors_elsewhere) {
            return Err(self.at_node(
                node,
                "`mirror` must name another display of the same arrangement",
            ));
        }
        if placements.iter().filter(|placement| placement.main).count() > 1 {
            return Err(self.at_node(node, "only one display can be the main display"));
        }
        Ok(placements)
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

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("main.rs"),
            include_str!("accounts.rs"),
            include_str!("connectors.rs"),
            include_str!("edit.rs"),
            include_str!("report.rs"),
            include_str!("service.rs"),
            include_str!("store.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
