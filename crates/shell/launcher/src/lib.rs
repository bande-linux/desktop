mod client;
mod clips;
mod emoji;
mod model;
mod preview;
mod rows;
mod tones;
mod view;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use client::{LauncherShell, role};
pub use clips::{Clips, Reply, Request};
pub use model::{Action, AppInfo, Clip, Heading, Hit, Kind, Mode, PaneInfo, SettingInfo, Target};
pub use view::{CARD_WIDTH, Catalog, ICON_SIZE, Launcher, Loaded, Surface};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-launcher.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
