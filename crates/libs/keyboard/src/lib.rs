mod layouts;
mod shortcuts;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use layouts::{EVDEV_RULES, Layout, Layouts};
pub use shortcuts::{Action, Area, SHORTCUTS, Shortcut, shortcuts_in};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-keyboard.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
