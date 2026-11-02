use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};

use bande_printers::{Cups, Job, JobState, NearbyPrinter, Printer, PrinterState, SPOOLER, browse};
use bande_settings_panes::PaneId;
use bande_toolkit::Spacing;
use bande_toolkit::TextRole;
use bande_toolkit::ui::{Axis, Button, Label, PopUpButton, Stack, TextField, Widget};
use zbus::blocking::Connection;
use zbus::{MatchRule, message};

use crate::L10N;
use crate::follow::{Update, Updates, spawn};
use crate::forget::Forget;
use crate::general::info_row;
use crate::pages::{Widgets, boxed, muted_row};
use crate::services::{SETTLE, signals};
use crate::view::{Anchor, Detail, Intent, SettingsView, intent};
use crate::widgets::{Caption, Card, HEADER_SYMBOL, Header, Row, Text, Tone};

pub(crate) const PRINTER_SYMBOL: &str = "printer-symbolic";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrintersState {
    pub installed: bool,
    pub printers: Vec<Printer>,
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrinterCommand {
    Watch(bool),
    SetDefault(String),
    Remove(String),
    Pause(String, bool),
    Cancel(i32),
    Add(NearbyPrinter),
}

fn read(cups: &Cups) -> Option<PrintersState> {
    if !cups.installed() {
        return Some(PrintersState::default());
    }
    match (cups.printers(), cups.jobs()) {
        (Ok(printers), Ok(jobs)) => Some(PrintersState {
            installed: true,
            printers,
            jobs,
        }),
        (Err(error), _) | (_, Err(error)) => {
            tracing::warn!("{error}");
            None
        }
    }
}

fn sentence(error: &bande_printers::Error) -> String {
    tracing::warn!("CUPS: {error}");
    match error {
        bande_printers::Error::NotAllowed => L10N.tr("administrator-only"),
        bande_printers::Error::Refused(_, message) if !message.is_empty() => message.clone(),
        _ => L10N.tr("couldnt-change"),
    }
}

pub fn follow_printers(system: &Connection, updates: Updates) -> Sender<PrinterCommand> {
    let (sender, receiver) = mpsc::channel();
    let (changed, changes) = mpsc::channel::<()>();
    let reader = updates.clone();
    spawn("printers", move || {
        while changes.recv().is_ok() {
            std::thread::sleep(SETTLE);
            changes.try_iter().count();
            if let Some(state) = read(&Cups::new())
                && !reader.send(Update::Printers(state))
            {
                break;
            }
        }
    });
    let bus = system.clone();
    spawn("printers-worker", move || {
        let cups = Cups::new();
        let watching = Arc::new(AtomicU64::new(0));
        let mut browse_started = false;
        for command in receiver {
            let result = match command {
                PrinterCommand::Watch(on) => {
                    let generation = watching.fetch_add(1, Ordering::Relaxed) + 1;
                    if on {
                        let current = Arc::clone(&watching);
                        if let Ok(rule) = MatchRule::builder()
                            .msg_type(message::Type::Signal)
                            .interface(SPOOLER)
                            .map(|builder| builder.build())
                        {
                            signals(
                                &bus,
                                rule,
                                changed.clone(),
                                |_| Some(()),
                                move || current.load(Ordering::Relaxed) == generation,
                            );
                        }
                        let _ = changed.send(());
                        if !browse_started {
                            browse_started = true;
                            let (bus, updates) = (bus.clone(), updates.clone());
                            spawn("printers-nearby", move || {
                                let found = browse(&bus, |nearby| {
                                    updates.send(Update::NearbyPrinters(nearby))
                                });
                                if let Err(error) = found {
                                    tracing::warn!("cannot look for nearby printers: {error}");
                                    updates.send(Update::NearbyPrinters(Vec::new()));
                                }
                            });
                        }
                    }
                    Ok(())
                }
                PrinterCommand::SetDefault(name) => {
                    cups.set_default(&name).map_err(|error| sentence(&error))
                }
                PrinterCommand::Remove(name) => {
                    cups.remove(&name).map_err(|error| sentence(&error))
                }
                PrinterCommand::Pause(name, paused) => {
                    cups.pause(&name, paused).map_err(|error| sentence(&error))
                }
                PrinterCommand::Cancel(job) => cups.cancel(job).map_err(|error| sentence(&error)),
                PrinterCommand::Add(printer) => {
                    let added = cups.add(&printer);
                    updates.send(Update::PrinterAdded(printer.uri.clone(), added.is_ok()));
                    added.map(drop).map_err(|error| {
                        tracing::warn!("cannot add {}: {error}", printer.uri);
                        L10N.tr_args("printer-add-failed", [("name", printer.name.into())])
                    })
                }
            };
            if let Err(message) = result {
                updates.send(Update::Failed(None, message));
            }
            let _ = changed.send(());
        }
    });
    sender
}

fn state_text(printer: &Printer, jobs: usize) -> String {
    let state = match printer.state {
        PrinterState::Stopped => L10N.tr("printer-paused"),
        PrinterState::Printing => L10N.tr("printer-printing"),
        PrinterState::Idle if !printer.accepting => L10N.tr("printer-not-accepting"),
        PrinterState::Idle => L10N.tr("printer-idle"),
    };
    let mut parts = vec![state];
    if jobs > 0 {
        parts.push(L10N.tr_args("printer-jobs", [("count", jobs.into())]));
    }
    if printer
        .reasons
        .iter()
        .any(|reason| reason.contains("offline"))
    {
        parts.push(L10N.tr("printer-offline"));
    }
    if printer.default {
        parts.push(L10N.tr("printer-default"));
    }
    parts.join(" · ")
}

fn job_state(state: JobState) -> String {
    L10N.tr(match state {
        JobState::Waiting => "job-waiting",
        JobState::Held => "job-held",
        JobState::Printing => "job-printing",
        JobState::Stopped => "job-stopped",
    })
}

impl SettingsView {
    fn queued(&self, printer: &str) -> Vec<Job> {
        self.printers
            .as_ref()
            .map(|state| {
                state
                    .jobs
                    .iter()
                    .filter(|job| job.printer == printer)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn printers_summary(&self) -> Option<String> {
        let state = self.printers.as_ref().filter(|state| state.installed)?;
        Some(
            match state.printers.iter().find(|printer| printer.default) {
                _ if state.printers.is_empty() => L10N.tr("no-printers"),
                Some(default) => L10N.tr_args(
                    "printers-with-default",
                    [
                        ("count", state.printers.len().into()),
                        ("name", default.title().to_owned().into()),
                    ],
                ),
                None => L10N.tr_args("printer-count", [("count", state.printers.len().into())]),
            },
        )
    }

    pub(crate) fn printer_rows(&mut self) -> Widgets {
        let Some(state) = self.printers.clone() else {
            return Vec::new();
        };
        if state.printers.is_empty() {
            return vec![muted_row(&L10N.tr("no-printers-hint"))];
        }
        let mut rows = Vec::new();
        for printer in &state.printers {
            let jobs = self.queued(&printer.name).len();
            let mut open = intent(&self.intents, Intent::OpenPrinter);
            let name = printer.name.clone();
            let row = Row::new(
                Text::new(printer.title(), Tone::Primary),
                Some(boxed(Label::new("›"))),
            )
            .detail(Text::new(&state_text(printer, jobs), Tone::Muted).caption())
            .on_activate(move || open(name.clone()));
            rows.push(self.item_row(format!("printer:{}", printer.name), row));
        }
        let titles: Vec<String> = state
            .printers
            .iter()
            .map(|printer| printer.title().to_owned())
            .chain([L10N.tr("printer-no-default")])
            .collect();
        let names: Vec<String> = state
            .printers
            .iter()
            .map(|printer| printer.name.clone())
            .collect();
        let selected = state
            .printers
            .iter()
            .position(|printer| printer.default)
            .unwrap_or(names.len());
        let mut choose = intent(&self.intents, Intent::DefaultPrinter);
        let menu = PopUpButton::new(
            &L10N.tr("default-printer"),
            &titles,
            selected,
            move |index| {
                if let Some(name) = names.get(index) {
                    choose(name.clone());
                }
            },
        );
        self.anchors
            .push((Anchor::Item("default-printer".to_owned()), menu.id()));
        rows.push(boxed(Row::new(
            Text::new(&L10N.tr("default-printer"), Tone::Primary),
            Some(boxed(menu)),
        )));
        rows
    }

    pub(crate) fn nearby_printer_rows(&mut self) -> Widgets {
        let added: Vec<String> = self
            .printers
            .as_ref()
            .map(|state| {
                state
                    .printers
                    .iter()
                    .map(|printer| printer.uri.clone())
                    .collect()
            })
            .unwrap_or_default();
        let nearby: Vec<NearbyPrinter> = self
            .nearby_printers
            .iter()
            .flatten()
            .filter(|printer| !added.contains(&printer.uri))
            .cloned()
            .collect();
        let mut rows = Vec::new();
        if nearby.is_empty() {
            rows.push(muted_row(&L10N.tr(match self.nearby_printers {
                None => "looking-for-printers",
                Some(_) => "no-nearby-printers",
            })));
        }
        for printer in nearby {
            let adding = self.adding.as_ref() == Some(&printer.uri);
            let detail = if adding {
                L10N.tr("printer-adding")
            } else {
                [printer.model.as_str(), printer.location.as_str()]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ")
            };
            let mut add = intent(&self.intents, Intent::AddPrinter);
            let target = printer.clone();
            let mut button = Button::new(&L10N.tr("printer-add"), move || add(target.clone()));
            button.set_enabled(self.adding.is_none());
            let mut row = Row::new(Text::new(&printer.name, Tone::Primary), Some(boxed(button)));
            if !detail.is_empty() {
                row = row.detail(Text::new(&detail, Tone::Muted).caption());
            }
            rows.push(self.item_row(format!("nearby:{}", printer.uri), row));
        }
        let mut address = intent(&self.intents, Intent::PrinterAddress);
        let field = TextField::new(
            &L10N.tr("printer-address-placeholder"),
            &self.printer_address,
            move |text| address(text.to_owned()),
        );
        self.anchors
            .push((Anchor::Item("printer-address".to_owned()), field.id()));
        let mut add = intent(&self.intents, |()| Intent::AddPrinterAddress);
        let mut button = Button::new(&L10N.tr("printer-add"), move || add(()));
        button.set_enabled(self.adding.is_none());
        let adding_address = self
            .printer_address_target()
            .is_some_and(|target| self.adding.as_ref() == Some(&target.uri));
        let detail = L10N.tr(if adding_address {
            "printer-adding"
        } else {
            "printer-address-detail"
        });
        rows.push(boxed(
            Row::new(
                Text::new(&L10N.tr("printer-address"), Tone::Primary),
                Some(boxed(Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    vec![boxed(field), boxed(button)],
                ))),
            )
            .detail(Text::new(&detail, Tone::Muted).caption()),
        ));
        rows
    }

    pub(crate) fn printer_page(&mut self) -> Option<Widgets> {
        let Some(Detail::Printer(name)) = self.detail.clone() else {
            return None;
        };
        let printer = self
            .printers
            .as_ref()?
            .printers
            .iter()
            .find(|printer| printer.name == name)?
            .clone();
        let jobs = self.queued(&name);
        let mut page: Widgets = self.back_button(PaneId::Printers).into_iter().collect();
        let ink = self.palette().ink;
        let symbol = self.symbol(PRINTER_SYMBOL, HEADER_SYMBOL, ink);
        page.push(boxed(Header::new(
            symbol,
            Text::new(printer.title(), Tone::Primary).role(TextRole::Title),
            &state_text(&printer, jobs.len()),
            None,
        )));
        let mut about = Vec::new();
        for (label, value) in [
            ("printer-kind", &printer.model),
            ("printer-location", &printer.location),
            ("printer-uri", &printer.uri),
            ("printer-queue", &printer.name),
        ] {
            if !value.is_empty() {
                about.push(info_row(&L10N.tr(label), value));
            }
        }
        page.push(boxed(Card::new(about)));
        page.push(boxed(Caption::new(Text::label(&L10N.tr("print-queue")))));
        let mut queue = Vec::new();
        if jobs.is_empty() {
            queue.push(muted_row(&L10N.tr("no-jobs")));
        }
        for job in jobs {
            let mut cancel = intent(&self.intents, Intent::CancelJob);
            let id = job.id;
            let button = Button::new(&L10N.tr("cancel"), move || cancel(id));
            let title = if job.name.is_empty() {
                L10N.tr_args("job-number", [("id", job.id.into())])
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
            queue.push(self.item_row(format!("job:{}", job.id), row));
        }
        page.push(boxed(Card::new(queue)));
        let paused = printer.state == PrinterState::Stopped;
        let mut pause = intent(&self.intents, Intent::PausePrinter);
        let target = name.clone();
        let pause_button = Button::new(
            &L10N.tr(if paused {
                "printer-resume"
            } else {
                "printer-pause"
            }),
            move || pause((target.clone(), !paused)),
        );
        self.anchors
            .push((Anchor::Item("printer-pause".to_owned()), pause_button.id()));
        let mut actions = vec![boxed(Row::new(
            Text::new(
                &L10N.tr(if paused {
                    "printer-paused-hint"
                } else {
                    "printer-pause-hint"
                }),
                Tone::Primary,
            ),
            Some(boxed(pause_button)),
        ))];
        if !printer.default {
            let mut make = intent(&self.intents, Intent::DefaultPrinter);
            let target = name.clone();
            let button = Button::new(&L10N.tr("printer-make-default"), move || {
                make(target.clone())
            });
            actions.push(boxed(Row::new(
                Text::new(&L10N.tr("printer-default-hint"), Tone::Primary),
                Some(boxed(button)),
            )));
        }
        let forget = Forget::Printer(name.clone());
        if self.forgetting.as_ref() == Some(&forget) {
            actions.push(self.confirm_row(
                &L10N.tr_args(
                    "printer-remove-question",
                    [("name", printer.title().to_owned().into())],
                ),
                &L10N.tr("printer-remove-warning"),
                &L10N.tr("printer-remove"),
            ));
        } else {
            let mut ask = intent(&self.intents, move |()| {
                Intent::AskForget(Some(forget.clone()))
            });
            let button = Button::new(&L10N.tr("printer-remove-ellipsis"), move || ask(()));
            self.anchors
                .push((Anchor::Item(format!("printer:{name}")), button.id()));
            actions.push(boxed(Row::new(
                Text::new(&L10N.tr("printer-remove-hint"), Tone::Primary),
                Some(boxed(button)),
            )));
        }
        page.push(boxed(Card::new(actions)));
        if let Some(message) = &self.message {
            page.push(boxed(Caption::new(Text::new(message, Tone::Muted))));
        }
        Some(page)
    }

    pub(crate) fn printer_address_target(&self) -> Option<NearbyPrinter> {
        let address = self.printer_address.trim();
        if address.is_empty() {
            return None;
        }
        let uri = if address.contains("://") {
            address.to_owned()
        } else {
            format!("ipp://{address}/ipp/print")
        };
        Some(NearbyPrinter {
            name: address.to_owned(),
            model: String::new(),
            location: String::new(),
            uri,
        })
    }
}
