mod access;
mod app_connection;
mod appearance;
mod chrome;
mod clipboard;
mod lock;
mod popup;
mod presentation;
mod primary;
mod run;
mod scale;
mod seat;
mod shell;
mod text_input;
mod window;

use std::sync::LazyLock;

use bande_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/bande-toolkit-platform.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub use app_connection::{AppConnection, AppRequest};
pub use bande_graphics::ResizeEdge;
pub use bande_protocols::client::bande_shell_v1::{
    BrightnessKey, Edge as DisplayEdge, LauncherMode, MediaKey, Panel, SessionEnd, VolumeKey,
};
pub use chrome::{FrameRegions, Header, SHEET_INSET, WINDOW_HEADER_HEIGHT, framed_scene};
pub use clipboard::{DragAction, DragEvent, DragIcon, Transfer};
pub use popup::MenuPlacement;
pub use run::{App, Surfaces, run, run_surfaces};
pub use shell::AppWindow;
pub use window::{
    ACTIVATION_TOKEN_VARIABLE, Edges, FramePath, KeyboardFocus, LayerKind, LayerOptions,
    RENDERER_VARIABLE, Role, ToplevelState, Window, WindowError, WindowEvent,
};

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [include_str!("window/menus.rs")];
        assert_eq!(
            bande_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
