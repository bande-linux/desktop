mod registry;
mod search;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use registry::{
    AUTOMATIC, Choice, Group, Kind, List, MAX_SEGMENTS, POINTER_SPEEDS, Pane, PaneId, Permission,
    SCALES, Section, Setting, SettingId, TEXT_SIZES, Value, WITH_DISPLAY, duration_label, panes,
    without_sleep,
};
pub use search::{PaneHit, SettingHit, search};

static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-settings-panes")));
