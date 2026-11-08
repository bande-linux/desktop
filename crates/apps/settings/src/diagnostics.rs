use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use bande_settings_panes::PaneId;
use bande_toolkit::ui::{Axis, Button, Stack, TextField, TextView, Toggle, Widget};
use bande_toolkit::{Spacing, TextRole};
use serde_json::{Value, json};

use crate::L10N;
use crate::failure::pane_failed;
use crate::follow::{Update, Updates, spawn};
use crate::pages::{boxed, muted_row};
use crate::view::{Detail, Intent, SettingsView, intent};
use crate::widgets::{Card, Header, Row, Text, Tone};

const SOCKET: &str = "/run/bande/report.sock";
const ANSWER_PATIENCE: Duration = Duration::from_secs(60);
const SEND_CHECKS: [Duration; 3] = [
    Duration::from_secs(3),
    Duration::from_secs(10),
    Duration::from_secs(30),
];
const REPORT_FONT_SIZE: f32 = 12.0;
const REPORT_SYMBOL: &str = "dialog-warning-symbolic";
const HEADER_SYMBOL: f32 = 32.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub id: String,
    pub kind: String,
    pub program: String,
    pub time: String,
    pub status: String,
    pub occurrences: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub reports: Vec<Report>,
    pub sending: bool,
    pub configured: bool,
    pub open: Option<(String, String)>,
    pub description: String,
    pub saved: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticsCommand {
    Refresh,
    Read(String),
    Delete(String),
    Send(String),
    SetSending(bool),
    Create(String),
    Save(String, String),
}

fn ask(request: &Value) -> io::Result<Value> {
    let mut stream = UnixStream::connect(SOCKET)?;
    stream.set_read_timeout(Some(ANSWER_PATIENCE))?;
    writeln!(stream, "{request}")?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    let answer: Value = serde_json::from_str(&line)?;
    match answer.get("ok") {
        Some(value) => Ok(value.clone()),
        None => Err(io::Error::other(
            answer
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("no answer")
                .to_owned(),
        )),
    }
}

