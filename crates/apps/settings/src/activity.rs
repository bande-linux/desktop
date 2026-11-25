use std::sync::mpsc::{self, Sender};

use bande_datetime::local_zone;
use bande_intents::{Client, Manifests, Receipt};
use bande_l10n::tr;
use bande_settings_panes::PaneId;
use bande_toolkit::ui::Button;
use jiff::Timestamp;
use zbus::blocking::Connection;

use crate::follow::{Command, Update, Updates, spawn};
use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityCommand {
    Refresh,
    Undo(String),
}

pub fn follow_activity(
    session: &Connection,
    manifests: impl Fn() -> Manifests + Send + 'static,
    updates: Updates,
) -> Sender<ActivityCommand> {
    let (sender, receiver) = mpsc::channel();
    let session = session.clone();
    spawn("activity", move || {
        let client = Client::new(&session);
        while let Ok(first) = receiver.recv() {
            let installed = manifests();
            for command in std::iter::once(first).chain(receiver.try_iter()) {
                if let ActivityCommand::Undo(receipt) = command
                    && let Err(refusal) = client.undo(&installed, &receipt)
                {
                    tracing::warn!("cannot undo: {refusal}");
                    updates.send(Update::PaneFailed(PaneId::Activity, refusal.message));
                }
            }
            if !updates.send(Update::Activity(client.all_receipts(&installed))) {
                break;
            }
        }
    });
    sender
}

fn anchor(receipt: &Receipt) -> String {
    format!("activity:{}", receipt.receipt)
}

impl SettingsView {
    pub(crate) fn activity_rows(&mut self) -> Widgets {
        let Some(receipts) = self.activity.clone() else {
            return vec![muted_row(&tr!("Looking for what was done…"))];
        };
        if receipts.is_empty() {
            return vec![muted_row(&tr!(
                "Changes made from Search, the command line and other programs appear here, where you can undo them for a day."
            ))];
        }
        let formats = self.date_formats();
        let mut rows = Vec::new();
        for receipt in receipts {
            let when = Timestamp::from_second(receipt.made)
                .map(|made| formats.date_time(&made.to_zoned(local_zone()), true))
                .unwrap_or_default();
            let detail = match receipt.agent.is_empty() {
                true => when,
                false => format!("{} · {when}", receipt.agent),
            };
            let mut undo = intent(&self.intents, Intent::UndoActivity);
            let name = receipt.receipt.clone();
            let button = Button::new(&tr!("Undo"), move || undo(name.clone()))
                .tooltip(&tr!("Undo “{change}”", change = receipt.summary.clone()));
            let row = Row::new(
                Text::new(&receipt.summary, Tone::Primary),
                Some(boxed(button)),
            )
            .detail(Text::new(&detail, Tone::Muted).caption());
            rows.push(self.item_row(anchor(&receipt), row));
        }
        rows
    }

    pub(crate) fn undo_activity(&mut self, receipt: String) {
        let receipts = self.activity.as_deref().unwrap_or_default();
        let next = receipts
            .iter()
            .position(|shown| shown.receipt == receipt)
            .and_then(|at| {
                receipts
                    .get(at + 1)
                    .or_else(|| receipts.get(at.checked_sub(1)?))
            });
        self.focus_next = next.map(|next| Anchor::Item(anchor(next)));
        self.commands
            .push(Command::Activity(ActivityCommand::Undo(receipt)));
    }
}
