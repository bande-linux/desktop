use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::thread;

use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};

use crate::access::{AccessUi, Answer, Question};
use crate::account::{AccountUi, Details, Shared};
use crate::app_chooser::{AppChooserUi, Offer, Picked};
use crate::dynamic_launcher::{Confirmed, Launcher, LauncherUi};
use crate::file_chooser::{
    Choice, Dialog, FileChooserUi, Filter, Mode, Outcome, Pattern, Selection,
};
use crate::request::Cancellation;
use crate::wallpaper::WallpaperUi;

pub const DIALOG_PROGRAM: &str = "/usr/libexec/bande-file-chooser";
pub const CHOSEN: u8 = 0;
pub const CANCELLED: u8 = 1;
pub const ACCESS: &str = "--access";
pub const CHOOSE_APP: &str = "--choose-app";
pub const CREATE_LAUNCHER: &str = "--create-launcher";
pub const SHARE_ACCOUNT: &str = "--share-account";

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

type Feed = Box<dyn FnOnce(ChildStdin) + Send>;

impl DialogProcess {
    fn show(
        &self,
        arguments: Vec<OsString>,
        cancellation: &Cancellation,
    ) -> Option<(Option<i32>, Vec<u8>)> {
        self.show_fed(arguments, None, cancellation)
    }

