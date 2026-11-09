use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use bande_l10n::{text, tr};

use crate::failure::pane_failed;
use crate::follow::{Update, Updates, spawn};
use crate::forget::Forget;
use crate::pages::{boxed, muted_row};
use crate::view::{Anchor, Detail, Intent, SettingsView, intent};
use crate::widgets::{Card, Header, Row, Text, Tone};
use bande_settings_panes::PaneId;
use bande_toolkit::ui::{Axis, Button, Stack, TextField, TextView, Toggle, Widget};
use bande_toolkit::{Spacing, TextRole};
use serde_json::{Value, json};

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
pub(crate) const SENT_REPORTS: &str = "sent-reports";

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
    pub enrolled: bool,
    pub forgetting: bool,
    pub privacy: Option<String>,
    pub checkin: Option<String>,
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
    ForgetSent,
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
        enrolled: flag("enrolled"),
        forgetting: flag("forgetting"),
        privacy: status
            .get("privacy")
            .and_then(Value::as_str)
            .map(str::to_owned),
        checkin: status
            .get("checkin")
            .and_then(Value::as_object)
            .map(|checkin| {
                checkin
                    .iter()
                    .map(|(name, value)| match value {
                        Value::String(text) => format!("{name}: {text}"),
                        other => format!("{name}: {other}"),
                    })
                    .collect::<Vec<_>>()
                    .join(" · ")
            }),
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

fn follow_sending(asked: &io::Result<()>, updates: &Updates) {
    if asked.is_err() {
        return;
    }
    for wait in SEND_CHECKS {
        if let Ok(diagnostics) = load() {
            updates.send(Update::Diagnostics(diagnostics));
        }
        std::thread::sleep(wait);
    }
}

