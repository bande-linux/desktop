use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use unbound_fs::{Action, Answer, Failure, Problem, Purpose, Resolution, Shortfall, format_size};
use unbound_toolkit::ui::Alert;

use crate::L10N;
use crate::browser::Location;
use crate::prompt::{Kind, Prompt};
use crate::sidebar::system_disk_name;
use crate::tabs::location_title;
use crate::view::{FilesView, Sheet};

fn action_key(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::Do(Action::Copy) => "copy",
        Purpose::Do(Action::Move) => "move",
        Purpose::Do(Action::Duplicate) => "duplicate",
        Purpose::Do(Action::Trash) => "trash",
        Purpose::Do(Action::PutBack) => "put-back",
        Purpose::Erase => "delete",
        _ => "other",
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
        Problem::Permission => L10N.tr("failure-permission"),
        Problem::ReadOnly => L10N.tr("failure-read-only"),
        Problem::NoSpace => L10N.tr("failure-no-space"),
        Problem::NameTooLong => L10N.tr("failure-name-too-long"),
        Problem::DeviceGone => L10N.tr("failure-device-gone"),
        Problem::Other => L10N.tr_args(
            "failure-other",
            [("error", failure.error.to_string().into())],
        ),
    }
}

fn mount_point(path: &Path) -> PathBuf {
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
        let title = L10N.tr_args(
            "failure-title",
            [
                ("name", item_name(&failure.path).into()),
                ("action", action_key(running.purpose).into()),
            ],
        );
        let detail = failure_sentence(failure);
        let (several, retryable) = (running.items > 1, failure.retryable());
        let mut prompt = Prompt::new(&title, &detail, self.choice())
            .button(&L10N.tr("stop"), "stop", Kind::Cancel);
        if several {
            prompt = prompt.button(&L10N.tr("skip-all"), "skip-all", Kind::Other);
        }
        let skip = match retryable {
            true => Kind::Other,
            false => Kind::Primary,
        };
        prompt = prompt.button(&L10N.tr("skip"), "skip", skip);
        if retryable {
            prompt = prompt.button(&L10N.tr("try-again"), "retry", Kind::Primary);
        }
        let cancel = prompt.cancel_button();
        self.show_sheet(prompt, Sheet::Failure, cancel);
    }

    pub(crate) fn answer_failure(&mut self, action: &str) {
        let (resolution, apply_to_all) = match action {
            "retry" => (Resolution::Retry, false),
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
        let action = action_key(purpose);
        let title = match shortfall.sources.as_slice() {
            [source] => L10N.tr_args(
                "no-space-item",
                [
                    ("name", item_name(source).into()),
                    ("action", action.into()),
                    ("volume", volume.into()),
                    ("size", size.into()),
                ],
            ),
            _ => L10N.tr_args(
                "no-space-items",
                [
                    ("action", action.into()),
                    ("volume", volume.into()),
                    ("size", size.into()),
                ],
            ),
        };
        let alert = Alert::new(&title, &L10N.tr("no-space-detail"), self.choice())
            .primary(&L10N.tr("ok"), "ok");
        self.show_sheet(alert, Sheet::NoSpace, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_only_volume_reads_as_such() {
        let failure = Failure {
            path: PathBuf::from("/media/disk/a.txt"),
            error: std::io::Error::from_raw_os_error(rustix::io::Errno::ROFS.raw_os_error()),
        };
        assert_eq!(failure_sentence(&failure), "The volume is read-only.");
    }

}
