mod layouts;
mod shortcuts;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use layouts::{EVDEV_RULES, Layout, Layouts};
pub use shortcuts::{
    Action, Area, Key, Modifiers, SHORTCUTS, Shortcut, bound, keys_for, keys_of, shortcuts_in,
};

static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-keyboard")));
