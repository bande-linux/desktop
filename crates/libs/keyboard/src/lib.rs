mod layouts;
mod shortcuts;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use layouts::{EVDEV_RULES, Layout, Layouts};
pub use shortcuts::{Action, Area, SHORTCUTS, Shortcut, shortcuts_in};

const ENGLISH: &str = include_str!("../locales/en-US/bande-keyboard.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
