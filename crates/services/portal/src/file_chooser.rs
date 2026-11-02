use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use bande_xdg::file_uri;
use zbus::Connection;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::request::{self, CANCELLED, Cancellation, OTHER, SUCCESS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Glob(String),
    MimeType(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub patterns: Vec<Pattern>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub options: Vec<(String, String)>,
    pub initial: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Open {
        multiple: bool,
        directory: bool,
    },
    Save {
        name: Option<String>,
        file: Option<PathBuf>,
    },
    SaveMany {
        names: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog {
    pub app_id: String,
    pub parent_window: String,
    pub title: String,
    pub accept_label: Option<String>,
    pub modal: bool,
    pub mode: Mode,
    pub filters: Vec<Filter>,
    pub current_filter: Option<Filter>,
    pub choices: Vec<Choice>,
    pub folder: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    pub paths: Vec<PathBuf>,
    pub choices: Vec<(String, String)>,
    pub filter: Option<Filter>,
    pub writable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Chosen(Selection),
    Cancelled,
    Unavailable,
}

pub trait FileChooserUi: Send + Sync {
    fn choose(&self, dialog: &Dialog, cancellation: &Cancellation) -> Outcome;
}

type RawFilter = (String, Vec<(u32, String)>);
type RawChoice = (String, String, Vec<(String, String)>, String);
type Options = HashMap<String, OwnedValue>;
type Results = HashMap<String, OwnedValue>;

fn filter((name, patterns): RawFilter) -> Filter {
    Filter {
        name,
        patterns: patterns
            .into_iter()
            .map(|(kind, pattern)| match kind {
                1 => Pattern::MimeType(pattern),
                _ => Pattern::Glob(pattern),
            })
            .collect(),
    }
}

fn raw_filter(filter: &Filter) -> RawFilter {
    (
        filter.name.clone(),
        filter
            .patterns
            .iter()
            .map(|pattern| match pattern {
                Pattern::Glob(glob) => (0, glob.clone()),
                Pattern::MimeType(mime) => (1, mime.clone()),
            })
            .collect(),
    )
}

fn path(bytes: Vec<u8>) -> PathBuf {
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(&bytes);
    PathBuf::from(OsStr::from_bytes(bytes))
}

fn is_plain_name(name: &Path) -> bool {
    let mut components = name.components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    ) && !name.as_os_str().as_bytes().contains(&b'/')
}

pub(crate) fn option<T: TryFrom<OwnedValue>>(options: &Options, key: &str) -> Option<T> {
    options
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| T::try_from(value).ok())
}

pub(crate) fn choices(options: &Options) -> Vec<Choice> {
    option::<Vec<RawChoice>>(options, "choices")
        .unwrap_or_default()
        .into_iter()
        .map(|(id, label, options, initial)| Choice {
            id,
            label,
            options,
            initial,
        })
        .collect()
}

fn dialog(
    app_id: String,
    parent_window: String,
    title: String,
    options: &Options,
    mode: Mode,
) -> Dialog {
    Dialog {
        app_id,
        parent_window,
        title,
        accept_label: option(options, "accept_label"),
        modal: option(options, "modal").unwrap_or(true),
        mode,
        filters: option::<Vec<RawFilter>>(options, "filters")
            .unwrap_or_default()
            .into_iter()
            .map(filter)
            .collect(),
        current_filter: option::<RawFilter>(options, "current_filter").map(filter),
        choices: choices(options),
        folder: option::<Vec<u8>>(options, "current_folder").map(path),
    }
}

fn results(dialog: &Dialog, selection: Selection) -> Results {
    let paths: Vec<PathBuf> = match &dialog.mode {
        Mode::SaveMany { names } => selection
            .paths
            .first()
            .map(|folder| names.iter().map(|name| folder.join(name)).collect())
            .unwrap_or_default(),
        _ => selection.paths,
    };
    let owned = |value: Value<'_>| OwnedValue::try_from(value).ok();
    let uris: Vec<String> = paths.iter().map(|path| file_uri(path)).collect();
    let mut results = Results::new();
    let entries = [
        ("uris", owned(Value::from(uris))),
        (
            "choices",
            (!selection.choices.is_empty())
                .then(|| owned(Value::from(selection.choices)))
                .flatten(),
        ),
        (
            "current_filter",
            selection
                .filter
                .as_ref()
                .and_then(|filter| owned(Value::from(raw_filter(filter)))),
        ),
        (
            "writable",
            matches!(dialog.mode, Mode::Open { .. })
                .then(|| owned(Value::from(selection.writable)))
                .flatten(),
        ),
    ];
    for (key, value) in entries {
        if let Some(value) = value {
            results.insert(key.to_owned(), value);
        }
    }
    results
}

struct Call {
    app_id: String,
    parent_window: String,
    title: String,
    options: Options,
}

pub struct FileChooser {
    pub ui: Arc<dyn FileChooserUi>,
}

impl FileChooser {
    async fn show(
        &self,
        connection: &Connection,
        handle: ObjectPath<'_>,
        mode: Mode,
        call: Call,
    ) -> (u32, Results) {
        let dialog = dialog(
            call.app_id,
            call.parent_window,
            call.title,
            &call.options,
            mode,
        );
        let ui = Arc::clone(&self.ui);
        let shown = dialog.clone();
        let outcome = request::run(connection.object_server(), &handle, move |cancellation| {
            ui.choose(&shown, cancellation)
        })
        .await;
        match outcome {
            Ok(Some(Outcome::Chosen(selection))) => (SUCCESS, results(&dialog, selection)),
            Ok(Some(Outcome::Cancelled) | None) => (CANCELLED, Results::new()),
            Ok(Some(Outcome::Unavailable)) => {
                tracing::warn!("the Bande file dialog cannot start");
                (OTHER, Results::new())
            }
            Err(_) => (OTHER, Results::new()),
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    async fn open_file(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        title: String,
        options: Options,
        #[zbus(connection)] connection: &Connection,
    ) -> (u32, Results) {
        let mode = Mode::Open {
            multiple: option(&options, "multiple").unwrap_or(false),
            directory: option(&options, "directory").unwrap_or(false),
        };
        let call = Call {
            app_id,
            parent_window,
            title,
            options,
        };
        self.show(connection, handle, mode, call).await
    }

    async fn save_file(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        title: String,
        options: Options,
        #[zbus(connection)] connection: &Connection,
    ) -> (u32, Results) {
        let mode = Mode::Save {
            name: option(&options, "current_name"),
            file: option::<Vec<u8>>(&options, "current_file").map(path),
        };
        let call = Call {
            app_id,
            parent_window,
            title,
            options,
        };
        self.show(connection, handle, mode, call).await
    }

    async fn save_files(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        title: String,
        options: Options,
        #[zbus(connection)] connection: &Connection,
    ) -> (u32, Results) {
        let names: Vec<PathBuf> = option::<Vec<Vec<u8>>>(&options, "files")
            .unwrap_or_default()
            .into_iter()
            .map(path)
            .collect();
        if !names.iter().all(|name| is_plain_name(name)) {
            tracing::warn!("{app_id} asked to save files under names that leave the folder");
            return (OTHER, Results::new());
        }
        let mode = Mode::SaveMany { names };
        let call = Call {
            app_id,
            parent_window,
            title,
            options,
        };
        self.show(connection, handle, mode, call).await
    }
}
