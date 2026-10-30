mod access;
mod appearance;
mod capture;
mod chrome;
mod clipboard;
mod effect;
mod lock;
mod popup;
mod run;
mod scale;
mod seat;
mod shell;
mod text_input;
mod window;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-toolkit-platform.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub use capture::ScreenImage;
pub use chrome::TITLE_BAR_HEIGHT;
pub use clipboard::{DragAction, DragEvent, DragIcon, Transfer};
pub use popup::MenuPlacement;
pub use run::{App, run};
pub use shell::AppWindow;
pub use unbound_protocols::client::unbound_shell_v1::{BrightnessKey, VolumeKey};
pub use window::{
    ACTIVATION_TOKEN_VARIABLE, Edges, FramePath, KeyboardFocus, LayerKind, LayerOptions,
    RENDERER_VARIABLE, ResizeEdge, Role, ToplevelState, Window, WindowError, WindowEvent,
};
