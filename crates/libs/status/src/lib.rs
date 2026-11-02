pub mod backlight;
pub mod captive;
pub mod clock;
pub mod dbusmenu;
pub mod devices;
pub mod history;
pub mod hub;
pub mod memory;
pub mod session;
pub mod status;
pub mod tray;
pub mod unread;

mod command;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use crate::command::Command;
use crate::dbusmenu::Node;
use crate::tray::TrayItem;

const ENGLISH: &str = include_str!("../locales/en-US/bande-status.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[derive(Debug, Clone)]
pub enum Update {
    Devices(devices::Devices),
    Sound(Option<status::Sound>),
    Capturing(Vec<bande_audio::Capture>),
    Tray(Vec<TrayItem>),
    Menu { menu: String, root: Node },
    AppMenu { menu: String, root: Option<Node> },
    AppMenuChanged,
    ClockFormat(String),
    Minute,
    BatteryPercentage(bool),
    ClockMark(unread::ClockMark),
    BarLayout { layout: String, edge: String },
}
