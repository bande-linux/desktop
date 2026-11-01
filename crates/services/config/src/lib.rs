mod edit;
mod login_picture;
mod report;
mod service;
mod store;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

pub use bande_design::Accent;
use bande_l10n::Localizer;
use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

pub use edit::Setting;
pub use login_picture::{
    LOGIN_PICTURE_LIMIT, LOGIN_PICTURES, copy_login_picture, login_picture, login_picture_file,
    share_login_picture,
};
pub use report::{EDIT_ACTION, ErrorNotifier};
pub use service::{
    ArrangedDisplay, BUS_NAME, ConfigService, NightLightValue, OBJECT_PATH, ServiceError,
};
pub use store::{ConfigStore, Loaded, ReloadError};

const ENGLISH: &str = include_str!("../locales/en-US/bande-config.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const SCALE_RANGE: (f64, f64) = (1.0, 3.0);
const REFRESH_RATE_RANGE: (f64, f64) = (60.0, 240.0);
pub const SCREEN_READER: &str = "orca";
pub const KEY_REPEAT_RATE_RANGE: (u32, u32) = (1, 60);
pub const KEY_REPEAT_DELAY_RANGE: (u32, u32) = (150, 2000);
pub const BAR_ICON_SIZE_RANGE: (u32, u32) = (32, 56);
pub const DEFAULT_BAR_ICON_SIZE: u32 = 40;
pub const MAX_INPUT_SOURCES: usize = 4;
pub const POINTER_SPEED_RANGE: (f64, f64) = (-1.0, 1.0);
pub const ALERT_VOLUME_RANGE: (f64, f64) = (0.0, 1.0);
pub const ZOOM_RANGE: (f64, f64) = (1.0, 16.0);
pub const TEXT_SIZE_RANGE: (u32, u32) = (100, 200);
pub const CURSOR_SIZE_RANGE: (u32, u32) = (24, 128);
pub const DEFAULT_CURSOR_SIZE: u32 = 24;
pub const SYSTEM_KEYBOARDS: [&str; 2] = ["/etc/vconsole.conf", "/etc/default/keyboard"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
    Automatic,
}

pub const ACCENTS: [(&str, Accent); 8] = [
    ("ultramarine", Accent::Ultramarine),
    ("teal", Accent::Teal),
    ("moss", Accent::Moss),
    ("amber", Accent::Amber),
    ("ember", Accent::Ember),
    ("rose", Accent::Rose),
    ("plum", Accent::Plum),
    ("graphite", Accent::Graphite),
];

pub fn accent_keyword(accent: Accent) -> &'static str {
    ACCENTS
        .iter()
        .find(|(_, candidate)| *candidate == accent)
        .map_or("ultramarine", |(keyword, _)| keyword)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClockFormat {
    #[default]
    Automatic,
    TwelveHour,
    TwentyFourHour,
}

impl ClockFormat {
    pub const ALL: [Self; 3] = [Self::Automatic, Self::TwelveHour, Self::TwentyFourHour];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::TwelveHour => "12-hour",
            Self::TwentyFourHour => "24-hour",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarEdge {
    Left,
    Bottom,
    Right,
    Top,
}

impl BarEdge {
    pub const ALL: [Self; 4] = [Self::Left, Self::Bottom, Self::Right, Self::Top];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Bottom => "bottom",
            Self::Right => "right",
            Self::Top => "top",
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|edge| edge.keyword() == keyword)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuBarPlacement {
    #[default]
    InBar,
    Separate,
}

impl MenuBarPlacement {
    pub const ALL: [Self; 2] = [Self::InBar, Self::Separate];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::InBar => "in-bar",
            Self::Separate => "separate",
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|placement| placement.keyword() == keyword)
    }

    pub fn combines(self, edge: BarEdge) -> bool {
        self == Self::InBar && edge == BarEdge::Left
    }
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

pub fn system_keyboard<'a>(files: &[&'a str]) -> &'a str {
    files
        .iter()
        .copied()
        .find(|text| {
            text.lines().any(|line| {
                line.trim()
                    .strip_prefix("XKBLAYOUT=")
                    .is_some_and(|value| !value.trim().trim_matches('"').is_empty())
            })
        })
        .unwrap_or_default()
}

