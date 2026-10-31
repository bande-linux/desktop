mod listing;
mod naming;
mod operations;
mod ops;
mod trash;
mod undo;
mod watch;

use std::io;
use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use listing::{
    Entry, Kind, Listing, Sort, SortBy, compare, finder_order, format_date, format_list_date,
    format_size, is_hidden, list, list_date, list_trash, merge, same_volume, sort,
};
pub use naming::{duplicate_name, keep_both_name, untitled_folder};
pub use operations::{Finished, Operations, Purpose, Running};
pub use ops::{
    Answer, Change, Conflict, Failure, Job, Outcome, Progress, RenameError, Report, Resolution,
    Task, archive_path, can_extract, new_document, new_folder, rename,
};
pub use trash::{Trash, TrashDir, TrashItem};
pub use undo::{Action, History, Step};
pub use watch::{FolderChanges, FolderWatch};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-fs.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

fn user_error(kind: io::ErrorKind, message: &str) -> io::Error {
    io::Error::new(kind, L10N.tr(message))
}

fn described(error: io::Error) -> io::Error {
    if error.raw_os_error().is_none() {
        return error;
    }
    let message = match error.kind() {
        io::ErrorKind::NotFound => "error-not-found",
        io::ErrorKind::PermissionDenied => "error-permission-denied",
        io::ErrorKind::AlreadyExists => "error-name-taken",
        io::ErrorKind::StorageFull => "error-storage-full",
        io::ErrorKind::QuotaExceeded => "error-storage-full",
        io::ErrorKind::ReadOnlyFilesystem => "error-read-only",
        io::ErrorKind::DirectoryNotEmpty => "error-folder-not-empty",
        io::ErrorKind::FileTooLarge => "error-too-large",
        io::ErrorKind::ResourceBusy => "error-busy",
        io::ErrorKind::InvalidFilename => "error-invalid-name",
        _ => return error,
    };
    user_error(error.kind(), message)
}
