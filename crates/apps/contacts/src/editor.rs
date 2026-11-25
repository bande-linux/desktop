use std::cell::RefCell;
use std::rc::Rc;

use bande_address_book::vcard::{Address, Birthday, Contact, Label, Labelled, Photo};
use bande_datetime::DateFormats as Formats;
use bande_l10n::tr;
use bande_toolkit::ui::{
    Axis, Button, Form, FormLine, PopUpButton, ScrollState, ScrollView, Stack, TextField, TextView,
    Ui, Widget, WidgetId,
};

use crate::widgets::{Avatar, Pair};

const LABEL_WIDTH: f32 = 110.0;
const MONTH_WIDTH: f32 = 150.0;
const DAY_WIDTH: f32 = 76.0;
const PHOTO_SIZE: f32 = 72.0;
const SPACING: f32 = 16.0;
const PADDING: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Add {
    Email,
    Phone,
    Address,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    Add(Add),
    ChoosePhoto,
    RemovePhoto,
    Done,
    Cancel,
}

pub type Signals = Rc<RefCell<Vec<Signal>>>;

const LABELS: [Label; 4] = [Label::Home, Label::Work, Label::Mobile, Label::Other];

pub fn label_name(label: Label) -> String {
    match label {
        Label::Home => tr!("home"),
        Label::Work => tr!("work"),
        Label::Mobile => tr!("mobile"),
        Label::Other => tr!("other"),
    }
}

fn label_choices(phone: bool) -> Vec<Label> {
    LABELS
        .into_iter()
        .filter(|label| phone || *label != Label::Mobile)
        .collect()
}

struct AddressFields {
    label: WidgetId,
    street: WidgetId,
    locality: WidgetId,
    region: WidgetId,
    code: WidgetId,
    country: WidgetId,
}

/// The contact being edited in the window, and where its fields are.
pub struct Editor {
    pub key: Option<String>,
    pub base: Contact,
    pub photo: Option<Photo>,
    pub choosing_photo: bool,
    pub books: Vec<(String, String)>,
    pub book: String,
    pub groups: Vec<String>,
    given: WidgetId,
    family: WidgetId,
    organisation: WidgetId,
    title: WidgetId,
    emails: Vec<(WidgetId, WidgetId)>,
    phones: Vec<(WidgetId, WidgetId)>,
    addresses: Vec<AddressFields>,
    month: WidgetId,
    day: WidgetId,
    year: WidgetId,
    group_field: WidgetId,
    note: WidgetId,
    book_field: Option<WidgetId>,
    scroll: Rc<RefCell<ScrollState>>,
}

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn send(signals: &Signals, signal: Signal) -> impl FnMut() + 'static {
    let signals = Rc::clone(signals);
    move || signals.borrow_mut().push(signal.clone())
}

fn field(placeholder: &str, value: &str) -> (TextField, WidgetId) {
    let field = TextField::new(placeholder, value, |_| {});
    let id = field.id();
    (field, id)
}

fn one_line(street: &str) -> String {
    street.replace('\n', ", ")
}

fn listed_groups(groups: &[String]) -> String {
    groups.join(", ").trim().to_owned()
}

fn popup(name: &str, choices: &[String], selected: usize) -> (PopUpButton, WidgetId) {
    let popup = PopUpButton::new(name, choices, selected, |_| {});
    let id = popup.id();
    (popup, id)
}

fn within_month(birthday: Birthday) -> Birthday {
    (1..=birthday.day)
        .rev()
        .map(|day| Birthday { day, ..birthday })
        .find(|birthday| birthday.date().is_some())
        .unwrap_or(birthday)
}

impl Editor {
    pub fn new(
        key: Option<String>,
        base: Contact,
        books: Vec<(String, String)>,
        book: String,
        groups: Vec<String>,
    ) -> Self {
        Self {
            key,
            photo: base.photo.clone(),
            choosing_photo: false,
            base,
            books,
            book,
            groups,
            given: WidgetId::next(),
            family: WidgetId::next(),
            organisation: WidgetId::next(),
            title: WidgetId::next(),
            emails: Vec::new(),
            phones: Vec::new(),
            addresses: Vec::new(),
            month: WidgetId::next(),
            day: WidgetId::next(),
            year: WidgetId::next(),
            group_field: WidgetId::next(),
            note: WidgetId::next(),
            book_field: None,
            scroll: Rc::default(),
        }
    }

    fn labelled_lines(
        &mut self,
        lines: &mut Vec<FormLine>,
        values: &[Labelled],
        phone: bool,
    ) -> Vec<(WidgetId, WidgetId)> {
        let choices = label_choices(phone);
        let names: Vec<String> = choices.iter().map(|label| label_name(*label)).collect();
        let (kind, placeholder) = match phone {
            true => (tr!("Phone"), tr!("Phone number")),
            false => (tr!("Email"), tr!("Email address")),
        };
        values
            .iter()
            .map(|value| {
                let selected = choices
                    .iter()
                    .position(|label| *label == value.label)
                    .unwrap_or(choices.len() - 1);
                let (label, label_id) =
                    popup(&tr!("{kind} label", kind = kind.clone()), &names, selected);
                let (text, text_id) = field(&placeholder, &value.value);
                lines.push(FormLine::Row(
                    kind.clone(),
                    boxed(Pair::new(boxed(label), LABEL_WIDTH, boxed(text))),
                ));
                (label_id, text_id)
            })
            .collect()
    }

