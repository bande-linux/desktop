use std::fs;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};

use bande_inbox::Inbox;
use bande_watch::{Changes, FolderWatch};

use crate::entry::{DesktopEntry, current_desktops, data_dirs};

pub fn is_installed(app_id: &str) -> bool {
    let id = format!("{app_id}.desktop");
    let Some(path) = data_dirs()
        .into_iter()
        .map(|dir| dir.join("applications").join(&id))
        .find(|path| path.is_file())
    else {
        return false;
    };
    fs::read_to_string(&path).is_ok_and(|text| {
        DesktopEntry::parse_handler(&id, &path, &text, None, &current_desktops()).is_some()
    })
}

pub struct InstallWatch {
    watch: FolderWatch,
    folders: Vec<PathBuf>,
    changes: Inbox<()>,
}

impl InstallWatch {
    pub fn start() -> io::Result<Self> {
        let folders: Vec<PathBuf> = data_dirs()
            .into_iter()
            .map(|dir| dir.join("applications"))
            .collect();
        let changes = Inbox::new()?;
        let changed = changes.outbox();
        let wanted = folders.clone();
        let watch = FolderWatch::spawn(Changes::Any, move |paths| {
            if paths.iter().any(|path| touches(&wanted, path)) {
                changed.send(());
            }
        })?;
        let installs = Self {
            watch,
            folders,
            changes,
        };
        installs.follow();
        Ok(installs)
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.changes.fd()
    }

    pub fn take_changed(&mut self) -> bool {
        let changed = !self.changes.drain().is_empty();
        if changed {
            self.follow();
        }
        changed
    }

    fn follow(&self) {
        for folder in &self.folders {
            if let Some(nearest) = folder.ancestors().find(|ancestor| ancestor.is_dir())
                && let Err(error) = self.watch.watch(nearest)
            {
                tracing::debug!("cannot follow {}: {error}", nearest.display());
            }
        }
    }
}

fn touches(folders: &[PathBuf], path: &Path) -> bool {
    folders
        .iter()
        .any(|folder| path.parent() == Some(folder.as_path()) || folder.starts_with(path))
}
