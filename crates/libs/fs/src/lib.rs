mod listing;
mod naming;
mod operations;
mod ops;
mod trash;
mod undo;
mod watch;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use listing::{
    Entry, Kind, Sort, SortBy, finder_order, format_date, format_size, is_hidden, list, list_trash,
    same_volume, sort,
};
pub use naming::{duplicate_name, keep_both_name, untitled_folder};
pub use operations::{Finished, Operations, Purpose, Running};
pub use ops::{
    Answer, Change, Conflict, Failure, Job, Outcome, Progress, RenameError, Report, Resolution,
    Task, archive_path, can_extract, new_document, new_folder, rename,
};
pub use trash::{Trash, TrashDir, TrashItem};
pub use undo::{Action, History, Step};
pub use watch::FolderWatch;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-fs.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("listing.rs"),
            include_str!("naming.rs"),
            include_str!("ops/archive.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
