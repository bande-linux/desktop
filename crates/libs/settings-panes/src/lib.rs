mod registry;
mod search;

pub use registry::{
    AUTOMATIC, Choice, Computer, Group, Kind, List, MAX_SEGMENTS, Mark, POINTER_SPEEDS, Pane,
    PaneId, Permission, SCALES, Section, Setting, SettingId, TEXT_SIZES, Value, WITH_DISPLAY,
    duration_label, panes, panes_of, without_battery, without_sleep,
};
pub use search::{PaneHit, SettingHit, search};
