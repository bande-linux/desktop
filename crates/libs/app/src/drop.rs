use std::path::{Path, PathBuf};

use bande_toolkit::{DragAction, DragEvent, Window, WindowEvent};

/// What [`FileDrop::event`] made of a window event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dropping {
    /// Not a drag of files this window takes: handle the event as usual.
    Other,
    /// A drag of files the window takes is over it.
    Over,
    /// These files were dropped on the window.
    Dropped(Vec<PathBuf>),
}

/// The local files among `uris` that `accepts` takes.
pub fn local_files(uris: &[String], accepts: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    uris.iter()
        .filter_map(|uri| bande_xdg::path_from_file_uri(uri))
        .filter(|path| accepts(path))
        .collect()
}

/// Takes files dragged onto a window, such as from Files, for the app to open, attach or
/// import.
#[derive(Debug, Default)]
pub struct FileDrop {
    offered: Vec<PathBuf>,
}

impl FileDrop {
    /// Accepts a drag that carries local files `accepts` takes, as a copy, and hands them back
    /// once dropped. Any other event, a drag of text among them, is left to the window.
    pub fn event(
        &mut self,
        window: &mut Window,
        event: &WindowEvent,
        accepts: impl Fn(&Path) -> bool,
    ) -> Dropping {
        let WindowEvent::Drag(drag) = event else {
            return Dropping::Other;
        };
        match drag {
            DragEvent::Entered { uris, .. } => {
                self.offered = local_files(uris, accepts);
            }
            DragEvent::Dropped { .. } if !self.offered.is_empty() => {
                return Dropping::Dropped(std::mem::take(&mut self.offered));
            }
            DragEvent::Left | DragEvent::Finished(_) | DragEvent::Dropped { .. } => {
                self.offered.clear();
                return Dropping::Other;
            }
            DragEvent::Moved(_) => {}
        }
        if self.offered.is_empty() {
            return Dropping::Other;
        }
        window.accept_drag(Some(DragAction::Copy));
        Dropping::Over
    }
}
