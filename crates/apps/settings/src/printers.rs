use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};

use bande_l10n::{text, tr, trn};

use crate::follow::{Update, Updates, spawn};
use crate::forget::Forget;
use crate::general::info_row;
use crate::pages::{Widgets, boxed, muted_row};
use crate::services::{SETTLE, signals};
use crate::users::{accounts, members_of};
use crate::view::{Anchor, Detail, Intent, SettingsView, intent};
use crate::widgets::{Caption, Card, HEADER_SYMBOL, Header, Row, Text, Tone};
use bande_packages::{Client, Info, Mode};
use bande_printers::{
    Cups, Job, JobInfo, JobState, Kind, Mechanism, PrintClient, Printer, PrinterInfo, PrinterState,
    SPOOLER, mechanism_available,
};
use bande_settings_panes::PaneId;
use bande_toolkit::Spacing;
use bande_toolkit::TextRole;
use bande_toolkit::ui::{Axis, Button, Label, PopUpButton, Stack, TextField, Widget};
use zbus::blocking::Connection;
use zbus::{MatchRule, message};

pub(crate) const PRINTER_SYMBOL: &str = "printer-symbolic";
const CUPS_PREFIX: &str = "cups:";
const PRINTER_ADMINS: &str = "lpadmin";
const PRINTING_PACKAGES: [&str; 3] = ["cups", "cups-pk-helper", "ipp-usb"];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrintersState {
    pub cups_installed: bool,
    pub may_change: bool,
    pub printers: Vec<PrinterInfo>,
    pub jobs: Vec<JobInfo>,
    pub queues: Vec<Printer>,
    pub queue_jobs: Vec<Job>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrinterCommand {
    Watch(bool),
    SetDefault(String),
    Remove(String),
    Pause(String, bool),
    CancelQueued(i32),
    Cancel(u32),
    AddAddress(String),
    InstallPrinting,
}

fn read(session: &Connection, cups: &Cups, system: &Connection) -> Option<PrintersState> {
    let client = PrintClient::new(session).ok()?;
    let (printers, jobs) = match (client.printers(), client.jobs()) {
        (Ok(printers), Ok(jobs)) => (printers, jobs),
        (Err(error), _) | (_, Err(error)) => {
            tracing::warn!("cannot reach the print service: {error}");
            return None;
        }
    };
    let cups_installed = cups.installed();
    Some(PrintersState {
        cups_installed,
        may_change: administers_printers() || mechanism_available(system),
        printers,
        jobs,
        queues: match cups_installed {
            true => cups.printers().unwrap_or_default(),
            false => Vec::new(),
        },
        queue_jobs: match cups_installed {
            true => cups.jobs().unwrap_or_default(),
            false => Vec::new(),
        },
    })
}

fn install_printing(system: &Connection) -> Result<(), bande_packages::Error> {
    let client = Client::new(system);
    let available: Vec<_> = client
        .resolve(&PRINTING_PACKAGES.map(str::to_owned))?
        .into_iter()
        .filter(|package| package.info == Info::Available)
        .collect();
    let ids: Vec<String> = PRINTING_PACKAGES
        .iter()
        .filter_map(|name| available.iter().find(|package| package.name == *name))
        .map(|package| package.id.clone())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    client.install(&ids, Mode::Apply, &|_| {}).map(drop)
}

fn administers_printers() -> bool {
    let members = members_of(&[PRINTER_ADMINS]);
    rustix::process::getuid().is_root()
        || accounts()
            .into_iter()
            .find(|account| account.you)
            .is_some_and(|you| members.contains(&you.login))
}

fn administer<T>(
    system: &Connection,
    direct: impl FnOnce() -> bande_printers::Result<T>,
    authorized: impl FnOnce(&Mechanism) -> bande_printers::Result<T>,
) -> bande_printers::Result<T> {
    match direct() {
        Err(bande_printers::Error::NotAllowed) if !administers_printers() => {
            Mechanism::new(system).and_then(|mechanism| authorized(&mechanism))
        }
        other => other,
    }
}

fn sentence(error: &bande_printers::Error) -> String {
    tracing::warn!("CUPS: {error}");
    match error {
        bande_printers::Error::NotAllowed => {
            tr!("Only members of the lpadmin group can change printers.")
        }
        bande_printers::Error::Refused(_, message) if !message.is_empty() => message.clone(),
        _ => tr!("Couldn't change this printer."),
    }
}

fn watch_print_service(changed: Sender<()>) -> zbus::Result<Connection> {
    let session = bande_bus::session()?;
    let client = PrintClient::new(&session)?;
    let changes = client.changes()?;
    client.watch()?;
    let _ = changed.send(());
    spawn("printers-service", move || {
        for _ in changes {
            let _ = changed.send(());
        }
    });
    Ok(session)
}

fn stop_watching(watch: &mut Option<Connection>) {
    if let Some(session) = watch.take()
        && let Err(error) = session.close()
    {
        tracing::debug!("cannot close the print service watch: {error}");
    }
}

pub fn follow_printers(
    system: &Connection,
    session: &Connection,
    updates: Updates,
) -> Sender<PrinterCommand> {
    let (sender, receiver) = mpsc::channel();
    let (changed, changes) = mpsc::channel::<()>();
    let reader = updates.clone();
    let reading = system.clone();
    let session = session.clone();
    let commands_session = crate::follow::waiting_session(&session);
    spawn("printers", move || {
        while changes.recv().is_ok() {
            std::thread::sleep(SETTLE);
            changes.try_iter().count();
            if let Some(state) = read(&session, &Cups::new(), &reading)
                && !reader.send(Update::Printers(state))
            {
                break;
            }
        }
    });
    let bus = system.clone();
    spawn("printers-worker", move || {
        let cups = Cups::new();
        let watching = Arc::new(AtomicBool::new(false));
        let mut cups_followed = false;
        let mut watch = None;
        let service = |work: &dyn Fn(&PrintClient<'_>) -> zbus::Result<()>| {
            PrintClient::new(&commands_session)
                .and_then(|client| work(&client))
                .map_err(|error| {
                    tracing::warn!("the print service refused: {error}");
                    tr!("Couldn't change this printer.")
                })
        };
        for command in receiver {
            let result = match command {
                PrinterCommand::Watch(on) => {
                    watching.store(on, Ordering::Relaxed);
                    if on {
                        if !cups_followed
                            && let Ok(rule) = MatchRule::builder()
                                .msg_type(message::Type::Signal)
                                .interface(SPOOLER)
                                .map(|builder| builder.build())
                        {
                            cups_followed = true;
                            let shown = Arc::clone(&watching);
                            signals(
                                &bus,
                                rule,
                                changed.clone(),
                                move |_| shown.load(Ordering::Relaxed).then_some(()),
                                || true,
                            );
                        }
                        if watch.is_none() {
                            watch = watch_print_service(changed.clone())
                                .inspect_err(|error| {
                                    tracing::warn!("cannot follow the print service: {error}");
                                })
                                .ok();
                        }
                    } else {
                        stop_watching(&mut watch);
                    }
                    Ok(())
                }
                PrinterCommand::SetDefault(id) => service(&|client| client.set_default(&id)),
                PrinterCommand::Remove(id) => match id.strip_prefix(CUPS_PREFIX) {
                    Some(name) => administer(
                        &bus,
                        || cups.remove(name),
                        |mechanism| mechanism.remove(name),
                    )
                    .map_err(|error| sentence(&error)),
                    None => service(&|client| client.forget(&id)),
                },
                PrinterCommand::Pause(name, paused) => administer(
                    &bus,
                    || cups.pause(&name, paused),
                    |mechanism| mechanism.pause(&name, paused),
                )
                .map_err(|error| sentence(&error)),
                PrinterCommand::CancelQueued(job) => {
                    administer(&bus, || cups.cancel(job), |mechanism| mechanism.cancel(job))
                        .map_err(|error| sentence(&error))
                }
                PrinterCommand::Cancel(job) => service(&|client| client.cancel(job)),
                PrinterCommand::InstallPrinting => {
                    let installed = install_printing(&bus);
                    updates.send(Update::PrintingInstalled);
                    installed.map_err(|error| {
                        tracing::warn!("cannot install printing: {error}");
                        tr!("Couldn't install printing support.")
                    })
                }
                PrinterCommand::AddAddress(address) => {
                    let added = service(&|client| client.add_address(&address).map(drop));
                    updates.send(Update::PrinterAdded(added.is_ok()));
                    added.map_err(|_| {
                        tr!("Couldn't add “{name}”. Check that it is turned on and on this network.", name = address.clone())
                    })
                }
            };
            if let Err(message) = result {
                updates.send(Update::PaneFailed(PaneId::Printers, message));
            }
            let _ = changed.send(());
        }
    });
    sender
}

fn state_text(printer: &PrinterInfo, queue: Option<&Printer>, jobs: usize) -> String {
    let state = match queue.map(|queue| (queue.state, queue.accepting)) {
        Some((PrinterState::Stopped, _)) => tr!("Paused"),
        Some((PrinterState::Printing, _)) => tr!("Printing"),
        Some((PrinterState::Idle, false)) => tr!("Not accepting documents"),
        Some(_) => tr!("Idle"),
        None if !printer.reachable => tr!("Offline"),
        None if jobs > 0 => tr!("Printing"),
        None => tr!("Idle"),
    };
    let mut parts = vec![state];
    if jobs > 0 {
        parts.push(trn!("1 document", "{count} documents", count = jobs));
    }
    if queue.is_some_and(|queue| {
        queue
            .reasons
            .iter()
            .any(|reason| reason.contains("offline"))
    }) {
        parts.push(tr!("Offline"));
    }
    if printer.default {
        parts.push(tr!("Default"));
    }
    parts.join(" · ")
}

fn job_state(state: JobState) -> String {
    (match state {
        JobState::Waiting => text!("Waiting"),
        JobState::Held => text!("On hold"),
        JobState::Printing => text!("Printing"),
        JobState::Stopped => text!("Stopped"),
    })
    .tr()
}

impl SettingsView {
    fn queue_of(&self, printer: &PrinterInfo) -> Option<Printer> {
        let name = printer.id.strip_prefix(CUPS_PREFIX)?;
        self.printers
            .as_ref()?
            .queues
            .iter()
            .find(|queue| queue.name == name)
            .cloned()
    }

    fn waiting_jobs(&self, printer: &PrinterInfo) -> usize {
        let Some(state) = self.printers.as_ref() else {
            return 0;
        };
        let direct = state.jobs.iter().filter(|job| job.printer == printer.id);
        let queued = printer.id.strip_prefix(CUPS_PREFIX).map_or(0, |name| {
            state
                .queue_jobs
                .iter()
                .filter(|job| job.printer == name)
                .count()
        });
        direct.count() + queued
    }

    pub(crate) fn printers_summary(&self) -> Option<String> {
        let state = self.printers.as_ref()?;
        Some(
            match state.printers.iter().find(|printer| printer.default) {
                _ if state.printers.is_empty() => tr!("No printers"),
                Some(default) => trn!(
                    "“{name}” is the default printer",
                    "{count} printers · “{name}” is the default",
                    count = state.printers.len(),
                    name = default.name.clone()
                ),
                None => trn!(
                    "1 printer",
                    "{count} printers",
                    count = state.printers.len()
                ),
            },
        )
    }

    pub(crate) fn printer_rows(&mut self) -> Widgets {
        let Some(state) = self.printers.clone() else {
            return vec![muted_row(&tr!("Looking for printers on this network…"))];
        };
        if state.printers.is_empty() {
            return vec![muted_row(&tr!(
                "No printers on this network yet. They appear here as soon as they are found."
            ))];
        }
        let mut rows = Vec::new();
        for printer in &state.printers {
            let text = state_text(
                printer,
                self.queue_of(printer).as_ref(),
                self.waiting_jobs(printer),
            );
            let mut open = intent(&self.intents, Intent::OpenPrinter);
            let id = printer.id.clone();
            let row = Row::new(
                Text::new(&printer.name, Tone::Primary),
                Some(boxed(Label::new("›"))),
            )
            .detail(Text::new(&text, Tone::Muted).caption())
            .on_activate(move || open(id.clone()));
            rows.push(self.item_row(format!("printer:{}", printer.id), row));
        }
        let titles: Vec<String> = state
            .printers
            .iter()
            .map(|printer| printer.name.clone())
            .collect();
        let ids: Vec<String> = state
            .printers
            .iter()
            .map(|printer| printer.id.clone())
            .collect();
        let selected = state
            .printers
            .iter()
            .position(|printer| printer.default)
            .unwrap_or(0);
        let mut choose = intent(&self.intents, Intent::DefaultPrinter);
        let menu = PopUpButton::new(&tr!("Default printer"), &titles, selected, move |index| {
            if let Some(id) = ids.get(index) {
                choose(id.clone());
            }
        });
        self.anchors
            .push((Anchor::Item("default-printer".to_owned()), menu.id()));
        rows.push(boxed(Row::new(
            Text::new(&tr!("Default printer"), Tone::Primary),
            Some(boxed(menu)),
        )));
        rows
    }

    pub(crate) fn add_printer_rows(&mut self) -> Widgets {
        let mut address = intent(&self.intents, Intent::PrinterAddress);
        let field = TextField::new(
            &tr!("192.168.1.30 or printer.local"),
            &self.printer_address,
            move |text| address(text.to_owned()),
        )
        .without_text_checking();
        self.anchors
            .push((Anchor::Item("printer-address".to_owned()), field.id()));
        let mut add = intent(&self.intents, |()| Intent::AddPrinterAddress);
        let mut button = Button::new(&tr!("Add"), move || add(()));
        button.set_enabled(!self.adding_printer);
        let detail = (if self.adding_printer {
            text!("Adding…")
        } else {
            text!("For a printer that does not show up: its IP address or host name")
        })
        .tr();
        let mut rows = vec![boxed(
            Row::new(
                Text::new(&tr!("Add by address"), Tone::Primary),
                Some(boxed(Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    vec![boxed(field), boxed(button)],
                ))),
            )
            .detail(Text::new(&detail, Tone::Muted).caption()),
        )];
        if self
            .printers
            .as_ref()
            .is_some_and(|state| !state.cups_installed)
        {
            rows.push(self.install_printing_row());
        }
        rows
    }

    fn install_printing_row(&mut self) -> Box<dyn Widget> {
        let mut install = intent(&self.intents, |()| Intent::InstallPrinting);
        let mut button = Button::new(&tr!("Install"), move || install(()));
        button.set_enabled(!self.installing_printing);
        let detail = (if self.installing_printing {
            text!("Installing…")
        } else {
            text!("Network printers need nothing more. USB printers and printers that need a driver use the CUPS print system, from the Debian archive")
        }).tr();
        self.anchors
            .push((Anchor::Item("install-printing".to_owned()), button.id()));
        boxed(
            Row::new(
                Text::new(&tr!("Support for USB and older printers"), Tone::Primary),
                Some(boxed(button)),
            )
            .detail(Text::new(&detail, Tone::Muted).caption()),
        )
    }

    pub(crate) fn printer_page(&mut self) -> Option<Widgets> {
        let Some(Detail::Printer(id)) = self.detail.clone() else {
            return None;
        };
        let state = self.printers.clone()?;
        let printer = state
            .printers
            .iter()
            .find(|printer| printer.id == id)?
            .clone();
        let queue = self.queue_of(&printer);
        let mut page: Widgets = self.back_button(PaneId::Printers).into_iter().collect();
        let ink = self.palette().ink;
        let symbol = self.symbol(PRINTER_SYMBOL, HEADER_SYMBOL, ink);
        page.push(boxed(Header::new(
            symbol,
            Text::new(&printer.name, Tone::Primary).role(TextRole::Title),
            &state_text(&printer, queue.as_ref(), self.waiting_jobs(&printer)),
            None,
        )));
        let uri = queue
            .as_ref()
            .map_or(String::new(), |queue| queue.uri.clone());
        let mut about = Vec::new();
        for (label, value) in [
            (text!("Kind"), &printer.model),
            (text!("Location"), &printer.location),
            (text!("Address"), &uri),
        ] {
            if !value.is_empty() {
                about.push(info_row(&label.tr(), value));
            }
        }
        if !about.is_empty() {
            page.push(boxed(Card::new(about)));
        }
        page.push(boxed(Caption::new(Text::label(&tr!("Print Queue")))));
        let mut rows = Vec::new();
        for job in state.jobs.iter().filter(|job| job.printer == id) {
            let mut cancel = intent(&self.intents, Intent::CancelJob);
            let job_id = job.id;
            let button = Button::new(&tr!("Cancel"), move || cancel(job_id));
            let detail = match job.message.is_empty() {
                true => (match job.state.as_str() {
                    "sending" | "printing" => text!("Printing"),
                    "stopped" => text!("Stopped"),
                    _ => text!("Waiting"),
                })
                .tr(),
                false => job.message.clone(),
            };
            let row = Row::new(Text::new(&job.title, Tone::Primary), Some(boxed(button)))
                .detail(Text::new(&detail, Tone::Muted).caption());
            rows.push(self.item_row(format!("job:{}", job.id), row));
        }
        let name = id.strip_prefix(CUPS_PREFIX).unwrap_or_default().to_owned();
        for job in state.queue_jobs.iter().filter(|job| job.printer == name) {
            let mut cancel = intent(&self.intents, Intent::CancelQueuedJob);
            let job_id = job.id;
            let button = Button::new(&tr!("Cancel"), move || cancel(job_id));
            let title = if job.name.is_empty() {
                tr!("Document {id}", id = job.id)
            } else {
                job.name.clone()
            };
            let detail = [job_state(job.state), job.user.clone()]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let row = Row::new(Text::new(&title, Tone::Primary), Some(boxed(button)))
                .detail(Text::new(&detail, Tone::Muted).caption());
            rows.push(self.item_row(format!("queued:{}", job.id), row));
        }
        if rows.is_empty() {
            rows.push(muted_row(&tr!("No documents waiting")));
        }
        page.push(boxed(Card::new(rows)));
        let mut actions = Vec::new();
        if let Some(queue) = &queue {
            let paused = queue.state == PrinterState::Stopped;
            let mut pause = intent(&self.intents, Intent::PausePrinter);
            let target = name.clone();
            let pause_button = Button::new(
                &(if paused {
                    text!("Resume")
                } else {
                    text!("Pause")
                })
                .tr(),
                move || pause((target.clone(), !paused)),
            );
            self.anchors
                .push((Anchor::Item("printer-pause".to_owned()), pause_button.id()));
            actions.push(boxed(Row::new(
                Text::new(
                    &(if paused {
                        text!("Printing is paused")
                    } else {
                        text!("Pause printing")
                    })
                    .tr(),
                    Tone::Primary,
                ),
                Some(boxed(pause_button)),
            )));
        }
        if !printer.default {
            let mut make = intent(&self.intents, Intent::DefaultPrinter);
            let target = id.clone();
            let button = Button::new(&tr!("Make Default"), move || make(target.clone()));
            actions.push(boxed(Row::new(
                Text::new(&tr!("Use as the default printer"), Tone::Primary),
                Some(boxed(button)),
            )));
        }
        let removable = printer.kind != Kind::Direct || !printer.reachable;
        let forget = Forget::Printer(id.clone());
        if !removable {
        } else if self.forgetting.as_ref() == Some(&forget) {
            actions.push(self.confirm_row(
                &tr!("Remove “{name}”?", name = printer.name.clone()),
                &tr!("Documents waiting for it are cancelled. You can add it again later."),
                &tr!("Remove"),
            ));
        } else {
            let mut ask = intent(&self.intents, move |()| {
                Intent::AskForget(Some(forget.clone()))
            });
            let button = Button::new(&tr!("Remove…"), move || ask(()));
            self.anchors
                .push((Anchor::Item(format!("printer:{id}")), button.id()));
            actions.push(boxed(Row::new(
                Text::new(&tr!("Remove this printer"), Tone::Primary),
                Some(boxed(button)),
            )));
        }
        if !actions.is_empty() {
            page.push(boxed(Card::new(actions)));
        }
        Some(page)
    }
}