    /// The editor's widgets for `contact`, the values shown in its fields.
    pub fn build(
        &mut self,
        contact: &Contact,
        signals: &Signals,
        formats: &Formats,
        photo: Option<bande_toolkit::Image>,
    ) -> Box<dyn Widget> {
        let mut lines = Vec::new();
        let text_line = |lines: &mut Vec<FormLine>, label: String, value: &str| {
            let (text, id) = field(&label, value);
            lines.push(FormLine::Row(label, boxed(text)));
            id
        };
        self.given = text_line(&mut lines, tr!("First name"), &contact.name.given);
        self.family = text_line(&mut lines, tr!("Last name"), &contact.name.family);
        self.organisation = text_line(&mut lines, tr!("Organization"), &contact.organisation);
        self.title = text_line(&mut lines, tr!("Job title"), &contact.title);
        lines.push(FormLine::Heading(tr!("Email")));
        self.emails = self.labelled_lines(&mut lines, &contact.emails, false);
        lines.push(FormLine::Wide(boxed(
            Button::new(&tr!("Add Email"), send(signals, Signal::Add(Add::Email))).quiet(),
        )));
        lines.push(FormLine::Heading(tr!("Phone")));
        self.phones = self.labelled_lines(&mut lines, &contact.phones, true);
        lines.push(FormLine::Wide(boxed(
            Button::new(&tr!("Add Phone"), send(signals, Signal::Add(Add::Phone))).quiet(),
        )));
        lines.push(FormLine::Heading(tr!("Address")));
        let choices = label_choices(false);
        let names: Vec<String> = choices.iter().map(|label| label_name(*label)).collect();
        self.addresses = contact
            .addresses
            .iter()
            .map(|address| {
                let selected = choices
                    .iter()
                    .position(|label| *label == address.label)
                    .unwrap_or(choices.len() - 1);
                let (label, label_id) = popup(&tr!("Address label"), &names, selected);
                lines.push(FormLine::Row(tr!("Label"), boxed(label)));
                AddressFields {
                    label: label_id,
                    street: text_line(&mut lines, tr!("Street"), &one_line(&address.street)),
                    locality: text_line(&mut lines, tr!("City"), &address.locality),
                    region: text_line(&mut lines, tr!("Region"), &address.region),
                    code: text_line(&mut lines, tr!("Postal code"), &address.code),
                    country: text_line(&mut lines, tr!("Country"), &address.country),
                }
            })
            .collect();
        lines.push(FormLine::Wide(boxed(
            Button::new(
                &tr!("Add Address"),
                send(signals, Signal::Add(Add::Address)),
            )
            .quiet(),
        )));
        lines.push(FormLine::Heading(tr!("More")));
        let mut months = vec![tr!("No birthday")];
        months.extend((1..=12).map(|month| formats.month_name(month)));
        let mut days = vec!["–".to_owned()];
        days.extend((1..=31).map(|day: i8| day.to_string()));
        let birthday = contact.birthday;
        let (month, month_id) = popup(
            &tr!("Birthday month"),
            &months,
            birthday.map_or(0, |day| day.month as usize),
        );
        let (day, day_id) = popup(
            &tr!("Birthday day"),
            &days,
            birthday.map_or(0, |day| day.day as usize),
        );
        let year_text = birthday
            .and_then(|day| day.year)
            .map(|year| year.to_string())
            .unwrap_or_default();
        let (year, year_id) = field(&tr!("Year"), &year_text);
        (self.month, self.day, self.year) = (month_id, day_id, year_id);
        lines.push(FormLine::Row(
            tr!("Birthday"),
            boxed(Pair::new(
                boxed(month),
                MONTH_WIDTH,
                boxed(Pair::new(boxed(day), DAY_WIDTH, boxed(year))),
            )),
        ));
        self.group_field = text_line(&mut lines, tr!("Groups"), &listed_groups(&self.groups));
        if self.key.is_none() && self.books.len() > 1 {
            let names: Vec<String> = self.books.iter().map(|(_, name)| name.clone()).collect();
            let selected = self
                .books
                .iter()
                .position(|(id, _)| *id == self.book)
                .unwrap_or(0);
            let (book, id) = popup(&tr!("Address book"), &names, selected);
            self.book_field = Some(id);
            lines.push(FormLine::Row(tr!("Address book"), boxed(book)));
        }
        let note = TextView::new(&tr!("Note"), &contact.note, |_| {})
            .named(&tr!("Note"))
            .visible_lines(4);
        self.note = note.id();
        lines.push(FormLine::Row(tr!("Note"), boxed(note)));
        let name = bande_address_book::vcard::display_name(contact);
        let mut photo_buttons: Vec<Box<dyn Widget>> = vec![boxed(Button::new(
            &tr!("Choose Photo…"),
            send(signals, Signal::ChoosePhoto),
        ))];
        if self.photo.is_some() {
            photo_buttons.push(boxed(Button::new(
                &tr!("Remove Photo"),
                send(signals, Signal::RemovePhoto),
            )));
        }
        let header = Stack::new(
            Axis::Horizontal,
            SPACING,
            0.0,
            vec![
                boxed(Avatar::new(photo, &name, PHOTO_SIZE)),
                boxed(Stack::new(Axis::Horizontal, 8.0, 0.0, photo_buttons)),
            ],
        );
        let buttons = Stack::new(
            Axis::Horizontal,
            8.0,
            0.0,
            vec![
                boxed(Button::new(&tr!("Cancel"), send(signals, Signal::Cancel)).cancel()),
                boxed(Button::new(&tr!("Done"), send(signals, Signal::Done)).primary()),
            ],
        )
        .trailing();
        let content = Stack::new(
            Axis::Vertical,
            SPACING,
            PADDING,
            vec![
                boxed(header),
                boxed(Form::new(&tr!("Contact"), lines)),
                boxed(buttons),
            ],
        );
        boxed(ScrollView::shared(boxed(content), Rc::clone(&self.scroll)))
    }

