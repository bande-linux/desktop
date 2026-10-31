use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};

use crate::file_chooser::{Dialog, FileChooserUi, Filter, Mode, Outcome, Pattern, Selection};
use crate::request::Cancellation;
use crate::wallpaper::WallpaperUi;

pub const DIALOG_PROGRAM: &str = "/usr/libexec/unbound-file-chooser";
pub const CHOSEN: u8 = 0;
pub const CANCELLED: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgumentError(pub String);

fn os(text: &str) -> OsString {
    OsString::from(text)
}

fn offered_filters(dialog: &Dialog) -> Vec<Filter> {
    let mut filters = dialog.filters.clone();
    if let Some(current) = &dialog.current_filter
        && !filters.contains(current)
    {
        filters.push(current.clone());
    }
    filters
}

pub fn arguments(dialog: &Dialog) -> Vec<OsString> {
    let mut arguments = vec![
        os("--app-id"),
        os(&dialog.app_id),
        os("--title"),
        os(&dialog.title),
    ];
    if let Some(label) = &dialog.accept_label {
        arguments.extend([os("--accept-label"), os(label)]);
    }
    match &dialog.mode {
        Mode::Open {
            multiple,
            directory,
        } => {
            arguments.push(os(if *directory {
                "--open-folder"
            } else {
                "--open"
            }));
            if *multiple {
                arguments.push(os("--multiple"));
            }
        }
        Mode::Save { name, file } => {
            arguments.push(os("--save"));
            if let Some(name) = name {
                arguments.extend([os("--name"), os(name)]);
            }
            if let Some(file) = file {
                arguments.extend([os("--file"), file.clone().into_os_string()]);
            }
        }
        Mode::SaveMany { .. } => arguments.push(os("--save-into")),
    }
    if let Some(folder) = &dialog.folder {
        arguments.extend([os("--folder"), folder.clone().into_os_string()]);
    }
    let filters = offered_filters(dialog);
    for filter in &filters {
        arguments.extend([os("--filter"), os(&filter.name)]);
        for pattern in &filter.patterns {
            arguments.extend(match pattern {
                Pattern::Glob(glob) => [os("--glob"), os(glob)],
                Pattern::MimeType(mime) => [os("--mime"), os(mime)],
            });
        }
    }
    if let Some(index) = dialog
        .current_filter
        .as_ref()
        .and_then(|current| filters.iter().position(|filter| filter == current))
    {
        arguments.extend([os("--current-filter"), os(&index.to_string())]);
    }
    arguments
}

pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Dialog, ArgumentError> {
    let mut dialog = Dialog {
        app_id: String::new(),
        parent_window: String::new(),
        title: String::new(),
        accept_label: None,
        modal: true,
        mode: Mode::Open {
            multiple: false,
            directory: false,
        },
        filters: Vec::new(),
        current_filter: None,
        choices: Vec::new(),
        folder: None,
    };
    let mut current = None;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let flag = argument.to_string_lossy().into_owned();
        let mut value = || {
            arguments
                .next()
                .ok_or_else(|| ArgumentError(format!("{flag} needs a value")))
        };
        let text = |value: OsString| value.to_string_lossy().into_owned();
        match flag.as_str() {
            "--app-id" => dialog.app_id = text(value()?),
            "--title" => dialog.title = text(value()?),
            "--accept-label" => dialog.accept_label = Some(text(value()?)),
            "--open" | "--open-folder" => {
                dialog.mode = Mode::Open {
                    multiple: false,
                    directory: flag == "--open-folder",
                }
            }
            "--multiple" => {
                if let Mode::Open { multiple, .. } = &mut dialog.mode {
                    *multiple = true;
                }
            }
            "--save" => {
                dialog.mode = Mode::Save {
                    name: None,
                    file: None,
                }
            }
            "--save-into" => dialog.mode = Mode::SaveMany { names: Vec::new() },
            "--name" => {
                let given = text(value()?);
                if let Mode::Save { name, .. } = &mut dialog.mode {
                    *name = Some(given);
                }
            }
            "--file" => {
                let given = PathBuf::from(value()?);
                if let Mode::Save { file, .. } = &mut dialog.mode {
                    *file = Some(given);
                }
            }
            "--folder" => dialog.folder = Some(PathBuf::from(value()?)),
            "--filter" => dialog.filters.push(Filter {
                name: text(value()?),
                patterns: Vec::new(),
            }),
            "--glob" | "--mime" => {
                let pattern = text(value()?);
                let filter = dialog
                    .filters
                    .last_mut()
                    .ok_or_else(|| ArgumentError(format!("{flag} comes before --filter")))?;
                filter.patterns.push(match flag.as_str() {
                    "--glob" => Pattern::Glob(pattern),
                    _ => Pattern::MimeType(pattern),
                });
            }
            "--current-filter" => {
                current =
                    Some(text(value()?).parse::<usize>().map_err(|_| {
                        ArgumentError("--current-filter needs an index".to_owned())
                    })?);
            }
            _ => return Err(ArgumentError(format!("unknown argument {flag}"))),
        }
    }
    dialog.current_filter = current.and_then(|index| dialog.filters.get(index).cloned());
    Ok(dialog)
}

