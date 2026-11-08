mod registry;
mod search;

pub use registry::{
    AUTOMATIC, Choice, Group, Kind, List, MAX_SEGMENTS, POINTER_SPEEDS, Pane, PaneId, Permission,
    SCALES, Section, Setting, SettingId, TEXT_SIZES, Value, WITH_DISPLAY, duration_label, panes,
    without_sleep,
};
pub use search::{PaneHit, SettingHit, search};