    pub fn given_field(&self) -> WidgetId {
        self.given
    }

    /// The first field of the email, phone or address that was added last.
    pub fn added_field(&self, add: Add) -> WidgetId {
        let last = |fields: &[(WidgetId, WidgetId)]| fields.last().map(|(_, text)| *text);
        match add {
            Add::Email => last(&self.emails),
            Add::Phone => last(&self.phones),
            Add::Address => self.addresses.last().map(|address| address.street),
        }
        .unwrap_or(self.given)
    }

    /// The contact as the fields have it now.
    pub fn read(&mut self, ui: &Ui) -> Contact {
        let text = |id: WidgetId| {
            ui.widget_ref::<TextField>(id)
                .map(|field| field.text().trim().to_owned())
                .unwrap_or_default()
        };
        let picked = |id: WidgetId| {
            ui.widget_ref::<PopUpButton>(id)
                .map_or(0, PopUpButton::selected)
        };
        let labelled = |fields: &[(WidgetId, WidgetId)], phone: bool| {
            let choices = label_choices(phone);
            fields
                .iter()
                .map(|(label, value)| Labelled {
                    label: choices.get(picked(*label)).copied().unwrap_or_default(),
                    value: text(*value),
                })
                .collect::<Vec<_>>()
        };
        let mut contact = self.base.clone();
        let (given, family) = (text(self.given), text(self.family));
        if given != contact.name.given || family != contact.name.family {
            contact.formatted.clear();
        }
        contact.name.given = given;
        contact.name.family = family;
        contact.organisation = text(self.organisation);
        contact.title = text(self.title);
        contact.emails = labelled(&self.emails, false);
        contact.phones = labelled(&self.phones, true);
        let choices = label_choices(false);
        contact.addresses = self
            .addresses
            .iter()
            .enumerate()
            .map(|(index, fields)| Address {
                label: choices
                    .get(picked(fields.label))
                    .copied()
                    .unwrap_or_default(),
                street: match (text(fields.street), self.base.addresses.get(index)) {
                    (typed, Some(base)) if typed == one_line(&base.street) => base.street.clone(),
                    (typed, _) => typed,
                },
                locality: text(fields.locality),
                region: text(fields.region),
                code: text(fields.code),
                country: text(fields.country),
            })
            .collect();
        let (month, day) = (picked(self.month), picked(self.day));
        contact.birthday = (month > 0 && day > 0)
            .then(|| Birthday {
                year: text(self.year)
                    .parse()
                    .ok()
                    .filter(|year| (1..=9999).contains(year)),
                month: month as i8,
                day: day as i8,
            })
            .map(within_month);
        contact.note = ui
            .widget_ref::<TextView>(self.note)
            .map(|note| note.text().trim().to_owned())
            .unwrap_or_default();
        contact.photo = self.photo.clone();
        let groups = text(self.group_field);
        if groups != listed_groups(&self.groups) {
            self.groups = groups
                .split(',')
                .map(|group| group.trim().to_owned())
                .filter(|group| !group.is_empty())
                .collect();
        }
        if let Some(book) = self
            .book_field
            .and_then(|id| self.books.get(picked(id)))
            .map(|(id, _)| id.clone())
        {
            self.book = book;
        }
        contact
    }

    /// The contact with one more empty email, phone or address to fill in.
    pub fn with_added(contact: &mut Contact, add: Add) {
        match add {
            Add::Email => contact.emails.push(Labelled::default()),
            Add::Phone => contact.phones.push(Labelled::default()),
            Add::Address => contact.addresses.push(Address::default()),
        }
    }

    /// Whether the contact has anything in it worth keeping.
    pub fn empty(contact: &Contact) -> bool {
        bande_address_book::vcard::display_name(contact).is_empty()
            && contact.note.is_empty()
            && contact.addresses.iter().all(Address::is_empty)
    }
}
