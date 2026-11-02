mod access;
mod access_keys;
mod alert;
mod app;
mod appearance;
mod apps;
mod arrange;
mod assist;
mod assistive;
mod capture;
mod capture_shell;
mod cascade;
mod clock;
mod constraint;
mod cursor;
mod decoration;
mod displays;
mod drag;
mod drm;
mod effect;
mod element;
mod environment;
mod family;
mod force_quit;
mod frame;
mod gesture;
mod grab;
mod hot_corner;
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
mod menu_button;
mod motion;
mod night;
mod output_power;
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
pub use bande_render::{DeviceInfo, RenderError};
pub use clock::{FrameClock, FrameOutput, FrameScheduler, Poll, SAFETY_MARGIN, Submission, Vblank};
pub use cursor::{
    ARROW_FILL, ARROW_HEIGHT, ARROW_OUTLINE, ARROW_WIDTH, CursorTheme, DEFAULT_CURSOR_SIZE,
};
pub use drm::{DrmBackend, DrmBackendError, DrmNotifiers, HeadChanges};
pub use element::ScreenElement;
pub use environment::{
    export_session_environment, notify_service_manager, watchdog_keepalive_interval,
};
pub use gesture::Gesture;
pub use input_settings::{InputSettings, follow_input_settings, user_config};
pub use keyboard_monitor::{
    KEYBOARD_MONITOR_NAME, KeyboardMonitorHandle, SCREEN_READER_NAME, serve_keyboard_monitor,
};
pub use keymap::KeyboardSettings;
pub use logind::{SessionRequest, Sleep, follow_session, follow_shutdown, follow_sleep};
pub use night::NightTick;
pub use pointing::PointerSettings;
pub use render::{BACKGROUND, FrameStats, Renderer, Snapshot};
pub use screen::{OutputConfig, OutputMode, OutputSettings, automatic_scale};
pub use session::{
    CLIPBOARD_DAEMON, GREETER, LOCK_SCREEN, REVIEW, SCREENSHOT, SESSION_CLIENT_RESTART_THROTTLE,
    SHELL, SessionRole, SessionUpkeep,
};
pub use state::{Compositor, OutputElement, REFRESH, ScanoutDevice};
pub use watchdog::UNRESPONSIVE_AFTER;
pub use window::Window;

const ENGLISH: &str = include_str!("../locales/en-US/bande-compositor.ftl");

static L10N: std::sync::LazyLock<bande_l10n::Localizer> =
    std::sync::LazyLock::new(|| bande_l10n::Localizer::new(&[("en-US", ENGLISH)]));
