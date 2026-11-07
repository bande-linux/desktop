use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};

use bande_settings_panes::PaneId;
use bande_toolkit::ui::{Axis, Button, Stack, TextView, Widget};
use bande_toolkit::{Spacing, TextRole};
use bande_watch::{Changes, FolderWatch};

use crate::L10N;
use crate::failure::pane_failed;
use crate::follow::{Update, Updates, spawn};
use crate::pages::{boxed, muted_row};
use crate::view::{Detail, Intent, SettingsView, intent};
use crate::widgets::{Card, Header, Row, Text, Tone};

const REPORTS: &str = "/var/lib/bande/crash-reports";
const SYSTEM_FOLDER: &str = "system";
const HEADER_LINES: usize = 4;
const REPORT_FONT_SIZE: f32 = 12.0;
const REPORT_SYMBOL: &str = "dialog-warning-symbolic";
const HEADER_SYMBOL: f32 = 32.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReport {
    pub path: PathBuf,
    pub kind: String,
    pub program: String,
    pub time: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrashReportCommand {
    Refresh,
    Delete(PathBuf),
}

fn read_header(path: &Path) -> Option<CrashReport> {
    let lines = BufReader::new(File::open(path).ok()?).lines();
    let (mut kind, mut program, mut time) = (None, None, None);
    for line in lines.take(HEADER_LINES).map_while(Result::ok) {
        match line.split_once(": ") {
            Some(("Kind", value)) => kind = Some(value.to_owned()),
            Some(("Program", value)) => program = Some(value.to_owned()),
            Some(("Time", value)) => time = Some(value.to_owned()),
            _ => {}
        }
    }
    Some(CrashReport {
        path: path.to_path_buf(),
        kind: kind?,
        program: program?,
        time: time?,
    })
}

fn own_folder() -> PathBuf {
    Path::new(REPORTS).join(rustix::process::getuid().as_raw().to_string())
}

fn list_reports() -> Vec<CrashReport> {
    let mut reports: Vec<CrashReport> = [own_folder(), Path::new(REPORTS).join(SYSTEM_FOLDER)]
        .iter()
        .filter_map(|folder| fs::read_dir(folder).ok())
        .flatten()
        .flatten()
        .filter_map(|entry| read_header(&entry.path()))
        .collect();
    reports.sort_by(|a, b| b.path.file_name().cmp(&a.path.file_name()));
    reports
}

pub fn follow_crash_reports(updates: Updates) -> Sender<CrashReportCommand> {
    let (sender, receiver) = mpsc::channel();
    spawn("crash-reports", move || {
        let changed = updates.clone();
        let watch = FolderWatch::spawn(Changes::Any, move |_| {
            changed.send(Update::CrashReportsChanged);
        });
        let watch = match watch {
            Ok(watch) => Some(watch),
            Err(error) => {
                tracing::warn!("cannot watch the crash reports: {error}");
                None
            }
        };
        let watch_folders = || {
            for folder in [
                PathBuf::from(REPORTS),
                own_folder(),
                Path::new(REPORTS).join(SYSTEM_FOLDER),
            ] {
                if let Some(watch) = &watch
                    && let Err(error) = watch.watch(&folder)
                {
                    tracing::debug!("cannot watch {}: {error}", folder.display());
                }
            }
        };
        watch_folders();
        updates.send(Update::CrashReports(list_reports()));
        for command in receiver {
            match command {
                CrashReportCommand::Refresh => watch_folders(),
                CrashReportCommand::Delete(path) => {
                    if let Err(error) = fs::remove_file(&path) {
                        updates.send(pane_failed(
                            PaneId::CrashReports,
                            "crash-report-delete-failed",
                            error,
                        ));
                    }
                }
            }
            updates.send(Update::CrashReports(list_reports()));
        }
    });
    sender
}

fn kind_label(kind: &str) -> String {
    L10N.tr(match kind {
        "not-responding" => "crash-kind-not-responding",
        "failed" => "crash-kind-failed",
        "kernel-panic" => "crash-kind-kernel-panic",
        "unexpected-restart" => "crash-kind-unexpected-restart",
        _ => "crash-kind-crash",
    })
}

impl SettingsView {
    pub(crate) fn crash_report_rows(&mut self) -> Vec<Box<dyn Widget>> {
        if self.crash_reports.is_empty() {
            return vec![muted_row(&L10N.tr("no-crash-reports"))];
        }
        let mut rows = Vec::new();
        for report in self.crash_reports.clone() {
            let mut open = intent(&self.intents, Intent::OpenCrashReport);
            let path = report.path.clone();
            let title = L10N.tr_args(
                "crash-report-title",
                [
                    ("program", report.program.clone().into()),
                    ("kind", kind_label(&report.kind).into()),
                ],
            );
            let row = Row::new(
                Text::new(&title, Tone::Primary),
                Some(boxed(bande_toolkit::ui::Label::new("›"))),
            )
            .detail(Text::new(&report.time, Tone::Muted).caption())
            .on_activate(move || open(path.clone()));
            rows.push(self.item_row(format!("crash:{}", report.path.display()), row));
        }
        rows
    }

    pub(crate) fn crash_report_page(&mut self) -> Option<Vec<Box<dyn Widget>>> {
        let Some(Detail::CrashReport(path)) = self.detail.clone() else {
            return None;
        };
        let text = fs::read_to_string(&path).ok()?;
        let report = read_header(&path)?;
        let mut page: Vec<Box<dyn Widget>> =
            self.back_button(PaneId::CrashReports).into_iter().collect();
        let mut copy = intent(&self.intents, Intent::CopyCrashReport);
        let copied = text.clone();
        let mut buttons: Vec<Box<dyn Widget>> = vec![boxed(Button::new(
            &L10N.tr("crash-report-copy"),
            move || copy(copied.clone()),
        ))];
        if path.starts_with(own_folder()) {
            let mut delete = intent(&self.intents, Intent::DeleteCrashReport);
            let target = path.clone();
            buttons.push(boxed(Button::new(
                &L10N.tr("crash-report-delete"),
                move || delete(target.clone()),
            )));
        }
        let ink = self.palette().ink;
        let symbol = self.symbol(REPORT_SYMBOL, HEADER_SYMBOL, ink);
        page.push(boxed(Header::new(
            symbol,
            Text::new(&report.program, Tone::Primary).role(TextRole::Title),
            &format!("{} · {}", kind_label(&report.kind), report.time),
            Some(boxed(Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                buttons,
            ))),
        )));
        let mut view = TextView::document(&text).read_only().borderless();
        view.set_font(true, REPORT_FONT_SIZE);
        page.push(boxed(Card::new(vec![boxed(view)])));
        Some(page)
    }
}
