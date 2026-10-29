mod client;
mod model;
mod view;

pub use client::SettingsClient;
pub use model::{Choice, Control, Key, Match, Pane, Value, panes, search};
pub use view::{SIDEBAR_WIDTH, SettingsView};
