use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, TreeId};
use bande_l10n::tr;
use bande_toolkit::ui::{Button, Label, TEXT_FIELD_HEIGHT, TextField, Ui, WidgetId};

use super::panel::{Line, Panel, Width};
use super::{Signal, Signals, sender};
use crate::caldav::Principal;
use crate::invite::UserType;

const WIDTH: f32 = 440.0;
const LABEL_WIDTH: f32 = 70.0;
const ROW: f32 = TEXT_FIELD_HEIGHT;
const SHOWN: usize = 8;

pub struct Sheet {
    pub text: String,
    panel: WidgetId,
    cancel: WidgetId,
    status: WidgetId,
    results: Vec<(usize, WidgetId)>,
    found: Vec<Principal>,
    dismissed: Rc<Cell<bool>>,
}

pub fn kind_title(kind: UserType) -> Option<String> {
    match kind {
        UserType::Room => Some(tr!("room")),
        UserType::Resource => Some(tr!("resource")),
        UserType::Group => Some(tr!("group")),
        UserType::Individual | UserType::Unknown => None,
    }
}

fn title(principal: &Principal) -> String {
    let address = principal.addresses.first().cloned().unwrap_or_default();
    match kind_title(principal.kind) {
        Some(kind) => format!("{} · {kind} · {address}", principal.name),
        None => format!("{} · {address}", principal.name),
    }
}

impl Sheet {
    pub fn build(signals: &Signals) -> (Panel, Self) {
        let mut panel = Panel::new(Some(&tr!("People and Rooms")), WIDTH, LABEL_WIDTH).sheet();
        let send = sender(signals);
        let field = panel.add(TextField::new(&tr!("Name or address"), "", move |text| {
            send(Signal::DirectoryText(text.to_owned()))
        }));
        let send = sender(signals);
        let search =
            panel.add(Button::new(&tr!("Search"), move || send(Signal::DirectorySearch)).primary());
        panel.line(Line::new(
            Some(&tr!("Find:")),
            vec![(field, Width::Fill), (search, Width::Fixed(96.0))],
            ROW,
        ));
        let status = panel.add(
            Label::new(&tr!(
                "Search the people, rooms and resources on your server."
            ))
            .secondary(),
        );
        panel.line(Line::new(None, vec![(status, Width::Fill)], 18.0));
        let results = (0..SHOWN)
            .map(|index| {
                let send = sender(signals);
                let button = panel.add(Button::new("", move || send(Signal::DirectoryPick(index))));
                let line = panel.line(Line::new(None, vec![(button, Width::Fill)], ROW));
                panel.show_line(line, false);
                (line, button)
            })
            .collect::<Vec<_>>();
        let send = sender(signals);
        let cancel =
            panel.add(Button::new(&tr!("Close"), move || send(Signal::DirectoryClose)).cancel());
        panel.line(
            Line::new(None, vec![(cancel, Width::Fixed(96.0))], ROW)
                .separated()
                .trailing(),
        );
        let id = |index: usize| panel.id_of(index).unwrap_or_else(WidgetId::next);
        let sheet = Self {
            text: String::new(),
            panel: bande_toolkit::ui::Widget::id(&panel),
            cancel: id(cancel),
            status: id(status),
            results: results
                .into_iter()
                .map(|(line, button)| (line, id(button)))
                .collect(),
            found: Vec::new(),
            dismissed: panel.dismisser(),
        };
        (panel, sheet)
    }

    pub fn searching(&self, ui: &mut Ui) {
        let text = tr!("Searching…");
        ui.update_widget::<Label>(self.status, |label, _| label.set_text(&text));
    }

    pub fn answered(&mut self, ui: &mut Ui, found: Result<Vec<Principal>, String>) {
        let (status, found) = match found {
            Ok(found) if found.is_empty() => (tr!("No one found."), found),
            Ok(found) => (String::new(), found),
            Err(error) => (tr!("Can’t search: {error}", error = error), Vec::new()),
        };
        ui.update_widget::<Label>(self.status, |label, _| label.set_text(&status));
        let panel = self.panel;
        for (index, (line, button)) in self.results.iter().enumerate() {
            let text = found.get(index).map(title);
            let (line, shown) = (*line, text.is_some());
            ui.update_widget::<Button>(*button, |button, _| {
                button.set_title(&text.unwrap_or_default())
            });
            ui.update_widget::<Panel>(panel, |panel, _| panel.show_line(line, shown));
        }
        self.found = found;
    }

    pub fn picked(&self, index: usize) -> Option<&Principal> {
        self.found.get(index)
    }

    pub fn close(&self, ui: &mut Ui) {
        self.dismissed.set(true);
        ui.perform(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: self.cancel.into(),
            data: None,
        });
    }
}
