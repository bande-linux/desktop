mod layouts;
mod shortcuts;

pub use layouts::{EVDEV_RULES, Layout, Layouts};
pub use shortcuts::{
    Action, Area, Key, Modifiers, SHORTCUTS, Shortcut, bound, keys_for, keys_of, shortcuts_in,
};
