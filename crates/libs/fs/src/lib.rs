mod batch;
mod listing;
mod naming;
mod operations;
mod ops;
mod permissions;
mod trash;
mod undo;
mod watch;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use batch::{
    BatchItem, BatchProblem, Numbering, RenameRule, new_name, plan_renames, rename_all,
};
pub use listing::{
    Entry, Kind, Listing, Recency, Sort, SortBy, compare, finder_order, format_date,
    format_list_date, format_size, is_hidden, list, list_date, list_trash, merge, same_volume,
    sort,
};
pub use naming::{duplicate_name, keep_both_name, typed_location, untitled_folder};
pub use operations::{Finished, Operations, Purpose, Running};
pub use ops::{
    Answer, Change, Conflict, Failure, Job, Outcome, Progress, RenameError, Report, Resolution,
    Task, archive_path, can_extract, new_document, new_folder, rename,
};
pub use permissions::{Access, Class, access, runnable, set_mode, with_access, with_runnable};
pub use trash::{Trash, TrashDir, TrashItem};
pub use undo::{Action, History, Step};
pub use watch::{FolderChanges, FolderWatch};

const ENGLISH: &str = include_str!("../locales/en-US/bande-fs.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