pub fn read_system_input_sources() -> Vec<InputSource> {
    let files = SYSTEM_KEYBOARDS.map(|path| std::fs::read_to_string(path).unwrap_or_default());
    system_input_sources(system_keyboard(&files.each_ref().map(String::as_str)))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dragging {
    Off,
    #[default]
    Tap,
    TapWithLock,
}

impl Dragging {
    pub const ALL: [Self; 3] = [Self::Off, Self::Tap, Self::TapWithLock];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Tap => "tap",
            Self::TapWithLock => "tap-with-lock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollMethod {
    #[default]
    TwoFingers,
    Edge,
}

impl ScrollMethod {
    pub const ALL: [Self; 2] = [Self::TwoFingers, Self::Edge];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::TwoFingers => "two-fingers",
            Self::Edge => "edge",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrimaryButton {
    #[default]
    Left,
    Right,
}

impl PrimaryButton {
    pub const ALL: [Self; 2] = [Self::Left, Self::Right];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trackpad {
    pub tap_to_click: bool,
    pub natural_scrolling: bool,
    pub speed: f64,
    pub acceleration: bool,
    pub secondary_click: SecondaryClick,
    pub dragging: Dragging,
    pub scroll_method: ScrollMethod,
    pub disable_while_typing: bool,
    pub workspace_swipe: bool,
    pub overview_swipe: bool,
    pub desktop_spread: bool,
    pub launcher_pinch: bool,
}

impl Default for Trackpad {
    fn default() -> Self {
        Self {
            tap_to_click: true,
            natural_scrolling: true,
            speed: 0.0,
            acceleration: true,
            secondary_click: SecondaryClick::TwoFingers,
            dragging: Dragging::Tap,
            scroll_method: ScrollMethod::TwoFingers,
            disable_while_typing: true,
            workspace_swipe: true,
            overview_swipe: true,
            desktop_spread: true,
            launcher_pinch: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mouse {
    pub primary_button: PrimaryButton,
    pub natural_scrolling: bool,
    pub speed: f64,
    pub acceleration: bool,
}

impl Default for Mouse {
    fn default() -> Self {
        Self {
            primary_button: PrimaryButton::Left,
            natural_scrolling: false,
            speed: 0.0,
            acceleration: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LidOnPower {
    #[default]
    Sleep,
    KeepRunning,
}

impl LidOnPower {
    pub const ALL: [Self; 2] = [Self::Sleep, Self::KeepRunning];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::Sleep => "sleep",
            Self::KeepRunning => "keep-running",
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Power {
    pub show_battery_percentage: bool,
    pub low_power_on_battery: bool,
    pub lid_on_power: LidOnPower,
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

pub const DEFAULT_BAR_APPS: [&str; 3] = [
    "org.bande.Files",
    "org.bande.Terminal",
    "org.bande.Settings",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bar {
    pub edge: BarEdge,
    pub apps: Vec<String>,
    pub auto_hide: bool,
    pub icon_size: u32,
    pub menu_bar: MenuBarPlacement,
}

impl Default for Bar {
    fn default() -> Self {
        Self {
            edge: BarEdge::Left,
            apps: DEFAULT_BAR_APPS.map(str::to_owned).to_vec(),
            auto_hide: false,
            icon_size: DEFAULT_BAR_ICON_SIZE,
            menu_bar: MenuBarPlacement::InBar,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accessibility {
    pub screen_reader: bool,
    pub zoom: f64,
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
    pub reduce_motion: bool,
    pub text_size: u32,
    pub cursor_size: u32,
    pub flash_screen: bool,
    pub sticky_keys: bool,
    pub slow_keys: bool,
    pub bounce_keys: bool,
}

impl Default for Accessibility {
    fn default() -> Self {
        Self {
            screen_reader: false,
            zoom: 1.0,
            increase_contrast: false,
            reduce_transparency: false,
            reduce_motion: false,
            text_size: 100,
            cursor_size: DEFAULT_CURSOR_SIZE,
            flash_screen: false,
            sticky_keys: false,
            slow_keys: false,
            bounce_keys: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockTime {
    pub hour: u8,
    pub minute: u8,
}

impl ClockTime {
    pub fn parse(text: &str) -> Option<Self> {
        let (hour, minute) = text.split_once(':')?;
        let digits = |part: &str| {
            (part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| part.parse::<u8>().ok())
                .flatten()
        };
        let (hour, minute) = (digits(hour)?, digits(minute)?);
        (hour < 24 && minute < 60).then_some(Self { hour, minute })
    }

    pub fn text(self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NightSchedule {
    Sunset,
    Custom,
}

impl NightSchedule {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Sunset => "sunset",
            Self::Custom => "custom",
        }
    }
}

pub const NIGHT_LIGHT_WARMTH_RANGE: (u32, u32) = (0, 100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NightLight {
    pub on: bool,
    pub schedule: NightSchedule,
    pub from: ClockTime,
    pub to: ClockTime,
    pub warmth: u32,
}

impl Default for NightLight {
    fn default() -> Self {
        Self {
            on: false,
            schedule: NightSchedule::Sunset,
            from: ClockTime {
                hour: 22,
                minute: 0,
            },
            to: ClockTime { hour: 7, minute: 0 },
            warmth: 50,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SearchCategory {
    Applications,
    RecentApps,
    RecentDocuments,
    Documents,
    Folders,
    Settings,
    System,
    Calculator,
    Conversion,
}

impl SearchCategory {
    pub const ALL: [Self; 9] = [
        Self::Applications,
        Self::RecentApps,
        Self::RecentDocuments,
        Self::Documents,
        Self::Folders,
        Self::Settings,
        Self::System,
        Self::Calculator,
        Self::Conversion,
    ];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::Applications => "applications",
            Self::RecentApps => "recent-apps",
            Self::RecentDocuments => "recent-documents",
            Self::Documents => "documents",
            Self::Folders => "folders",
            Self::Settings => "system-settings",
            Self::System => "system-actions",
            Self::Calculator => "calculator",
            Self::Conversion => "conversion",
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.keyword() == keyword)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Search {
    pub hidden: Vec<SearchCategory>,
    pub exclude: Vec<PathBuf>,
}

impl Search {
    pub fn shows(&self, category: SearchCategory) -> bool {
        !self.hidden.contains(&category)
    }

    pub fn set_shown(&mut self, category: SearchCategory, shown: bool) {
        self.hidden.retain(|hidden| *hidden != category);
        if !shown {
            self.hidden.push(category);
            self.hidden.sort();
        }
    }

    pub fn excluded(&self, home: Option<&Path>) -> Vec<PathBuf> {
        self.exclude
            .iter()
            .filter_map(|path| match path.strip_prefix("~") {
                Ok(rest) => home.map(|home| home.join(rest)),
                Err(_) => Some(path.clone()),
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub appearance: Appearance,
    pub accent: Accent,
    pub tinted_frame: bool,
    pub reopen_apps: bool,
    pub reopen_apps_after_crash: bool,
    pub clock: ClockFormat,
    pub accessibility: Accessibility,
    pub bar: Bar,
    pub idle: Idle,
    pub keyboard: Keyboard,
    pub trackpad: Trackpad,
    pub mouse: Mouse,
    pub power: Power,
    pub sound: Sound,
    pub wallpaper: Wallpaper,
    pub night_light: NightLight,
    pub search: Search,
    pub displays: BTreeMap<String, DisplaySettings>,
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
            accent: Accent::default(),
            tinted_frame: false,
            reopen_apps: true,
            reopen_apps_after_crash: false,
            clock: ClockFormat::Automatic,
            accessibility: Accessibility::default(),
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
            power: Power::default(),
            sound: Sound::default(),
            wallpaper: Wallpaper::default(),
            night_light: NightLight::default(),
            search: Search::default(),
            displays: BTreeMap::new(),
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
    Some(base.join("bande").join("config.kdl"))
}

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

    pub fn display(&self, identity: &str) -> DisplaySettings {
        self.displays.get(identity).copied().unwrap_or_default()
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
            let name = node.name().value();
            if !matches!(name, "display" | "arrangement") {
                if seen.contains(&name) {
                    self.settle(Err(
                        self.at_node(node, format!("`{name}` is set more than once"))
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
                "accent" => self
                    .keyword(node, &ACCENTS)
                    .map(|accent| config.accent = accent),
                "tinted-frame" => self.flag(node).map(|tinted| config.tinted_frame = tinted),
                "reopen-apps" => self.flag(node).map(|reopen| config.reopen_apps = reopen),
                "reopen-apps-after-crash" => self
                    .flag(node)
                    .map(|reopen| config.reopen_apps_after_crash = reopen),
                "accessibility" => {
                    config.accessibility = self.accessibility(node);
                    Ok(())
                }
                "clock" => self
                    .keyword(
                        node,
                        &ClockFormat::ALL.map(|format| (format.keyword(), format)),
                    )
                    .map(|clock| config.clock = clock),
                "bar" => {
                    self.bar(node, &mut config.bar);
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
                "power" => {
                    config.power = self.power(node);
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
                "night-light" => {
                    config.night_light = self.night_light(node);
                    Ok(())
                }
                "search" => {
                    config.search = self.search(node);
                    Ok(())
                }
                "display" => self.display(node, &mut config.displays),
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
                _ => Err(self.at_node(node, format!("unknown setting `{name}`"))),
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

    fn accessibility(&self, node: &KdlNode) -> Accessibility {
        let mut access = Accessibility::default();
        self.section(node, |child| {
            match child.name().value() {
                "screen-reader" => access.screen_reader = self.flag(child)?,
                "zoom" => access.zoom = self.number_in(child, ZOOM_RANGE)?,
                "increase-contrast" => access.increase_contrast = self.flag(child)?,
                "reduce-transparency" => access.reduce_transparency = self.flag(child)?,
                "reduce-motion" => access.reduce_motion = self.flag(child)?,
                "text-size" => access.text_size = self.integer_in(child, TEXT_SIZE_RANGE)?,
                "cursor-size" => access.cursor_size = self.integer_in(child, CURSOR_SIZE_RANGE)?,
                "flash-screen" => access.flash_screen = self.flag(child)?,
                "sticky-keys" => access.sticky_keys = self.flag(child)?,
                "slow-keys" => access.slow_keys = self.flag(child)?,
                "bounce-keys" => access.bounce_keys = self.flag(child)?,
                _ => return Err(self.unknown_child("accessibility", child)),
            }
            Ok(())
        });
        access
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

    fn bar(&self, node: &KdlNode, bar: &mut Bar) {
        self.section(node, |child| {
            match child.name().value() {
                "edge" => {
                    bar.edge =
                        self.keyword(child, &BarEdge::ALL.map(|edge| (edge.keyword(), edge)))?
                }
                "apps" => bar.apps = self.app_ids(child)?,
                "auto-hide" => bar.auto_hide = self.flag(child)?,
                "names" => {}
                "size" => bar.icon_size = self.integer_in(child, BAR_ICON_SIZE_RANGE)?,
                "menu-bar" => {
                    bar.menu_bar = self.keyword(
                        child,
                        &MenuBarPlacement::ALL.map(|placement| (placement.keyword(), placement)),
                    )?
                }
                _ => return Err(self.unknown_child("bar", child)),
            }
            Ok(())
        });
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
                        "`bar apps` takes app IDs such as \"org.bande.Files\"",
                    )
                })?;
            if apps.iter().any(|known| known == app) {
                return Err(self.at_entry(entry, format!("\"{app}\" is in the bar twice")));
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

    fn night_light(&self, node: &KdlNode) -> NightLight {
        let mut night = NightLight::default();
        self.section(node, |child| {
            match child.name().value() {
                "on" => night.on = self.flag(child)?,
                "schedule" => {
                    night.schedule = self.keyword(
                        child,
                        &[NightSchedule::Sunset, NightSchedule::Custom]
                            .map(|schedule| (schedule.keyword(), schedule)),
                    )?
                }
                "from" => night.from = self.clock_time(child)?,
                "to" => night.to = self.clock_time(child)?,
                "warmth" => night.warmth = self.integer_in(child, NIGHT_LIGHT_WARMTH_RANGE)?,
                _ => return Err(self.unknown_child("night-light", child)),
            }
            Ok(())
        });
        night
    }

    fn search(&self, node: &KdlNode) -> Search {
        let mut search = Search::default();
        self.section(node, |child| {
            let name = child.name().value();
            match SearchCategory::from_keyword(name) {
                Some(category) => search.set_shown(category, self.flag(child)?),
                None if name == "exclude" => search.exclude = self.excluded_paths(child)?,
                None => return Err(self.unknown_child("search", child)),
            }
            Ok(())
        });
        search
    }

    fn excluded_paths(&self, node: &KdlNode) -> Result<Vec<PathBuf>, ConfigError> {
        let mut paths: Vec<PathBuf> = Vec::new();
        for entry in node.entries() {
            let path = entry
                .value()
                .as_string()
                .filter(|path| entry.name().is_none() && is_wallpaper_path(path))
                .map(PathBuf::from)
                .ok_or_else(|| {
                    self.at_entry(
                        entry,
                        "`search exclude` takes folders with an absolute path or one starting with \"~/\"",
                    )
                })?;
            if paths.contains(&path) {
                return Err(
                    self.at_entry(entry, format!("\"{}\" is excluded twice", path.display()))
                );
            }
            paths.push(path);
        }
        Ok(paths)
    }

    fn clock_time(&self, node: &KdlNode) -> Result<ClockTime, ConfigError> {
        let entry = self.argument(node)?;
        entry
            .value()
            .as_string()
            .and_then(ClockTime::parse)
            .ok_or_else(|| {
                self.at_entry(
                    entry,
                    format!(
                        "`{}` must be a time of day such as \"22:00\"",
                        node.name().value()
                    ),
                )
            })
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
                "acceleration" => trackpad.acceleration = self.flag(child)?,
                "dragging" => {
                    trackpad.dragging = self.keyword(
                        child,
                        &Dragging::ALL.map(|choice| (choice.keyword(), choice)),
                    )?
                }
                "scroll-method" => {
                    trackpad.scroll_method = self.keyword(
                        child,
                        &ScrollMethod::ALL.map(|choice| (choice.keyword(), choice)),
                    )?
                }
                "secondary-click" => {
                    trackpad.secondary_click = self.keyword(
                        child,
                        &[SecondaryClick::TwoFingers, SecondaryClick::Corner]
                            .map(|choice| (choice.keyword(), choice)),
                    )?
                }
                "disable-while-typing" => trackpad.disable_while_typing = self.flag(child)?,
                "workspace-swipe" => trackpad.workspace_swipe = self.flag(child)?,
                "overview-swipe" => trackpad.overview_swipe = self.flag(child)?,
                "desktop-spread" => trackpad.desktop_spread = self.flag(child)?,
                "launcher-pinch" => trackpad.launcher_pinch = self.flag(child)?,
                _ => return Err(self.unknown_child("trackpad", child)),
            }
            Ok(())
        });
        trackpad
    }

    fn power(&self, node: &KdlNode) -> Power {
        let mut power = Power::default();
        self.section(node, |child| {
            match child.name().value() {
                "show-battery-percentage" => power.show_battery_percentage = self.flag(child)?,
                "low-power-on-battery" => power.low_power_on_battery = self.flag(child)?,
                "lid-on-power" => {
                    power.lid_on_power = self.keyword(
                        child,
                        &LidOnPower::ALL.map(|choice| (choice.keyword(), choice)),
                    )?
                }
                _ => return Err(self.unknown_child("power", child)),
            }
            Ok(())
        });
        power
    }

    fn mouse(&self, node: &KdlNode) -> Mouse {
        let mut mouse = Mouse::default();
        self.section(node, |child| {
            match child.name().value() {
                "primary-button" => {
                    mouse.primary_button = self.keyword(
                        child,
                        &PrimaryButton::ALL.map(|choice| (choice.keyword(), choice)),
                    )?
                }
                "natural-scrolling" => mouse.natural_scrolling = self.flag(child)?,
                "speed" => mouse.speed = self.number_in(child, POINTER_SPEED_RANGE)?,
                "acceleration" => mouse.acceleration = self.flag(child)?,
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
        entry
            .value()
            .as_string()
            .filter(|identity| entry.name().is_none() && !identity.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                self.at_entry(
                    entry,
                    format!(
                        "`{}` needs the display's name such as \"Dell Inc. DELL U2720Q 7ZD5K53\"",
                        node.name().value()
                    ),
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
