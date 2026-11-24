use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use bande_datetime::{DateFormats, local_zone};
use bande_l10n::{text, tr};
use jiff::Timestamp;

use crate::failure::pane_failed;
use crate::follow::{Update, Updates, spawn};
use crate::forget::Forget;
use crate::pages::{boxed, muted_row};
use crate::view::{Anchor, Detail, Intent, SettingsView, intent};
use crate::widgets::{Card, Header, Row, Text, Tone};
use bande_settings_panes::{PaneId, SettingId};
use bande_toolkit::ui::{Axis, Button, Spinner, Stack, TextView, Toggle, Widget};
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
pub(crate) const DESCRIPTION: &str = "report-description";
const REVIEW_REPORT: &str = "report-review";
const SEND_REPORT: &str = "report-send";
const DESCRIPTION_LIMIT: usize = 8192;
const DESCRIPTION_LINES: u16 = 4;
const WAITING: &str = "waiting";
const SENT: &str = "sent";

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
    pub enrolled: bool,
    pub forgetting: bool,
    pub privacy: Option<String>,
    pub checkin: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum After {
    Review,
    Send,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Progress {
    #[default]
    Idle,
    Preparing,
    Failed,
    Sending(String),
}

#[derive(Debug, Default)]
pub(crate) struct ReportForm {
    pub(crate) description: String,
    pub(crate) progress: Progress,
    pub(crate) open: Option<(String, String)>,
    pub(crate) saved: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticsCommand {
    Refresh,
    Read(String),
    Delete(String),
    Send(String),
    SetSending(bool),
    ForgetSent,
    Create(String, After),
    OpenPrivacy(String),
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

fn follow_sending(updates: &Updates) {
    let updates = updates.clone();
    spawn("report-sending", move || {
        for wait in SEND_CHECKS {
            std::thread::sleep(wait);
            match load() {
                Ok(diagnostics) => {
                    if !updates.send(Update::Diagnostics(diagnostics)) {
                        return;
                    }
                }
                Err(error) => tracing::debug!("cannot list the reports: {error}"),
            }
        }
    });
}

fn create(description: &str, after: After) -> io::Result<String> {
    let send = after == After::Send;
    let description: String = description.chars().take(DESCRIPTION_LIMIT).collect();
    let created = ask(&json!({"op": "diagnose", "description": description, "send": send}))?;
    Ok(text_of(&created, "id"))
}

fn publish_reports(updates: &Updates) {
    match load() {
        Ok(diagnostics) => {
            updates.send(Update::Diagnostics(diagnostics));
        }
        Err(error) => tracing::debug!("cannot list the reports: {error}"),
    }
}

pub fn follow_diagnostics(updates: Updates) -> Sender<DiagnosticsCommand> {
    let (sender, receiver) = mpsc::channel();
    spawn("diagnostics", move || {
        let failed = |error: io::Error| {
            updates.send(pane_failed(
                PaneId::Diagnostics,
                text!("Couldn’t do that. Try again."),
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
                    ask(&json!({"op": "send", "id": id})).map(|_| follow_sending(&updates))
                }
                DiagnosticsCommand::ForgetSent => {
                    ask(&json!({"op": "forget-sent"})).map(|_| follow_sending(&updates))
                }
                DiagnosticsCommand::SetSending(on) => {
                    ask(&json!({"op": "set-sending", "on": on})).map(drop)
                }
                DiagnosticsCommand::Create(description, after) => {
                    match create(&description, after) {
                        Ok(id) => {
                            publish_reports(&updates);
                            updates.send(Update::ReportCreated(id, after));
                            if after == After::Send {
                                follow_sending(&updates);
                            }
                        }
                        Err(error) => {
                            tracing::warn!("cannot make a report: {error}");
                            updates.send(Update::ReportFailed);
                        }
                    }
                    continue;
                }
                DiagnosticsCommand::OpenPrivacy(address) => {
                    if let Err(error) = bande_xdg::open(&address, None) {
                        failed(io::Error::other(error));
                    }
                    continue;
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
            publish_reports(&updates);
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
        "oom" => tr!("Out of memory"),
        "gpu-reset" => tr!("Graphics problem"),
        "display-failure" => tr!("Display problem"),
        "wake-failure" => tr!("Wake problem"),
        "mount-stall" => tr!("Network drive stalled"),
        "manual" => tr!("Report by hand"),
        _ => tr!("Problem"),
    }
}

fn status_label(status: &str) -> String {
    match status {
        SENT => tr!("Sent"),
        WAITING => tr!("Waiting to send"),
        "rejected" => tr!("Not accepted"),
        _ => tr!("On this computer"),
    }
}

fn title_of(report: &Report) -> String {
    if report.program.is_empty() {
        return kind_label(&report.kind);
    }
    tr!(
        "{program}: {kind}",
        program = report.program.clone(),
        kind = kind_label(&report.kind)
    )
}

fn detail_of(report: &Report) -> String {
    let status = status_label(&report.status);
    match report.time.parse::<Timestamp>() {
        Ok(time) => {
            let time = DateFormats::for_language().date_time(&time.to_zoned(local_zone()), true);
            format!("{time} · {status}")
        }
        Err(_) => status,
    }
}

impl SettingsView {
    pub(crate) fn report_sending_rows(&mut self) -> Vec<Box<dyn Widget>> {
        let set = intent(&self.intents, Intent::SetSendingReports);
        let label = tr!("Send reports automatically");
        let toggle = Toggle::new(&label, self.diagnostics.sending, set);
        self.anchors
            .push((Anchor::Setting(SettingId::ReportSending, None), toggle.id()));
        let mut rows = vec![boxed(
            Row::new(Text::new(&label, Tone::Primary), Some(boxed(toggle))).detail(
                Text::new(
                    &tr!("Names, addresses and file names are removed first"),
                    Tone::Muted,
                )
                .caption(),
            ),
        )];
        if let Some(checkin) = self.diagnostics.checkin.clone() {
            rows.push(boxed(
                Row::new(Text::new(&tr!("Also sent once a day"), Tone::Primary), None)
                    .detail(Text::new(&checkin, Tone::Muted).caption()),
            ));
        }
        if let Some(privacy) = self.diagnostics.privacy.clone() {
            let mut open = intent(&self.intents, Intent::OpenPrivacy);
            let row = Row::new(
                Text::new(&tr!("Privacy policy"), Tone::Primary),
                Some(boxed(bande_toolkit::ui::Label::new("›"))),
            )
            .detail(Text::new(&privacy, Tone::Muted).caption())
            .on_activate(move || open(privacy.clone()));
            rows.push(self.item_row("report-privacy".to_owned(), row));
        }
        rows.extend(self.sent_reports_row());
        rows
    }

    pub(crate) fn report_rows(&mut self) -> Vec<Box<dyn Widget>> {
        let mut rows: Vec<Box<dyn Widget>> = Vec::new();
        if self.diagnostics.reports.is_empty() {
            rows.push(muted_row(&tr!("Nothing has gone wrong")));
        }
        for report in self.diagnostics.reports.clone() {
            let mut open = intent(&self.intents, Intent::OpenReport);
            let id = report.id.clone();
            let row = Row::new(
                Text::new(&title_of(&report), Tone::Primary),
                Some(boxed(bande_toolkit::ui::Label::new("›"))),
            )
            .detail(Text::new(&detail_of(&report), Tone::Muted).caption())
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

    fn report_progress(&self) -> Text {
        let (message, tone) = match &self.report_form.progress {
            Progress::Idle => (
                tr!("Includes system logs. You can review it before sending."),
                Tone::Muted,
            ),
            Progress::Preparing => (tr!("Preparing the report…"), Tone::Muted),
            Progress::Failed => (tr!("Couldn’t prepare the report. Try again."), Tone::Danger),
            Progress::Sending(id) => {
                let report = self
                    .diagnostics
                    .reports
                    .iter()
                    .find(|report| report.id == *id);
                match report.map(|report| report.status.as_str()) {
                    None => (
                        tr!("Includes system logs. You can review it before sending."),
                        Tone::Muted,
                    ),
                    Some(SENT) => (tr!("Report sent. Thank you."), Tone::Muted),
                    Some(WAITING) => (
                        tr!("The report is sent when the server can be reached."),
                        Tone::Muted,
                    ),
                    Some(_) => (tr!("The report wasn’t sent."), Tone::Danger),
                }
            }
        };
        Text::new(&message, tone).caption()
    }

    pub(crate) fn send_report_rows(&mut self) -> Vec<Box<dyn Widget>> {
        let mut draft = intent(&self.intents, Intent::DraftReportDescription);
        let preparing = self.report_form.progress == Progress::Preparing;
        let mut description = TextView::new(
            &tr!("What happened? (optional)"),
            &self.report_form.description,
            move |text| draft(text.to_owned()),
        )
        .visible_lines(DESCRIPTION_LINES);
        description.set_read_only(preparing);
        self.anchors.extend([
            (Anchor::Item(DESCRIPTION.to_owned()), description.id()),
            (
                Anchor::Setting(SettingId::SendReport, None),
                description.id(),
            ),
        ]);
        let mut review = intent(&self.intents, |()| Intent::CreateReport(After::Review));
        let mut review = Button::new(&tr!("Review…"), move || review(()));
        review.set_enabled(!preparing);
        let mut send = intent(&self.intents, |()| Intent::CreateReport(After::Send));
        let mut send = Button::new(&tr!("Send"), move || send(()));
        send.set_enabled(!preparing);
        self.anchors.extend([
            (Anchor::Item(REVIEW_REPORT.to_owned()), review.id()),
            (Anchor::Item(SEND_REPORT.to_owned()), send.id()),
        ]);
        let mut controls: Vec<Box<dyn Widget>> = vec![boxed(review), boxed(send)];
        if preparing {
            controls.insert(0, boxed(Spinner::new(&tr!("Preparing the report…"))));
        }
        vec![
            boxed(Stack::new(
                Axis::Vertical,
                0.0,
                Spacing::M as f32,
                vec![boxed(description)],
            )),
            boxed(Row::new(
                self.report_progress(),
                Some(boxed(Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    controls,
                ))),
            )),
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
            .report_form
            .open
            .as_ref()
            .filter(|(open, _)| *open == id)
            .map_or_else(String::new, |(_, text)| text.clone());
        let mut page: Vec<Box<dyn Widget>> =
            self.back_button(PaneId::Diagnostics).into_iter().collect();
        let ink = self.palette().ink;
        let symbol = self.symbol(REPORT_SYMBOL, HEADER_SYMBOL, ink);
        page.push(boxed(Header::new(
            symbol,
            Text::new(&title_of(&report), Tone::Primary).role(TextRole::Title),
            &detail_of(&report),
            None,
        )));
        let mut delete = intent(&self.intents, Intent::DeleteReport);
        let target = id.clone();
        let mut copy = intent(&self.intents, Intent::CopyReport);
        let copied = text.clone();
        let mut save = intent(&self.intents, Intent::SaveReport);
        let saved = (id.clone(), text.clone());
        let mut copy = Button::new(&tr!("Copy"), move || copy(copied.clone()));
        copy.set_enabled(!text.is_empty());
        let mut save = Button::new(&tr!("Save"), move || save(saved.clone()));
        save.set_enabled(!text.is_empty());
        let mut buttons: Vec<Box<dyn Widget>> = vec![
            boxed(Button::new(&tr!("Delete"), move || delete(target.clone()))),
            boxed(copy),
            boxed(save),
        ];
        if report.status != SENT {
            let mut send = intent(&self.intents, Intent::SendReport);
            let target = id.clone();
            buttons.push(boxed(Button::new(&tr!("Send"), move || {
                send(target.clone());
            })));
        }
        let saved = match &self.report_form.saved {
            Some(path) => tr!("Saved as {path}", path = path.clone()),
            None => tr!("Report {id}", id = report.id.clone()),
        };
        page.push(boxed(Card::new(vec![boxed(Row::new(
            Text::new(&saved, Tone::Muted).caption(),
            Some(boxed(Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                buttons,
            ))),
        ))])));
        let mut view = TextView::document(&text).read_only().borderless();
        view.set_font(true, REPORT_FONT_SIZE);
        page.push(boxed(Card::new(vec![boxed(view)])));
        Some(page)
    }
}
