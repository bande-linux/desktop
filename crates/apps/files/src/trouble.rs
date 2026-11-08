use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use bande_l10n::{text, tr};

use crate::browser::Location;
use crate::prompt::{Kind, Prompt};
use crate::sidebar::system_disk_name;
use crate::tabs::location_title;
use crate::view::{FilesView, Sheet};
use bande_fs::{Action, Answer, Failure, Problem, Purpose, Resolution, Shortfall, format_size};
use bande_toolkit::ui::Alert;

fn failure_title(name: &str, purpose: Purpose) -> String {
    match purpose {
        Purpose::Do(Action::Copy) => tr!("“{name}” couldn’t be copied.", name = name),
        Purpose::Do(Action::Move) => tr!("“{name}” couldn’t be moved.", name = name),
        Purpose::Do(Action::Duplicate) => tr!("“{name}” couldn’t be duplicated.", name = name),
        Purpose::Do(Action::Trash) => tr!("“{name}” couldn’t be moved to the Trash.", name = name),
        Purpose::Do(Action::PutBack) => tr!("“{name}” couldn’t be put back.", name = name),
        Purpose::Erase => tr!("“{name}” couldn’t be deleted.", name = name),
        _ => tr!("“{name}” couldn’t be changed.", name = name),
    }
}

fn shortfall_title(purpose: Purpose, item: Option<&str>, volume: &str, size: &str) -> String {
    match (purpose, item) {
        (Purpose::Do(Action::Move), Some(name)) => tr!(
            "“{name}” can’t be moved because “{volume}” needs {size} more space.",
            name = name,
            volume = volume,
            size = size
        ),
        (Purpose::Do(Action::Duplicate), Some(name)) => tr!(
            "“{name}” can’t be duplicated because “{volume}” needs {size} more space.",
            name = name,
            volume = volume,
            size = size
        ),
        (_, Some(name)) => tr!(
            "“{name}” can’t be copied because “{volume}” needs {size} more space.",
            name = name,
            volume = volume,
            size = size
        ),
        (Purpose::Do(Action::Move), None) => tr!(
            "The items can’t be moved because “{volume}” needs {size} more space.",
            volume = volume,
            size = size
        ),
        (Purpose::Do(Action::Duplicate), None) => tr!(
            "The items can’t be duplicated because “{volume}” needs {size} more space.",
            volume = volume,
            size = size
        ),
        (_, None) => tr!(
            "The items can’t be copied because “{volume}” needs {size} more space.",
            volume = volume,
            size = size
        ),
    }
}

fn item_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

pub(crate) fn failure_sentence(failure: &Failure) -> String {
    match failure.problem() {
        Problem::Permission => tr!("You don’t have permission."),
        Problem::ReadOnly => tr!("The volume is read-only."),
        Problem::NoSpace => tr!("The disk is full."),
        Problem::NameTooLong => tr!("The name is too long."),
        Problem::NameInvalid => tr!("The name isn’t allowed on this disk."),
        Problem::TooLarge => tr!("The item is too large for the format of this disk."),
        Problem::DeviceGone => tr!("The disk was disconnected."),
        Problem::ServerGone => tr!(
            "The server stopped responding. Nothing was left under the real name. Try again when it’s back."
        ),
        Problem::DiskError => tr!(
            "The disk couldn’t be read or written. It may have been disconnected or be damaged."
        ),
        Problem::Missing => tr!("The item or its folder no longer exists."),
        Problem::Other => tr!("{error}", error = failure.error.to_string()),
    }
}

fn mount_point(path: &Path) -> PathBuf {
    if let Some(volume) = bande_volumes::connected_volume_of(path) {
        return volume;
    }
    let device = |path: &Path| std::fs::metadata(path).map(|metadata| metadata.dev()).ok();
    let own = device(path);
    path.ancestors()
        .take_while(|ancestor| device(ancestor) == own)
        .last()
        .unwrap_or(path)
        .to_path_buf()
}

fn volume_name(path: &Path) -> String {
    match mount_point(path) {
        root if root == Path::new("/") => system_disk_name(),
        mount => location_title(&Location::Folder(mount)),
    }
}

