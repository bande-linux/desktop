use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::time::Instant;

use bande_fs::FolderWatch;
use bande_xdg::{DesktopEntry, Locale, applications, current_desktops, data_dirs};

pub struct InstalledApps {
    entries: Vec<DesktopEntry>,
    wanted: Vec<PathBuf>,
    watch: Option<FolderWatch>,
}

impl InstalledApps {
    pub fn load() -> Self {
        let watch = FolderWatch::new()
            .inspect_err(|error| tracing::warn!("cannot follow installed apps: {error}"))
            .ok();
        let mut apps = Self {
            entries: Vec::new(),
            wanted: Vec::new(),
            watch,
        };
        apps.reload();
        apps
    }

    pub fn entries(&self) -> &[DesktopEntry] {
        &self.entries
    }

    pub fn fd(&self) -> Option<BorrowedFd<'_>> {
        self.watch.as_ref().map(FolderWatch::fd)
    }

    pub fn read(&mut self) {
        if let Some(watch) = &mut self.watch {
            watch.read();
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.watch.as_ref().and_then(FolderWatch::deadline)
    }

    pub fn reload_if_changed(&mut self, now: Instant) -> bool {
        let wanted = &self.wanted;
        let changed = self.watch.as_mut().is_some_and(|watch| {
            let changes = watch.take_changed(now);
            !changes.folders.is_empty()
                || changes.entries.iter().any(|entry| {
                    wanted
                        .iter()
                        .any(|folder| folder.starts_with(entry) || entry.starts_with(folder))
                })
        });
        if changed {
            self.reload();
        }
        changed
    }

    pub fn reload(&mut self) {
        let dirs = data_dirs();
        self.wanted = dirs.iter().map(|dir| dir.join("applications")).collect();
        if let Some(watch) = &mut self.watch {
            let mut folders: Vec<PathBuf> = self
                .wanted
                .iter()
                .filter_map(|folder| folder.ancestors().find(|ancestor| ancestor.is_dir()))
                .map(Path::to_path_buf)
                .collect();
            folders.dedup();
            watch.watch(&folders);
        }
        self.entries = applications(&dirs, Locale::from_env().as_ref(), &current_desktops());
    }
}