    fn show_fed(
        &self,
        arguments: Vec<OsString>,
        feed: Option<Feed>,
        cancellation: &Cancellation,
    ) -> Option<(Option<i32>, Vec<u8>)> {
        let child = Command::new(&self.program)
            .args(arguments)
            .stdin(match feed {
                Some(_) => Stdio::piped(),
                None => Stdio::null(),
            })
            .stdout(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!("cannot start {}: {error}", self.program.display());
                return None;
            }
        };
        if let (Some(feed), Some(stdin)) = (feed, child.stdin.take()) {
            thread::spawn(move || feed(stdin));
        }
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

fn text(value: OsString) -> String {
    value.to_string_lossy().into_owned()
}

pub fn encode_pairs(pairs: &[(String, String)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (key, value) in pairs {
        for part in [key, value] {
            bytes.extend_from_slice(part.as_bytes());
            bytes.push(0);
        }
    }
    bytes
}

pub fn decode_pairs(bytes: &[u8]) -> Vec<(String, String)> {
    let parts: Vec<String> = bytes
        .split(|byte| *byte == 0)
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect();
    parts
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[key, value]| (key.clone(), value.clone()))
        .collect()
}

pub fn access_arguments(question: &Question) -> Vec<OsString> {
    let mut arguments = vec![
        os(ACCESS),
        os("--app-id"),
        os(&question.app_id),
        os("--title"),
        os(&question.title),
        os("--subtitle"),
        os(&question.subtitle),
        os("--body"),
        os(&question.body),
    ];
    for (flag, value) in [
        ("--deny-label", &question.deny_label),
        ("--grant-label", &question.grant_label),
        ("--icon", &question.icon),
    ] {
        if let Some(value) = value {
            arguments.extend([os(flag), os(value)]);
        }
    }
    for choice in &question.choices {
        arguments.extend([
            os("--choice"),
            os(&choice.id),
            os(&choice.label),
            os(&choice.initial),
        ]);
        for (id, label) in &choice.options {
            arguments.extend([os("--option"), os(id), os(label)]);
        }
    }
    arguments
}

pub fn parse_access(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Question, ArgumentError> {
    let mut question = Question::default();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let flag = text(argument);
        let mut value = || {
            arguments
                .next()
                .map(text)
                .ok_or_else(|| ArgumentError(format!("{flag} needs a value")))
        };
        match flag.as_str() {
            ACCESS => {}
            "--app-id" => question.app_id = value()?,
            "--title" => question.title = value()?,
            "--subtitle" => question.subtitle = value()?,
            "--body" => question.body = value()?,
            "--deny-label" => question.deny_label = Some(value()?),
            "--grant-label" => question.grant_label = Some(value()?),
            "--icon" => question.icon = Some(value()?),
            "--choice" => question.choices.push(Choice {
                id: value()?,
                label: value()?,
                options: Vec::new(),
                initial: value()?,
            }),
            "--option" => {
                let option = (value()?, value()?);
                question
                    .choices
                    .last_mut()
                    .ok_or_else(|| ArgumentError("--option comes before --choice".to_owned()))?
                    .options
                    .push(option);
            }
            _ => return Err(ArgumentError(format!("unknown argument {flag}"))),
        }
    }
    Ok(question)
}

pub fn app_chooser_arguments(offer: &Offer) -> Vec<OsString> {
    let mut arguments = vec![os(CHOOSE_APP), os("--app-id"), os(&offer.app_id)];
    for (flag, value) in [
        ("--last-choice", &offer.last_choice),
        ("--content-type", &offer.content_type),
        ("--location", &offer.location),
    ] {
        if let Some(value) = value {
            arguments.extend([os(flag), os(value)]);
        }
    }
    for choice in &offer.choices {
        arguments.extend([os("--app"), os(choice)]);
    }
    arguments
}

pub fn parse_app_chooser(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Offer, ArgumentError> {
    let mut offer = Offer::default();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let flag = text(argument);
        let mut value = || {
            arguments
                .next()
                .map(text)
                .ok_or_else(|| ArgumentError(format!("{flag} needs a value")))
        };
        match flag.as_str() {
            CHOOSE_APP => {}
            "--app-id" => offer.app_id = value()?,
            "--last-choice" => offer.last_choice = Some(value()?),
            "--content-type" => offer.content_type = Some(value()?),
            "--location" => offer.location = Some(value()?),
            "--app" => offer.choices.push(value()?),
            _ => return Err(ArgumentError(format!("unknown argument {flag}"))),
        }
    }
    Ok(offer)
}

pub fn encode_choices(choices: &[String]) -> Vec<u8> {
    let mut line = choices.join("\0").into_bytes();
    line.push(b'\n');
    line
}

pub fn decode_choices(line: &[u8]) -> Vec<String> {
    line.split(|byte| *byte == 0)
        .filter(|choice| !choice.is_empty())
        .map(|choice| String::from_utf8_lossy(choice).into_owned())
        .collect()
}

impl AccessUi for DialogProcess {
    fn ask(&self, question: &Question, cancellation: &Cancellation) -> Answer {
        match self.show(access_arguments(question), cancellation) {
            Some((Some(code), output)) if code == i32::from(CHOSEN) => {
                Answer::Granted(decode_pairs(&output))
            }
            Some((Some(code), _)) if code == i32::from(CANCELLED) => Answer::Denied,
            Some(_) if cancellation.is_cancelled() => Answer::Denied,
            _ => Answer::Unavailable,
        }
    }
}

impl AppChooserUi for DialogProcess {
    fn choose(
        &self,
        offer: &Offer,
        updates: Receiver<Vec<String>>,
        cancellation: &Cancellation,
    ) -> Picked {
        let feed: Feed = Box::new(move |mut stdin| {
            for choices in updates {
                if stdin
                    .write_all(&encode_choices(&choices))
                    .and_then(|()| stdin.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        match self.show_fed(app_chooser_arguments(offer), Some(feed), cancellation) {
            Some((Some(code), output)) if code == i32::from(CHOSEN) => {
                let mut pairs = decode_pairs(&output).into_iter();
                match pairs.next() {
                    Some((id, token)) if !id.is_empty() => Picked::App {
                        id,
                        activation_token: (!token.is_empty()).then_some(token),
                    },
                    _ => Picked::Unavailable,
                }
            }
            Some((Some(code), _)) if code == i32::from(CANCELLED) => Picked::Cancelled,
            Some(_) if cancellation.is_cancelled() => Picked::Cancelled,
            _ => Picked::Unavailable,
        }
    }
}

pub fn launcher_arguments(launcher: &Launcher) -> Vec<OsString> {
    let mut arguments = vec![
        os(CREATE_LAUNCHER),
        os("--app-id"),
        os(&launcher.app_id),
        os("--name"),
        os(&launcher.name),
    ];
    if let Some(target) = &launcher.target {
        arguments.extend([os("--target"), os(target)]);
    }
    if launcher.webapp {
        arguments.push(os("--webapp"));
    }
    if !launcher.editable_name {
        arguments.push(os("--fixed-name"));
    }
    arguments
}

pub fn parse_launcher(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Launcher, ArgumentError> {
    let mut launcher = Launcher {
        editable_name: true,
        ..Launcher::default()
    };
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let flag = text(argument);
        let mut value = || {
            arguments
                .next()
                .map(text)
                .ok_or_else(|| ArgumentError(format!("{flag} needs a value")))
        };
        match flag.as_str() {
            CREATE_LAUNCHER => {}
            "--app-id" => launcher.app_id = value()?,
            "--name" => launcher.name = value()?,
            "--target" => launcher.target = Some(value()?),
            "--webapp" => launcher.webapp = true,
            "--fixed-name" => launcher.editable_name = false,
            _ => return Err(ArgumentError(format!("unknown argument {flag}"))),
        }
    }
    Ok(launcher)
}

impl LauncherUi for DialogProcess {
    fn confirm(&self, launcher: &Launcher, cancellation: &Cancellation) -> Confirmed {
        let icon = launcher.icon.clone();
        let feed: Feed = Box::new(move |mut stdin| {
            let _ = stdin.write_all(&icon);
        });
        match self.show_fed(launcher_arguments(launcher), Some(feed), cancellation) {
            Some((Some(code), output)) if code == i32::from(CHOSEN) => {
                Confirmed::Named(String::from_utf8_lossy(&output).into_owned())
            }
            Some((Some(code), _)) if code == i32::from(CANCELLED) => Confirmed::Cancelled,
            Some(_) if cancellation.is_cancelled() => Confirmed::Cancelled,
            _ => Confirmed::Unavailable,
        }
    }
}

pub fn account_arguments(details: &Details) -> Vec<OsString> {
    let mut arguments = vec![
        os(SHARE_ACCOUNT),
        os("--app-id"),
        os(&details.app_id),
        os("--login"),
        os(&details.login),
        os("--name"),
        os(&details.name),
    ];
    if let Some(reason) = &details.reason {
        arguments.extend([os("--reason"), os(reason)]);
    }
    if let Some(picture) = &details.picture {
        arguments.extend([os("--picture"), picture.clone().into_os_string()]);
    }
    arguments
}

pub fn parse_account(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Details, ArgumentError> {
    let mut details = Details::default();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let flag = text(argument);
        let mut value = || {
            arguments
                .next()
                .ok_or_else(|| ArgumentError(format!("{flag} needs a value")))
        };
        match flag.as_str() {
            SHARE_ACCOUNT => {}
            "--app-id" => details.app_id = text(value()?),
            "--login" => details.login = text(value()?),
            "--name" => details.name = text(value()?),
            "--reason" => details.reason = Some(text(value()?)),
            "--picture" => details.picture = Some(PathBuf::from(value()?)),
            _ => return Err(ArgumentError(format!("unknown argument {flag}"))),
        }
    }
    Ok(details)
}

impl AccountUi for DialogProcess {
    fn share(&self, details: &Details, cancellation: &Cancellation) -> Shared {
        match self.show(account_arguments(details), cancellation) {
            Some((Some(code), _)) if code == i32::from(CHOSEN) => Shared::Yes,
            Some((Some(code), _)) if code == i32::from(CANCELLED) => Shared::No,
            Some(_) if cancellation.is_cancelled() => Shared::No,
            _ => Shared::Unavailable,
        }
    }
}