impl FilesView {
    pub(crate) fn present_failure(&mut self) {
        let Some((_, running)) = self.operations.failure() else {
            return;
        };
        let Some(failure) = &running.failure else {
            return;
        };
        if failure.no_trash {
            let (name, several) = (item_name(&failure.path), running.items > 1);
            self.present_no_trash(&name, several);
            return;
        }
        let title = failure_title(&item_name(&failure.path), running.purpose);
        let detail = failure_sentence(failure);
        let (several, retryable) = (running.items > 1, failure.retryable());
        let mut prompt =
            Prompt::new(&title, &detail, self.choice()).button(&tr!("Stop"), "stop", Kind::Cancel);
        if several {
            prompt = prompt.button(&tr!("Skip All"), "skip-all", Kind::Other);
        }
        let skip = match retryable {
            true => Kind::Other,
            false => Kind::Primary,
        };
        prompt = prompt.button(&tr!("Skip"), "skip", skip);
        if retryable {
            prompt = prompt.button(&tr!("Try Again"), "retry", Kind::Primary);
        }
        let cancel = prompt.cancel_button();
        self.show_sheet(prompt, Sheet::Failure, cancel);
    }

    fn present_no_trash(&mut self, name: &str, several: bool) {
        let title = tr!(
            "“{name}” can’t be moved to the Trash.",
            name = name.to_owned()
        );
        let mut prompt = Prompt::new(
            &title,
            &tr!("This disk has no Trash you can use. Delete it now? This can’t be undone."),
            self.choice(),
        )
        .button(&tr!("Stop"), "stop", Kind::Cancel);
        if several {
            prompt = prompt
                .button(&tr!("Skip All"), "skip-all", Kind::Other)
                .button(&tr!("Delete All"), "delete-all", Kind::Other);
        }
        prompt = prompt.button(&tr!("Skip"), "skip", Kind::Other).button(
            &tr!("Delete"),
            "delete",
            Kind::Primary,
        );
        let cancel = prompt.cancel_button();
        self.show_sheet(prompt, Sheet::Failure, cancel);
    }

    pub(crate) fn present_busy(&mut self, quitting: bool) {
        let leave = match quitting {
            true => text!("Stop and Quit"),
            false => text!("Stop and Close"),
        };
        let prompt = Prompt::new(
            &tr!("Files is still working."),
            &tr!("Items already finished stay where they are. The item in progress is discarded, and its original is not touched."),
            self.choice(),
        )
        .button(&leave.tr(), "stop", Kind::Other)
        .button(&tr!("Keep Going"), "keep", Kind::Cancel);
        let cancel = prompt.cancel_button();
        self.show_sheet(prompt, Sheet::Busy { quitting }, cancel);
    }

    pub(crate) fn answer_failure(&mut self, action: &str) {
        let (resolution, apply_to_all) = match action {
            "retry" => (Resolution::Retry, false),
            "delete" => (Resolution::Delete, false),
            "delete-all" => (Resolution::Delete, true),
            "skip" => (Resolution::Skip, false),
            "skip-all" => (Resolution::Skip, true),
            _ => (Resolution::Stop, false),
        };
        if let Some((index, _)) = self.operations.failure() {
            self.operations.answer(
                index,
                Answer {
                    resolution,
                    apply_to_all,
                },
            );
        }
    }

    pub(crate) fn present_shortfall(&mut self, purpose: Purpose, shortfall: &Shortfall) {
        let volume = volume_name(&shortfall.destination);
        let size = format_size(shortfall.needed);
        let item = match shortfall.sources.as_slice() {
            [source] => Some(item_name(source)),
            _ => None,
        };
        let title = shortfall_title(purpose, item.as_deref(), &volume, &size);
        let alert = Alert::new(
            &title,
            &tr!("Nothing was written. Make room on the disk and try again."),
            self.choice(),
        )
        .primary(&tr!("OK"), "ok");
        self.show_sheet(alert, Sheet::NoSpace, None);
    }
}