pub fn follow_diagnostics(updates: Updates) -> Sender<DiagnosticsCommand> {
    let (sender, receiver) = mpsc::channel();
    spawn("diagnostics", move || {
        let failed = |error: io::Error| {
            updates.send(pane_failed(
                PaneId::Diagnostics,
                text!("That could not be done."),
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
                    follow_sending(&asked, &updates);
                    asked
                }
                DiagnosticsCommand::ForgetSent => {
                    let asked = ask(&json!({"op": "forget-sent"})).map(drop);
                    follow_sending(&asked, &updates);
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
    match kind {
        "crash" | "panic" => tr!("Crash"),
        "not-responding" | "app-hang" | "force-quit" => {
            tr!("Not responding")
        }
        "compositor-stall" | "shell-stall" => tr!("Desktop stalled"),
        "failed-unit" => tr!("Failed to run"),
        "kernel-panic" | "kernel-oops" | "kernel-warning" | "lockup" | "hung-task" => {
            tr!("Kernel problem")
        }
        "unexpected-restart" => tr!("Unexpected restart"),
        "oom" => tr!("Out of memory"),
        "gpu-reset" | "gpu-error" => tr!("Graphics problem"),
        "display-failure" | "display-error" => tr!("Display problem"),
        "sleep-failed" | "wake-failure" => tr!("Sleep or wake problem"),
        "storage-error" => tr!("Disk problem"),
        "mount-stall" => tr!("Network drive stalled"),
        "manual" => tr!("Report by hand"),
        _ => tr!("Problem"),
    }
}

fn status_label(status: &str) -> String {
    match status {
        "sent" => tr!("Sent"),
        "waiting" => tr!("Waiting to send"),
        "rejected" => tr!("Refused by the server"),
        _ => tr!("On this computer"),
    }
}

impl SettingsView {
    pub(crate) fn diagnostics_rows(&mut self) -> Vec<Box<dyn Widget>> {
        let mut rows: Vec<Box<dyn Widget>> = Vec::new();
        let configured = self.diagnostics.configured;
        let set = intent(&self.intents, Intent::SetSendingReports);
        let toggle = Toggle::new(
            &tr!("Send reports automatically"),
            self.diagnostics.sending,
            set,
        );
        rows.push(boxed(toggle));
        rows.push(muted_row(&if self.diagnostics.sending {
            tr!("New reports are sent to Bande's report server. Names, addresses and file names are removed first.")
        } else if configured {
            tr!("Reports stay on this computer unless you send one.")
        } else {
            tr!("Reports stay on this computer. No server is configured.")
        }));
        if let Some(checkin) = self.diagnostics.checkin.clone() {
            rows.push(muted_row(&tr!(
                "Once a day it also sends how long the computer was used, and nothing else: {checkin}",
                checkin = checkin
            )));
        }
        if let Some(privacy) = self.diagnostics.privacy.clone() {
            rows.push(muted_row(&tr!(
                "What the server keeps, and for how long: {address}",
                address = privacy
            )));
        }
        rows.extend(self.sent_reports_row());
        if self.diagnostics.reports.is_empty() {
            rows.push(muted_row(&tr!("Nothing has gone wrong")));
        }
        for report in self.diagnostics.reports.clone() {
            let mut open = intent(&self.intents, Intent::OpenReport);
            let id = report.id.clone();
            let title = tr!(
                "{program}: {kind}",
                program = report.program.clone(),
                kind = kind_label(&report.kind)
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

    fn sent_reports_row(&mut self) -> Option<Box<dyn Widget>> {
        if self.forgetting == Some(Forget::SentReports) {
            return Some(self.confirm_row(
                &tr!("Delete the reports sent from this computer?"),
                &tr!("The server erases them and stops knowing this computer. Reports sent later start afresh."),
                &tr!("Delete"),
            ));
        }
        if self.diagnostics.forgetting {
            return Some(muted_row(&tr!(
                "The reports sent from this computer are deleted from the server as soon as it can be reached."
            )));
        }
        if !self.diagnostics.enrolled {
            return None;
        }
        let mut ask = intent(&self.intents, |()| {
            Intent::AskForget(Some(Forget::SentReports))
        });
        let button = Button::new(&tr!("Delete…"), move || ask(()));
        self.anchors
            .push((Anchor::Item(SENT_REPORTS.to_owned()), button.id()));
        Some(boxed(
            Row::new(
                Text::new(
                    &tr!("Delete reports sent from this computer"),
                    Tone::Primary,
                ),
                Some(boxed(button)),
            )
            .detail(
                Text::new(
                    &tr!("Erases everything the server keeps about this computer"),
                    Tone::Muted,
                )
                .caption(),
            ),
        ))
    }

    pub(crate) fn send_report_rows(&mut self) -> Vec<Box<dyn Widget>> {
        let mut draft = intent(&self.intents, Intent::DraftReportDescription);
        let description = TextField::new(
            &tr!("What happened? (optional)"),
            &self.diagnostics.description,
            move |text| draft(text.to_owned()),
        );
        let mut create = intent(&self.intents, Intent::CreateReport);
        vec![
            muted_row(&tr!(
                "This gathers the logs of this boot and the one before, including anything left from a freeze or restart. You read it before it goes."
            )),
            boxed(description),
            boxed(Button::new(&tr!("Prepare Report"), move || create(()))),
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
        let mut buttons: Vec<Box<dyn Widget>> = vec![boxed(Button::new(&tr!("Copy"), move || {
            copy(copied.clone())
        }))];
        if self.diagnostics.configured && report.status != "sent" {
            let mut send = intent(&self.intents, Intent::SendReport);
            let target = id.clone();
            buttons.push(boxed(Button::new(&tr!("Send Now"), move || {
                send(target.clone());
            })));
        }
        if !self.diagnostics.configured {
            let mut save = intent(&self.intents, Intent::SaveReport);
            let target = (id.clone(), text.clone());
            buttons.push(boxed(Button::new(&tr!("Save to File"), move || {
                save(target.clone());
            })));
        }
        let mut delete = intent(&self.intents, Intent::DeleteReport);
        let target = id.clone();
        buttons.push(boxed(Button::new(&tr!("Delete"), move || {
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
                tr!("Report {id}", id = report.id.clone()),
                self.diagnostics
                    .saved
                    .as_ref()
                    .map_or_else(String::new, |path| {
                        format!(" · {}", tr!("Saved as {path}", path = path.clone()))
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
