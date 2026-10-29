use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use unbound_xdg::{
    DesktopEntry, Locale, MimeApps, MimeDatabase, current_desktops, handlers, mimeapps_paths,
    set_default_app,
};

const DESKTOP_ENTRY: &str = "application/x-desktop";

pub struct Apps {
    mime: MimeDatabase,
    associations: MimeApps,
    handlers: Vec<DesktopEntry>,
    locale: Option<Locale>,
    lists: Vec<PathBuf>,
    data_dirs: Vec<PathBuf>,
}

impl Apps {
    pub fn load(data_dirs: Vec<PathBuf>, config_dirs: &[PathBuf]) -> Self {
        let locale = Locale::from_env();
        let desktops = current_desktops();
        let lists = mimeapps_paths(config_dirs, &data_dirs, &desktops);
        Self {
            mime: MimeDatabase::load(&data_dirs),
            associations: MimeApps::load(&lists),
            handlers: handlers(&data_dirs, locale.as_ref(), &desktops),
            locale,
            lists,
            data_dirs,
        }
    }

    pub fn mime_for(&self, path: &Path) -> String {
        self.mime.for_path(path)
    }

    pub fn icon_names(&self, mime: &str) -> Vec<String> {
        self.mime.icon_names(mime)
    }

    pub fn icon_names_for_name(&self, name: &str) -> Vec<String> {
        self.mime
            .for_name(name)
            .map(|mime| self.mime.icon_names(mime))
            .unwrap_or_else(|| vec!["application-x-generic".to_owned()])
    }

    pub fn kind(&self, mime: &str) -> String {
        self.mime
            .description(mime, self.locale.as_ref())
            .unwrap_or_else(|| mime.to_owned())
    }

    pub fn default_for(&self, mime: &str) -> Option<&DesktopEntry> {
        self.associations
            .default_for(&self.mime.ancestors(mime), &self.handlers)
    }

    pub fn apps_for(&self, mime: &str) -> Vec<&DesktopEntry> {
        self.associations
            .apps_for(&self.mime.ancestors(mime), &self.handlers)
    }

    pub fn set_default(&mut self, mime: &str, app: &DesktopEntry) -> io::Result<()> {
        let list = self
            .lists
            .iter()
            .find(|path| path.file_name().is_some_and(|name| name == "mimeapps.list"))
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no configuration folder"))?;
        set_default_app(&list, mime, &app.id)?;
        self.associations = MimeApps::load(&self.lists);
        Ok(())
    }

    pub fn launcher(&self, path: &Path, mime: &str) -> Option<DesktopEntry> {
        if mime != DESKTOP_ENTRY {
            return None;
        }
        let text = fs::read_to_string(path).ok()?;
        let id = path.file_name()?.to_string_lossy().into_owned();
        DesktopEntry::parse_handler(&id, path, &text, self.locale.as_ref(), &current_desktops())
    }

    pub fn data_dirs(&self) -> &[PathBuf] {
        &self.data_dirs
    }
}
