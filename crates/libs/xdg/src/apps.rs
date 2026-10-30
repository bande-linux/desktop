use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::entry::{DesktopEntry, Locale, current_desktops, handlers};
use crate::mime::{DIRECTORY, MimeDatabase, OCTET_STREAM};
use crate::mimeapps::{MimeApps, mimeapps_paths, set_default_app};

const DESKTOP_ENTRY: &str = "application/x-desktop";

pub struct Apps {
    mime: MimeDatabase,
    associations: MimeApps,
    handlers: Vec<DesktopEntry>,
    locale: Option<Locale>,
    lists: Vec<PathBuf>,
    data_dirs: Vec<PathBuf>,
    kinds: HashMap<String, String>,
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
            kinds: HashMap::new(),
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

    pub fn kind(&mut self, mime: &str) -> String {
        if let Some(kind) = self.kinds.get(mime) {
            return kind.clone();
        }
        let kind = self
            .mime
            .description(mime, self.locale.as_ref())
            .unwrap_or_else(|| mime.to_owned());
        self.kinds.insert(mime.to_owned(), kind.clone());
        kind
    }

    pub fn kind_for_name(&mut self, name: &str, is_folder: bool) -> String {
        let mime = if is_folder {
            DIRECTORY.to_owned()
        } else {
            self.mime.for_name(name).unwrap_or(OCTET_STREAM).to_owned()
        };
        self.kind(&mime)
    }

    pub fn default_for(&self, mime: &str) -> Option<&DesktopEntry> {
        self.associations
            .default_for(&self.mime.ancestors(mime), &self.handlers)
    }

    pub fn apps_for(&self, mime: &str) -> Vec<&DesktopEntry> {
        self.associations
            .apps_for(&self.mime.ancestors(mime), &self.handlers)
    }

    pub fn default_for_path(&self, path: &Path) -> Option<&DesktopEntry> {
        self.default_for(&self.mime_for(path))
    }

    pub fn apps_for_path(&self, path: &Path) -> Vec<&DesktopEntry> {
        self.apps_for(&self.mime_for(path))
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
