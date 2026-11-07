use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bande_backup::client::{
    Browsing, CannotBrowse, VERSIONS_ROOT, in_version, live_path, version_times,
};
use bande_datetime::{DateFormats, local_zone};
use bande_fs::{Action, Purpose, Task};
use bande_toolkit::ui::{BUTTON_HEIGHT, Button, WidgetId};
use bande_toolkit::{Item, Point, Rect, TextAlign, TextRole, TextStyle};

use crate::L10N;
use crate::browser::Location;
use crate::controls::{Signal, sender};
use crate::view::{FilesView, Request, home};

pub const BACKUP_BAR_HEIGHT: f32 = 36.0;
const BUTTON_WIDTH: f32 = 84.0;
const BUTTON_GAP: f32 = 8.0;
const BAR_INSET: f32 = 12.0;

pub(crate) struct BackupBar {
    _browsing: Browsing,
    older: WidgetId,
    newer: WidgetId,
    restore: WidgetId,
    done: WidgetId,
}

fn fingerprint(path: &Path) -> Option<(u64, Option<SystemTime>)> {
    let metadata = path.symlink_metadata().ok()?;
    Some((metadata.len(), metadata.modified().ok()))
}

fn shown_time(time: &str) -> String {
    let formats = DateFormats::from_env();
    let at = time
        .parse::<jiff::civil::DateTime>()
        .ok()
        .and_then(|at| at.to_zoned(local_zone()).ok());
    match at {
        Some(at) => format!("{}, {}", formats.date(&at, true), formats.clock(&at)),
        None => time.to_owned(),
    }
}

impl FilesView {
    pub(crate) fn in_backup(&self) -> bool {
        self.browser
            .current_folder()
            .is_some_and(|folder| folder.starts_with(VERSIONS_ROOT))
    }

    pub(crate) fn browse_backups(&mut self) {
        if self.backups.is_none() {
            self.requests.push(Request::OpenBackups);
        }
    }

    pub fn backups_opened(&mut self, outcome: Result<Browsing, CannotBrowse>) {
        let browsing = match outcome {
            Ok(browsing) => browsing,
            Err(reason) => {
                let text = match reason {
                    CannotBrowse::NotInstalled | CannotBrowse::NoDisk => "backups-not-set-up",
                    CannotBrowse::DiskAway => "backups-disk-away",
                    CannotBrowse::Failed => "backups-cannot-open",
                };
                self.show_notice(L10N.tr(text));
                return;
            }
        };
        let followed = self
            .browser
            .selected()
            .filter(|entry| !entry.is_folder())
            .map(|entry| entry.path.clone());
        let folder = self.browser.current_folder().unwrap_or_else(home);
        let times = version_times();
        let Some(time) = times
            .iter()
            .rev()
            .find(|time| in_version(time, &folder).is_dir())
        else {
            self.show_notice(L10N.tr("backups-none"));
            return;
        };
        let target = in_version(time, &folder);
        let button = |view: &Self, title: &str, signal: Signal| {
            let send = sender(&view.signals);
            Button::new(&L10N.tr(title), move || send(signal.clone()))
        };
        let older = button(self, "backups-older", Signal::BackupStep(true));
        let newer = button(self, "backups-newer", Signal::BackupStep(false));
        let restore = button(self, "backups-restore", Signal::BackupRestore).primary();
        let done = button(self, "backups-done", Signal::BackupDone);
        self.backups = Some(BackupBar {
            _browsing: browsing,
            older: self.insert(older),
            newer: self.insert(newer),
            restore: self.insert(restore),
            done: self.insert(done),
        });
        self.go(Location::Folder(target.clone()));
        if let Some(file) = followed
            && let Some(name) = file.file_name()
        {
            self.browser.reveal(&target.join(name));
            self.selection_changed();
        }
    }

    pub fn step_backup(&mut self, older: bool) {
        let Some(folder) = self.browser.current_folder() else {
            return;
        };
        let Some((time, live_folder)) = live_path(&folder) else {
            return;
        };
        let file = self
            .browser
            .selected()
            .filter(|entry| !entry.is_folder())
            .map(|entry| entry.path.clone());
        let times = version_times();
        let Some(here) = times.iter().position(|known| *known == time) else {
            return;
        };
        let candidates: Vec<&String> = match older {
            true => times[..here].iter().rev().collect(),
            false => times[here + 1..].iter().collect(),
        };
        let found = match &file {
            Some(file) => {
                let current = fingerprint(file);
                let live_file = live_path(file).map(|(_, live)| live);
                live_file.and_then(|live| {
                    candidates
                        .iter()
                        .find(|time| {
                            let shown = fingerprint(&in_version(time, &live));
                            shown.is_some() && shown != current
                        })
                        .map(|time| in_version(time, &live))
                })
            }
            None => candidates
                .iter()
                .map(|time| in_version(time, &live_folder))
                .find(|path| path.is_dir()),
        };
        let Some(found) = found else {
            self.show_notice(L10N.tr(if older {
                "backups-no-older"
            } else {
                "backups-no-newer"
            }));
            return;
        };
        if file.is_some() {
            let parent = found.parent().map(Path::to_path_buf).unwrap_or_default();
            self.go(Location::Folder(parent));
            self.browser.reveal(&found);
            self.selection_changed();
        } else {
            self.go(Location::Folder(found));
        }
    }

