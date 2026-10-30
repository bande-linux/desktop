pub mod clock;
pub mod dbusmenu;
pub mod devices;
pub mod session;
pub mod status;
pub mod tray;
pub mod unread;

mod command;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use crate::command::Command;
use crate::dbusmenu::Node;
use crate::tray::TrayItem;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-status.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[derive(Debug, Clone)]
pub enum Update {
    Devices(devices::Devices),
    Sound(Option<status::Sound>),
    Tray(Vec<TrayItem>),
    Menu { menu: String, root: Node },
    AppMenu { app: String, root: Option<Node> },
    AppMenuChanged,
    ClockFormat(String),
    Minute,
    BatteryPercentage(bool),
    Unread(bool),
    BarLayout { menu_bar: String, edge: String },
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &[include_str!("session.rs")]),
            Vec::<&str>::new()
        );
    }
}
