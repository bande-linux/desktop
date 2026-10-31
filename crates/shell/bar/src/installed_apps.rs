use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::time::Instant;

use unbound_fs::FolderWatch;
use unbound_xdg::{DesktopEntry, Locale, applications, current_desktops, data_dirs, data_home};

pub struct InstalledApps {
    entries: Vec<DesktopEntry>,
    watch: Option<FolderWatch>,
}

impl InstalledApps {
    pub fn load() -> Self {
        let watch = FolderWatch::new()
            .inspect_err(|error| tracing::warn!("cannot follow installed apps: {error}"))
            .ok();
        let mut apps = Self {
            entries: Vec::new(),
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
        let changed = self
            .watch
            .as_mut()
            .is_some_and(|watch| !watch.take_changed(now).is_empty());
        if changed {
            self.reload();
        }
        changed
    }

    fn reload(&mut self) {
        let dirs = data_dirs();
        if let Some(watch) = &mut self.watch {
            let mut folders: Vec<PathBuf> =
                dirs.iter().map(|dir| dir.join("applications")).collect();
            folders.extend(data_home().filter(|home| !home.join("applications").is_dir()));
            watch.watch(&folders);
        }
        self.entries = applications(&dirs, Locale::from_env().as_ref(), &current_desktops());
    }
}