    pub(crate) fn restore_from_backup(&mut self) {
        let Some(folder) = self.browser.current_folder() else {
            return;
        };
        let Some((_, live_folder)) = live_path(&folder) else {
            return;
        };
        let mut sources: Vec<PathBuf> = self.browser.selected_paths();
        let destination = if sources.is_empty() {
            sources.push(folder);
            live_folder
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or(live_folder)
        } else {
            live_folder
        };
        if let Err(error) = std::fs::create_dir_all(&destination) {
            tracing::warn!("cannot restore into {}: {error}", destination.display());
            self.show_notice(L10N.tr("backups-cannot-restore"));
            return;
        }
        self.start(
            Task::Copy {
                sources,
                destination,
            },
            Purpose::Do(Action::Copy),
        );
    }

    pub(crate) fn leave_backups(&mut self) {
        let live = self
            .browser
            .current_folder()
            .and_then(|folder| live_path(&folder))
            .map(|(_, live)| live);
        if let Some(live) = live {
            let existing = live
                .ancestors()
                .find(|path| path.is_dir())
                .map(Path::to_path_buf);
            self.go(Location::Folder(existing.unwrap_or_else(home)));
        }
        self.close_backup_bar();
    }

    pub(crate) fn notice_left_backups(&mut self) {
        if self.backups.is_some() && !self.in_backup() {
            self.close_backup_bar();
        }
    }

    fn close_backup_bar(&mut self) {
        if let Some(bar) = self.backups.take() {
            for id in [bar.older, bar.newer, bar.restore, bar.done] {
                self.remove(id);
            }
        }
    }

    pub(crate) fn backup_bar_height(&self) -> f32 {
        if self.backups.is_some() {
            BACKUP_BAR_HEIGHT
        } else {
            0.0
        }
    }

    pub(crate) fn backup_bar_frames(&self) -> Vec<(WidgetId, Rect)> {
        let Some(bar) = &self.backups else {
            return Vec::new();
        };
        let area = self.backup_bar_area();
        let y = area.y + ((BACKUP_BAR_HEIGHT - BUTTON_HEIGHT) / 2.0).round();
        let right = area.x + area.width - BAR_INSET;
        let step = BUTTON_WIDTH + BUTTON_GAP;
        [bar.done, bar.restore, bar.newer, bar.older]
            .into_iter()
            .enumerate()
            .map(|(index, id)| {
                (
                    id,
                    Rect::new(
                        right - BUTTON_WIDTH - step * index as f32,
                        y,
                        BUTTON_WIDTH,
                        BUTTON_HEIGHT,
                    ),
                )
            })
            .collect()
    }

    fn backup_bar_area(&self) -> Rect {
        let content = self.content_frame();
        let search = self.search_bar_height();
        Rect::new(
            content.x,
            content.y - search - BACKUP_BAR_HEIGHT,
            content.width,
            BACKUP_BAR_HEIGHT,
        )
    }

    pub(crate) fn paint_backup_bar(&mut self, items: &mut Vec<Item>) {
        if self.backups.is_none() {
            return;
        }
        let Some((time, _)) = self
            .browser
            .current_folder()
            .and_then(|folder| live_path(&folder))
        else {
            return;
        };
        let area = self.backup_bar_area();
        let title = L10N.tr_args("backups-from", [("when", shown_time(&time).into())]);
        let ink = self.palette.ink;
        let label = self.text().paragraph(
            &title,
            TextStyle::role(TextRole::Callout, ink),
            None,
            TextAlign::Start,
        );
        items.push(Item::Text {
            origin: Point::new(
                area.x + BAR_INSET,
                area.y + ((BACKUP_BAR_HEIGHT - label.height()) / 2.0).round(),
            ),
            paragraph: label,
        });
    }
}

pub fn backups_installed() -> bool {
    Path::new("/usr/libexec/bande-backupd").exists()
}
