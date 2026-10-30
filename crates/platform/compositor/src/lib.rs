mod access;
mod access_keys;
mod alert;
mod app;
mod appearance;
mod arrange;
mod assist;
mod assistive;
mod capture;
mod capture_shell;
mod cascade;
mod clock;
mod cursor;
mod decoration;
mod displays;
mod dock;
mod drag;
mod drm;
mod element;

mod effect;
mod environment;
mod family;
mod frame;
mod gesture;
mod grab;
mod idle;
mod ime;
mod input;
mod input_settings;
mod keyboard_monitor;
mod keymap;
mod label;
mod lock;
mod logind;
mod management;
mod motion;
mod overview;
mod pointing;
mod popup;
mod recall;
mod render;
mod scanout;
mod scene;
mod screen;
mod session;
mod shell;
mod state;
mod switcher;
mod sync;
mod tile;
mod typing;
mod watchdog;
mod window;
mod workspace;
mod zoom;

pub use appearance::{SystemAppearance, follow_appearance};
pub use assistive::follow_alerts;
pub use clock::{FrameClock, FrameOutput, FrameScheduler, Poll, SAFETY_MARGIN, Submission, Vblank};
pub use cursor::{
    ARROW_FILL, ARROW_HEIGHT, ARROW_OUTLINE, ARROW_WIDTH, CursorTheme, DEFAULT_CURSOR_SIZE,
    DEFAULT_CURSOR_THEME,
};
pub use drm::{DrmBackend, DrmBackendError, DrmNotifiers, HeadChanges};
pub use element::ScreenElement;
pub use environment::{
    DESKTOP_NAME, export_session_environment, notify_ready, session_environment,
};
pub use gesture::Gesture;
pub use input_settings::{InputSettings, follow_input_settings};
pub use keyboard_monitor::{KEYBOARD_MONITOR_NAME, KeyboardMonitorHandle, serve_keyboard_monitor};
pub use keymap::KeyboardSettings;
pub use logind::{SessionRequest, Sleep, follow_session, follow_sleep};
pub use pointing::PointerSettings;
pub use render::{BACKGROUND, FrameStats, Renderer, Snapshot};
pub use screen::{OutputConfig, OutputMode, OutputSettings, automatic_scale};
pub use session::{
    CLIPBOARD_DAEMON, GREETER, LOCK_SCREEN, MONITOR, REVIEW, SCREENSHOT,
    SESSION_CLIENT_RESTART_THROTTLE, SHELL, SessionRole, SessionUpkeep,
};
pub use state::{Headless, OutputElement, REFRESH, Rect, ScanoutDevice};
pub use unbound_render::{DeviceInfo, RenderError};
pub use watchdog::UNRESPONSIVE_AFTER;
pub use window::Window;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-compositor.ftl");

static L10N: std::sync::LazyLock<unbound_l10n::Localizer> =
    std::sync::LazyLock::new(|| unbound_l10n::Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [include_str!("switcher.rs")];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