pub fn encode(selection: &Selection, dialog: &Dialog) -> Vec<u8> {
    let filter = selection
        .filter
        .as_ref()
        .and_then(|chosen| {
            offered_filters(dialog)
                .iter()
                .position(|filter| filter == chosen)
        })
        .map(|index| index.to_string())
        .unwrap_or_default();
    let mut bytes = filter.into_bytes();
    bytes.push(0);
    for path in &selection.paths {
        bytes.extend_from_slice(path.as_os_str().as_bytes());
        bytes.push(0);
    }
    bytes
}

pub fn decode(bytes: &[u8], dialog: &Dialog) -> Selection {
    let mut records = bytes.split(|byte| *byte == 0);
    let filter = records
        .next()
        .and_then(|index| std::str::from_utf8(index).ok()?.parse::<usize>().ok())
        .and_then(|index| offered_filters(dialog).get(index).cloned());
    Selection {
        paths: records
            .filter(|record| !record.is_empty())
            .map(|record| PathBuf::from(OsStr::from_bytes(record)))
            .collect(),
        choices: dialog
            .choices
            .iter()
            .map(|choice| (choice.id.clone(), choice.initial.clone()))
            .collect(),
        filter,
        writable: true,
    }
}

pub struct DialogProcess {
    pub program: PathBuf,
}

impl DialogProcess {
    fn show(
        &self,
        arguments: Vec<OsString>,
        cancellation: &Cancellation,
    ) -> Option<(Option<i32>, Vec<u8>)> {
        let child = Command::new(&self.program)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!("cannot start {}: {error}", self.program.display());
                return None;
            }
        };
        match pidfd_open(Pid::from_child(&child), PidfdFlags::empty()) {
            Ok(process) => cancellation.on_cancel(move || {
                let _ = pidfd_send_signal(&process, Signal::TERM);
            }),
            Err(error) => tracing::warn!("cannot watch {}: {error}", self.program.display()),
        }
        let mut output = Vec::new();
        if let Some(mut stdout) = child.stdout.take() {
            let _ = stdout.read_to_end(&mut output);
        }
        let status = child.wait().ok().and_then(|status| status.code());
        Some((status.filter(|_| !cancellation.is_cancelled()), output))
    }
}

impl FileChooserUi for DialogProcess {
    fn choose(&self, dialog: &Dialog, cancellation: &Cancellation) -> Outcome {
        match self.show(arguments(dialog), cancellation) {
            None => Outcome::Unavailable,
            Some(_) if cancellation.is_cancelled() => Outcome::Cancelled,
            Some((Some(code), output)) if code == i32::from(CHOSEN) => {
                Outcome::Chosen(decode(&output, dialog))
            }
            Some((Some(code), _)) if code == i32::from(CANCELLED) => Outcome::Cancelled,
            Some(_) => Outcome::Unavailable,
        }
    }
}

pub const SET_WALLPAPER: &str = "--set-wallpaper";

impl WallpaperUi for DialogProcess {
    fn confirm(&self, app_id: &str, picture: &Path, cancellation: &Cancellation) -> bool {
        let arguments = vec![
            os(SET_WALLPAPER),
            picture.as_os_str().to_owned(),
            os("--app-id"),
            os(app_id),
        ];
        matches!(
            self.show(arguments, cancellation),
            Some((Some(code), _)) if code == i32::from(CHOSEN)
        )
    }
}
