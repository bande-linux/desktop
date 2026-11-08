pub mod backlight;
pub mod brightness;
pub mod captive;
pub mod clock;
pub mod clock_app;
pub mod dbusmenu;
pub mod devices;
pub mod gtkmenu;
pub mod history;
pub mod hub;
pub mod memory;
pub mod session;
#[cfg(feature = "audio")]
mod sound;
pub mod status;
pub mod tray;
pub mod unread;

mod command;

pub use bande_bus::accounts;

pub use crate::command::Command;
use crate::dbusmenu::Node;
use crate::tray::TrayItem;

#[derive(Debug, Clone)]
pub enum Update {
    Devices(devices::Devices),
    Sound(Option<status::Sound>),
    #[cfg(feature = "audio")]
    Capturing(Vec<bande_audio::Capture>),
    Tray(Vec<TrayItem>),
    Menu {
        menu: String,
        root: Node,
    },
    AppMenu {
        menu: String,
        root: Option<Node>,
    },
    AppMenuChanged,
    GtkMenuChanged(String),
    ClockFormat(String),
    Minute,
    BatteryPercentage(bool),
    ClockMark(unread::ClockMark),
    ClockApp(clock_app::ClockApp),
    BarEdge(String),
}