fn text_of(value: &Value, name: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn load() -> io::Result<Diagnostics> {
    let list = ask(&json!({"op": "list"}))?;
    let status = ask(&json!({"op": "status"}))?;
    let mut reports: Vec<Report> = list
        .get("reports")
        .and_then(Value::as_array)
        .map(|reports| {
            reports
                .iter()
                .map(|report| Report {
                    id: text_of(report, "id"),
                    kind: text_of(report, "kind"),
                    program: text_of(report, "program"),
                    time: text_of(report, "time"),
                    status: text_of(report, "status"),
                    occurrences: report
                        .get("occurrences")
                        .and_then(Value::as_u64)
                        .unwrap_or(1),
                })
                .collect()
        })
        .unwrap_or_default();
    reports.sort_by(|a, b| b.time.cmp(&a.time));
    let flag = |name| status.get(name).and_then(Value::as_bool).unwrap_or(false);
    Ok(Diagnostics {
        reports,
        sending: flag("sending"),
        configured: flag("configured"),
        open: None,
        description: String::new(),
        saved: None,
    })
}

fn report_text(id: &str) -> io::Result<String> {
    let report = ask(&json!({"op": "read", "id": id}))?;
    let mut text = String::new();
    if let Some(manifest) = report
        .get("manifest")
        .and_then(Value::as_str)
        .and_then(|manifest| serde_json::from_str::<Value>(manifest).ok())
    {
        text.push_str(&serde_json::to_string_pretty(&manifest)?);
        text.push('\n');
    }
    if let Some(attachments) = report.get("attachments").and_then(Value::as_object) {
        for (name, content) in attachments {
            let _ = writeln!(
                text,
                "\n--- {name}\n{}",
                content.as_str().unwrap_or_default()
            );
        }
    }
    Ok(text)
}

pub fn follow_diagnostics(updates: Updates) -> Sender<DiagnosticsCommand> {
    let (sender, receiver) = mpsc::channel();
    spawn("diagnostics", move || {
        let failed = |error: io::Error| {
            updates.send(pane_failed(
                PaneId::Diagnostics,
                "diagnostics-failed",
                error,
            ));
        };
        for command in receiver {
            let result = match command {
                DiagnosticsCommand::Refresh => Ok(()),
                DiagnosticsCommand::Read(id) => match report_text(&id) {
                    Ok(text) => {
                        updates.send(Update::DiagnosticsReport(id, text));
                        continue;
                    }
                    Err(error) => Err(error),
                },
                DiagnosticsCommand::Delete(id) => ask(&json!({"op": "delete", "id": id})).map(drop),
                DiagnosticsCommand::Send(id) => {
                    let asked = ask(&json!({"op": "send", "id": id})).map(drop);
                    if asked.is_ok() {
                        for wait in SEND_CHECKS {
                            if let Ok(diagnostics) = load() {
                                updates.send(Update::Diagnostics(diagnostics));
                            }
                            std::thread::sleep(wait);
                        }
                    }
                    asked
                }
                DiagnosticsCommand::SetSending(on) => {
                    ask(&json!({"op": "set-sending", "on": on})).map(drop)
                }
                DiagnosticsCommand::Create(description) => {
                    match ask(&json!({"op": "diagnose", "description": description})) {
                        Ok(created) => {
                            let id = text_of(&created, "id");
                            if let Ok(diagnostics) = load() {
                                updates.send(Update::Diagnostics(diagnostics));
                            }
                            if let Ok(text) = report_text(&id) {
                                updates.send(Update::DiagnosticsReport(id.clone(), text));
                            }
                            updates.send(Update::ReportCreated(id));
                            continue;
                        }
                        Err(error) => Err(error),
                    }
                }
                DiagnosticsCommand::Save(id, text) => {
                    let home = std::env::var_os("HOME").unwrap_or_default();
                    let path = Path::new(&home).join(format!("bande-report-{id}.txt"));
                    match std::fs::write(&path, text) {
                        Ok(()) => {
                            updates.send(Update::ReportSaved(path.display().to_string()));
                            continue;
                        }
                        Err(error) => Err(error),
                    }
                }
            };
            if let Err(error) = result {
                failed(error);
            }
            match load() {
                Ok(diagnostics) => {
                    updates.send(Update::Diagnostics(diagnostics));
                }
                Err(error) => tracing::debug!("cannot list the reports: {error}"),
            }
        }
    });
    sender
}

fn kind_label(kind: &str) -> String {
    L10N.tr(match kind {
        "crash" | "panic" => "fault-crash",
        "not-responding" | "app-hang" | "force-quit" => "fault-not-responding",
        "compositor-stall" | "shell-stall" => "fault-desktop-stall",
        "failed-unit" => "fault-failed-service",
        "kernel-panic" | "kernel-oops" | "kernel-warning" | "lockup" | "hung-task" => {
            "fault-kernel"
        }
        "unexpected-restart" => "fault-unexpected-restart",
        "oom" => "fault-memory",
        "gpu-reset" | "gpu-error" => "fault-graphics",
        "display-failure" | "display-error" => "fault-display",
        "sleep-failed" | "wake-failure" => "fault-sleep",
        "storage-error" => "fault-storage",
        "mount-stall" => "fault-network-drive",
        "manual" => "fault-manual",
        _ => "fault-other",
    })
}

fn status_label(status: &str) -> String {
    L10N.tr(match status {
        "sent" => "report-sent",
        "waiting" => "report-waiting",
        "rejected" => "report-rejected",
        _ => "report-local",
    })
}

impl SettingsView {
    pub(crate) fn diagnostics_rows(&mut self) -> Vec<Box<dyn Widget>> {
        let mut rows: Vec<Box<dyn Widget>> = Vec::new();
        let configured = self.diagnostics.configured;
        let set = intent(&self.intents, Intent::SetSendingReports);
        let toggle = Toggle::new(
            &L10N.tr("reports-send-automatically"),
            self.diagnostics.sending,
            set,
        );
        rows.push(boxed(toggle));
        rows.push(muted_row(&L10N.tr(if self.diagnostics.sending {
            "reports-state-sending"
        } else if configured {
            "reports-state-local"
        } else {
            "reports-state-no-server"
        })));
        if self.diagnostics.reports.is_empty() {
            rows.push(muted_row(&L10N.tr("no-reports")));
        }
        for report in self.diagnostics.reports.clone() {
            let mut open = intent(&self.intents, Intent::OpenReport);
            let id = report.id.clone();
            let title = L10N.tr_args(
                "report-title",
                [
                    ("program", report.program.clone().into()),
                    ("kind", kind_label(&report.kind).into()),
                ],
            );
            let detail = format!("{} · {}", report.time, status_label(&report.status));
            let row = Row::new(
                Text::new(&title, Tone::Primary),
                Some(boxed(bande_toolkit::ui::Label::new("›"))),
            )
            .detail(Text::new(&detail, Tone::Muted).caption())
            .on_activate(move || open(id.clone()));
            rows.push(self.item_row(format!("report:{}", report.id), row));
        }
        rows
    }

    pub(crate) fn send_report_rows(&mut self) -> Vec<Box<dyn Widget>> {
        let mut draft = intent(&self.intents, Intent::DraftReportDescription);
        let description = TextField::new(
            &L10N.tr("report-description"),
            &self.diagnostics.description,
            move |text| draft(text.to_owned()),
        );
        let mut create = intent(&self.intents, Intent::CreateReport);
        vec![
            muted_row(&L10N.tr("report-intro")),
            boxed(description),
            boxed(Button::new(&L10N.tr("report-create"), move || create(()))),
        ]
    }

    pub(crate) fn report_page(&mut self) -> Option<Vec<Box<dyn Widget>>> {
        let Some(Detail::Report(id)) = self.detail.clone() else {
            return None;
        };
        let report = self
            .diagnostics
            .reports
            .iter()
            .find(|report| report.id == id)?
            .clone();
        let text = self
            .diagnostics
            .open
            .as_ref()
            .filter(|(open, _)| *open == id)
            .map_or_else(String::new, |(_, text)| text.clone());
        let mut page: Vec<Box<dyn Widget>> =
            self.back_button(PaneId::Diagnostics).into_iter().collect();
        let mut copy = intent(&self.intents, Intent::CopyReport);
        let copied = text.clone();
        let mut buttons: Vec<Box<dyn Widget>> =
            vec![boxed(Button::new(&L10N.tr("report-copy"), move || {
                copy(copied.clone())
            }))];
        if self.diagnostics.configured && report.status != "sent" {
            let mut send = intent(&self.intents, Intent::SendReport);
            let target = id.clone();
            buttons.push(boxed(Button::new(&L10N.tr("report-send-now"), move || {
                send(target.clone());
            })));
        }
        if !self.diagnostics.configured {
            let mut save = intent(&self.intents, Intent::SaveReport);
            let target = (id.clone(), text.clone());
            buttons.push(boxed(Button::new(&L10N.tr("report-save"), move || {
                save(target.clone());
            })));
        }
        let mut delete = intent(&self.intents, Intent::DeleteReport);
        let target = id.clone();
        buttons.push(boxed(Button::new(&L10N.tr("report-delete"), move || {
            delete(target.clone());
        })));
        let ink = self.palette().ink;
        let symbol = self.symbol(REPORT_SYMBOL, HEADER_SYMBOL, ink);
        page.push(boxed(Header::new(
            symbol,
            Text::new(&report.program, Tone::Primary).role(TextRole::Title),
            &format!(
                "{} · {} · {} · {}{}",
                kind_label(&report.kind),
                report.time,
                status_label(&report.status),
                L10N.tr_args("report-id", [("id", report.id.clone().into())]),
                self.diagnostics
                    .saved
                    .as_ref()
                    .map_or_else(String::new, |path| {
                        format!(
                            " · {}",
                            L10N.tr_args("report-saved", [("path", path.clone().into())])
                        )
                    }),
            ),
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
