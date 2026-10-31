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
mod night;
mod overview;
mod pointing;
mod popup;
mod recall;
mod render;
mod reveal;
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
mod window_menu;
mod workspace;
mod zoom;

pub use appearance::{SystemAppearance, follow_appearance};
pub use assistive::follow_alerts;
pub use clock::{FrameClock, FrameOutput, FrameScheduler, Poll, SAFETY_MARGIN, Submission, Vblank};
pub use cursor::{
    ARROW_FILL, ARROW_HEIGHT, ARROW_OUTLINE, ARROW_WIDTH, CursorTheme, DEFAULT_CURSOR_SIZE,
};
pub use drm::{DrmBackend, DrmBackendError, DrmNotifiers, HeadChanges};
pub use element::ScreenElement;
pub use environment::{export_session_environment, notify_ready};
pub use gesture::Gesture;
pub use input_settings::{InputSettings, follow_input_settings};
pub use keyboard_monitor::{KEYBOARD_MONITOR_NAME, KeyboardMonitorHandle, serve_keyboard_monitor};
pub use keymap::KeyboardSettings;
pub use logind::{SessionRequest, Sleep, follow_session, follow_shutdown, follow_sleep};
pub use night::NightTick;
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
