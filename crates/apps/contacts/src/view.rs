use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use bande_address_book::store::{Account, AccountKind, Found, LOCAL_ACCOUNT, Library};
use bande_address_book::vcard::{
    Address, Birthday, Card, Contact, Kind, Photo, display_name, sort_key,
};
use bande_datetime::DateFormats as Formats;
use bande_l10n::{tr, trn};
use bande_toolkit::ui::{
    Alert, Axis, Button, Cell, Column, Form, FormLine, Heading, Label, Menu, MenuItem, PopUpButton,
    Row, ScrollView, SearchField, Stack, Table, Ui, Widget, WidgetId,
};
use bande_toolkit::{Appearance, Image, Palette, SHEET_INSET, TextSystem};

use crate::editor::{self, Editor, Signal, label_name};
use crate::setup::{Done, Setup};
use crate::widgets::{Avatar, LetterIndex, Panes, TOOLBAR_HEIGHT};
use crate::worker::Job;

const AVATAR_SIZE: f32 = 72.0;
const SPACING: f32 = 16.0;
const PADDING: f32 = 24.0;
const ALL: &str = "all";
const MAX_PHOTO_PIXELS: u32 = 256;
const SYMBOL_PIXELS: u32 = 32;
const ADD_SYMBOL: &str = include_str!("../data/symbols/add.svg");

pub const NEW_CONTACT: &str = "new-contact";
pub const EDIT_CONTACT: &str = "edit-contact";
pub const DELETE_CONTACT: &str = "delete-contact";
pub const IMPORT: &str = "import";
pub const EXPORT: &str = "export";
pub const ADD_ACCOUNT: &str = "add-account";
pub const REFRESH: &str = "refresh";
pub const MERGE: &str = "merge-duplicates";
pub const FIND: &str = "find";
pub const UNDO: &str = "undo";
pub const REDO: &str = "redo";

/// What the window asks the app to do: the network, the file chooser and other apps.
#[derive(Debug)]
pub enum Request {
    Run(Job),
    Sync,
    ChooseImport,
    ChooseExport(String),
    ChoosePhoto,
    Open(String),
    Read(Vec<std::path::PathBuf>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Intent {
    Scope(String),
    Select(String),
    Activate,
    Search(String),
    Letter(String),
    Command(&'static str),
    Row(String, String),
    Sidebar(String, String),
    Confirm(String),
    Import(bool),
    Open(String),
    Failure(String),
}

type Intents = Rc<RefCell<Vec<Intent>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    book: String,
    href: String,
    before: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    key: String,
    found: Found,
    name: String,
    letter: String,
}

/// How syncing goes: whether the computer is online and which accounts failed and why.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub offline: bool,
    pub failed: Vec<(String, String)>,
}

pub struct ContactsView {
    ui: Ui,
    panes: WidgetId,
    sidebar: WidgetId,
    search: WidgetId,
    list: WidgetId,
    index: WidgetId,
    pub library: Library,
    scope: String,
    query: String,
    shown: Vec<Shown>,
    selected: Option<String>,
    editor: Option<Editor>,
    signals: editor::Signals,
    setup: Option<Setup>,
    pending_import: Vec<Card>,
    import_picker: Option<(WidgetId, Vec<(usize, String)>)>,
    undo: Vec<Vec<Step>>,
    redo: Vec<Vec<Step>>,
    intents: Intents,
    requests: Vec<Request>,
    formats: Formats,
    palette: Palette,
    pub status: Status,
    pub calendar_installed: bool,
}

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn push(intents: &Intents, intent: Intent) {
    intents.borrow_mut().push(intent);
}

fn key_of(library: &Library, found: &Found) -> Option<String> {
    let book = library.books.get(found.book)?;
    let stored = book.cards.get(found.card)?;
    Some(format!("{}\n{}", book.info.id, stored.href))
}

fn letter_of(name: &str) -> String {
    match name.chars().next() {
        Some(first) if first.is_alphabetic() => first.to_uppercase().collect(),
        _ => "#".to_owned(),
    }
}

fn photo_image(photo: Option<&Photo>) -> Option<Image> {
    let bytes = &photo?.bytes;
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let (width, height) = reader.into_dimensions().ok()?;
    if u64::from(width) * u64::from(height) > 40_000_000 {
        return None;
    }
    Image::decode(bytes, MAX_PHOTO_PIXELS)
}

fn birthday_text(formats: &Formats, birthday: Birthday) -> String {
    let date = jiff::civil::date(birthday.year.unwrap_or(2000), birthday.month, birthday.day);
    match date.to_zoned(jiff::tz::TimeZone::UTC) {
        Ok(zoned) => formats.date(&zoned, birthday.year.is_some()),
        Err(_) => String::new(),
    }
}

fn address_text(address: &Address) -> String {
    let city = [address.code.trim(), address.locality.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    [
        address.street.trim(),
        city.as_str(),
        address.region.trim(),
        address.country.trim(),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

fn normalised(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The groups of the contact with `uid` in `book`: the group cards that list it, and its
/// `CATEGORIES`.
fn groups_of(library: &Library, book: usize, contact: &Contact, uid: &str) -> Vec<String> {
    let mut groups: BTreeSet<String> = contact.categories.iter().cloned().collect();
    if let Some(book) = library.books.get(book) {
        for card in book.cards.iter().filter_map(|stored| stored.shown()) {
            if card.kind() == Kind::Group && card.members().iter().any(|member| member == uid) {
                groups.insert(display_name(&card.contact()));
            }
        }
    }
    groups.into_iter().collect()
}

fn book_groups(library: &Library, book: usize) -> Vec<String> {
    let mut groups = BTreeSet::new();
    if let Some(book) = library.books.get(book) {
        for card in book.cards.iter().filter_map(|stored| stored.shown()) {
            match card.kind() {
                Kind::Group => {
                    groups.insert(display_name(&card.contact()));
                }
                _ => groups.extend(card.contact().categories),
            }
        }
    }
    groups
        .into_iter()
        .filter(|group| !group.is_empty())
        .collect()
}

impl ContactsView {
    pub fn new(library: Library, appearance: Appearance) -> Self {
        let intents = Intents::default();
        let scoping = Rc::clone(&intents);
        let sidebar_menu = Rc::clone(&intents);
        let sidebar = Table::new(vec![Column::new("", 200.0).flexible()])
            .without_header()
            .sidebar()
            .on_selection(move |keys| {
                if let Some(key) = keys.first() {
                    push(&scoping, Intent::Scope(key.clone()));
                }
            })
            .context_menu(
                |keys| {
                    let key = keys.first()?;
                    let account = key.strip_prefix("account:")?;
                    Some(Menu::new(vec![
                        MenuItem::new(&tr!("Server Settings…"), &format!("settings:{account}")),
                        MenuItem::new(&tr!("Remove Account…"), &format!("remove:{account}")),
                    ]))
                },
                move |action, keys| {
                    if let Some(key) = keys.first() {
                        push(
                            &sidebar_menu,
                            Intent::Sidebar(action.to_owned(), key.clone()),
                        );
                    }
                },
            );
        let sidebar_id = sidebar.id();
        let searching = Rc::clone(&intents);
        let search = SearchField::new(&tr!("Search"), "", move |text| {
            push(&searching, Intent::Search(text.to_owned()));
        });
        let search_id = search.id();
        let (selecting, activating, row_menu) = (
            Rc::clone(&intents),
            Rc::clone(&intents),
            Rc::clone(&intents),
        );
        let list = Table::new(vec![Column::new(&tr!("Name"), 260.0).flexible()])
            .without_header()
            .on_selection(move |keys| {
                if let Some(key) = keys.first() {
                    push(&selecting, Intent::Select(key.clone()));
                }
            })
            .on_activate(move |_| push(&activating, Intent::Activate))
            .context_menu(
                |keys| {
                    keys.first()?;
                    Some(Menu::new(vec![
                        MenuItem::new(&tr!("Edit"), "edit"),
                        MenuItem::new(&tr!("Export…"), "export"),
                        MenuItem::separator(),
                        MenuItem::new(&tr!("Delete"), "delete"),
                    ]))
                },
                move |action, keys| {
                    if let Some(key) = keys.first() {
                        push(&row_menu, Intent::Row(action.to_owned(), key.clone()));
                    }
                },
            );
        let list_id = list.id();
        let lettering = Rc::clone(&intents);
        let index = LetterIndex::new(move |letter| {
            push(&lettering, Intent::Letter(letter.to_owned()));
        });
        let index_id = index.id();
        let panes = Panes::new(
            boxed(sidebar),
            boxed(search),
            boxed(list),
            boxed(index),
            boxed(Label::new("")),
        );
        let panes_id = panes.id();
        let mut ui = Ui::new(
            Box::new(panes),
            bande_toolkit::ui::Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        ui.set_sheet_top(TOOLBAR_HEIGHT);
        let mut view = Self {
            ui,
            panes: panes_id,
            sidebar: sidebar_id,
            search: search_id,
            list: list_id,
            index: index_id,
            library,
            scope: ALL.to_owned(),
            query: String::new(),
            shown: Vec::new(),
            selected: None,
            editor: None,
            signals: editor::Signals::default(),
            setup: None,
            pending_import: Vec::new(),
            import_picker: None,
            undo: Vec::new(),
            redo: Vec::new(),
            intents,
            requests: Vec::new(),
            formats: Formats::from_env(),
            palette: appearance.palette(),
            status: Status::default(),
            calendar_installed: false,
        };
        view.refresh();
        view
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    pub fn ui_ref(&self) -> &Ui {
        &self.ui
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.ui.set_appearance(appearance);
        self.refresh();
    }

    pub fn set_full_screen(&mut self, full_screen: bool) {
        let inset = match full_screen {
            true => 0.0,
            false => SHEET_INSET as f32,
        };
        if let Some(panes) = self.ui.widget_as::<Panes>(self.panes) {
            panes.set_inset(inset);
        }
    }

    pub fn header(&self) -> bande_toolkit::Header {
        bande_toolkit::Header {
            title_x: crate::widgets::SIDEBAR_WIDTH + 16.0,
            items: self
                .ui
                .widget_ref::<Panes>(self.panes)
                .map(Panes::tool_frames)
                .unwrap_or_default(),
            tabs: Vec::new(),
        }
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn editing(&self) -> bool {
        self.editor.is_some()
    }

    /// Offers to import the `.vcf` files dropped on the window.
    pub fn dropped(&mut self, paths: Vec<std::path::PathBuf>) {
        self.requests.push(Request::Read(paths));
    }

    fn selected_found(&self) -> Option<Found> {
        let key = self.selected.as_ref()?;
        self.shown
            .iter()
            .find(|shown| &shown.key == key)
            .map(|shown| shown.found.clone())
    }

    fn writable(&self, book: usize) -> bool {
        self.library
            .books
            .get(book)
            .is_some_and(|book| !book.info.read_only)
    }

    fn book_name(&self, book: usize) -> String {
        let Some(book) = self.library.books.get(book) else {
            return String::new();
        };
        match (
            book.info.account == LOCAL_ACCOUNT,
            book.info.name.is_empty(),
        ) {
            (true, _) | (false, true) => tr!("On This Computer"),
            (false, false) => book.info.name.clone(),
        }
    }

    fn account_name(account: &Account) -> String {
        match account.kind {
            AccountKind::Local => tr!("On This Computer"),
            AccountKind::CardDav { .. } => account.name.clone(),
        }
    }

    /// Whether the command can run now, for the menus.
    pub fn can(&self, command: &str) -> bool {
        let selected = self.selected_found();
        match command {
            EDIT_CONTACT | DELETE_CONTACT => {
                self.editor.is_none() && selected.is_some_and(|found| self.writable(found.book))
            }
            EXPORT => !self.shown.is_empty(),
            UNDO => !self.undo.is_empty() && self.editor.is_none(),
            REDO => !self.redo.is_empty() && self.editor.is_none(),
            MERGE => !self.duplicates().is_empty() && self.editor.is_none(),
            _ => true,
        }
    }

    fn in_scope(&self, found: &Found, card: &Card) -> bool {
        match self.scope.split_once(':') {
            Some(("book", id)) => self.library.books[found.book].info.id == id,
            Some(("group", rest)) => {
                let Some((id, group)) = rest.split_once(':') else {
                    return false;
                };
                self.library.books[found.book].info.id == id
                    && groups_of(&self.library, found.book, &card.contact(), &card.uid())
                        .iter()
                        .any(|name| name == group)
            }
            _ => true,
        }
    }

    fn matches(&self, contact: &Contact) -> bool {
        let needle = self.query.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        [
            display_name(contact),
            contact.organisation.clone(),
            contact.title.clone(),
            contact.note.clone(),
        ]
        .into_iter()
        .chain(contact.emails.iter().map(|email| email.value.clone()))
        .chain(contact.phones.iter().map(|phone| phone.value.clone()))
        .any(|text| text.to_lowercase().contains(&needle))
    }

    fn rebuild_list(&mut self) {
        let mut shown: Vec<(String, Shown)> = self
            .library
            .contacts()
            .into_iter()
            .filter(|(_, card)| card.kind() == Kind::Individual)
            .filter(|(found, card)| self.in_scope(found, card))
            .filter_map(|(found, card)| {
                let contact = card.contact();
                if !self.matches(&contact) {
                    return None;
                }
                let name = match display_name(&contact) {
                    name if name.is_empty() => tr!("No Name"),
                    name => name,
                };
                let sort = sort_key(&contact);
                Some((
                    sort.clone(),
                    Shown {
                        key: key_of(&self.library, &found)?,
                        found,
                        name,
                        letter: letter_of(&sort),
                    },
                ))
            })
            .collect();
        shown.sort_by(|a, b| a.0.cmp(&b.0));
        self.shown = shown.into_iter().map(|(_, shown)| shown).collect();
        if self
            .selected
            .as_ref()
            .is_some_and(|key| !self.shown.iter().any(|shown| &shown.key == key))
        {
            self.selected = None;
        }
    }

    fn sidebar_rows(&self) -> (Vec<Row>, Vec<Heading>, Vec<String>) {
        let mut rows = vec![Row::new(ALL, vec![Cell::new(tr!("All Contacts"))])];
        let mut headings = Vec::new();
        let mut expanded = Vec::new();
        for account in &self.library.accounts {
            let books: Vec<usize> = self
                .library
                .books
                .iter()
                .enumerate()
                .filter(|(_, book)| book.info.account == account.id)
                .map(|(index, _)| index)
                .collect();
            if books.is_empty() {
                continue;
            }
            headings.push(Heading::new(rows.len(), Self::account_name(account)));
            if account.kind != AccountKind::Local {
                rows.push(
                    Row::new(
                        format!("account:{}", account.id),
                        vec![Cell::new(tr!("Account Settings")).muted()],
                    )
                    .dimmed(),
                );
            }
            for book in books {
                let info = &self.library.books[book].info;
                let key = format!("book:{}", info.id);
                let children: Vec<Row> = book_groups(&self.library, book)
                    .into_iter()
                    .map(|group| {
                        Row::new(format!("group:{}:{group}", info.id), vec![Cell::new(group)])
                    })
                    .collect();
                let mut row = Row::new(key.clone(), vec![Cell::new(self.book_name(book))]);
                if !children.is_empty() {
                    row = row.children(children).expandable();
                    expanded.push(key);
                }
                rows.push(row);
            }
        }
        (rows, headings, expanded)
    }

    fn tools(&self) -> Vec<Box<dyn Widget>> {
        let mut tools: Vec<Box<dyn Widget>> = Vec::new();
        if self.status.offline {
            tools.push(boxed(Label::new(&tr!("Offline")).secondary()));
        } else if let Some((account, _)) = self.status.failed.first() {
            let intents = Rc::clone(&self.intents);
            let account = account.clone();
            tools.push(boxed(
                Button::new(&tr!("Not Synced"), move || {
                    push(&intents, Intent::Failure(account.clone()))
                })
                .quiet()
                .tooltip(&tr!("Some changes could not be synced")),
            ));
        }
        let duplicates = self.duplicates().len();
        if duplicates > 0 && self.editor.is_none() {
            let intents = Rc::clone(&self.intents);
            tools.push(boxed(
                Button::new(&tr!("Merge Duplicates…"), move || {
                    push(&intents, Intent::Command(MERGE))
                })
                .quiet(),
            ));
        }
        let intents = Rc::clone(&self.intents);
        tools.push(boxed(
            Button::symbol(
                Image::symbolic(ADD_SYMBOL, SYMBOL_PIXELS, self.palette.ink2),
                &tr!("New Contact"),
                move || push(&intents, Intent::Command(NEW_CONTACT)),
            )
            .quiet(),
        ));
        tools
    }

    fn empty_state(&self) -> Box<dyn Widget> {
        let any = self
            .library
            .contacts()
            .iter()
            .any(|(_, card)| card.kind() == Kind::Individual);
        let (title, message) = match (any, self.query.trim().is_empty()) {
            (false, _) => (
                tr!("No Contacts"),
                tr!(
                    "Add a contact, import a .vcf file, or add a CardDAV account to sync the contacts you keep on a server."
                ),
            ),
            (true, false) => (
                tr!("No Results"),
                tr!(
                    "No contact matches “{query}”.",
                    query = self.query.trim().to_owned()
                ),
            ),
            (true, true) => (
                tr!("No Contact Selected"),
                tr!("Choose a contact in the list, or type a name to find one."),
            ),
        };
        let mut children = vec![
            boxed(Label::heading(&title)),
            boxed(Label::new(&message).secondary()),
        ];
        if !any {
            let button = |title: String, command: &'static str| {
                let intents = Rc::clone(&self.intents);
                boxed(Button::new(&title, move || {
                    push(&intents, Intent::Command(command))
                }))
            };
            children.push(boxed(Stack::new(
                Axis::Horizontal,
                8.0,
                0.0,
                vec![
                    button(tr!("New Contact"), NEW_CONTACT),
                    button(tr!("Import…"), IMPORT),
                    button(tr!("Add Account…"), ADD_ACCOUNT),
                ],
            )));
        }
        boxed(Stack::new(Axis::Vertical, SPACING, PADDING * 2.0, children))
    }

    fn detail(&self, found: &Found) -> Box<dyn Widget> {
        let Some(card) = self.library.card(found) else {
            return self.empty_state();
        };
        let contact = card.contact();
        let name = match display_name(&contact) {
            name if name.is_empty() => tr!("No Name"),
            name => name,
        };
        let subtitle = [contact.title.as_str(), contact.organisation.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let mut names = vec![boxed(Label::heading(&name))];
        if !subtitle.is_empty() {
            names.push(boxed(Label::new(&subtitle).secondary()));
        }
        let header = Stack::new(
            Axis::Horizontal,
            SPACING,
            0.0,
            vec![
                boxed(Avatar::new(
                    photo_image(contact.photo.as_ref()),
                    &name,
                    AVATAR_SIZE,
                )),
                boxed(Stack::new(Axis::Vertical, 4.0, 0.0, names)),
            ],
        );
        let mut lines = Vec::new();
        for email in &contact.emails {
            let requests = Rc::clone(&self.intents);
            let address = email.value.clone();
            lines.push(FormLine::Row(
                label_name(email.label),
                boxed(
                    Button::new(&email.value, move || {
                        push(&requests, Intent::Open(format!("mailto:{address}")))
                    })
                    .link()
                    .tooltip(&tr!("New Message")),
                ),
            ));
        }
        for phone in &contact.phones {
            lines.push(FormLine::Row(
                label_name(phone.label),
                boxed(Label::new(&phone.value)),
            ));
        }
        for address in &contact.addresses {
            lines.push(FormLine::Row(
                label_name(address.label),
                boxed(Label::new(&address_text(address))),
            ));
        }
        if let Some(birthday) = contact.birthday {
            lines.push(FormLine::Row(
                tr!("birthday"),
                boxed(Label::new(&birthday_text(&self.formats, birthday))),
            ));
        }
        let groups = groups_of(&self.library, found.book, &contact, &card.uid());
        if !groups.is_empty() {
            lines.push(FormLine::Row(
                tr!("groups"),
                boxed(Label::new(&groups.join(", "))),
            ));
        }
        if !contact.note.is_empty() {
            lines.push(FormLine::Row(tr!("note"), boxed(Label::new(&contact.note))));
        }
        lines.push(FormLine::Row(
            tr!("address book"),
            boxed(Label::new(&self.book_name(found.book)).secondary()),
        ));
        let mut children = vec![boxed(header), boxed(Form::new(&name, lines))];
        if self.writable(found.book) {
            let (editing, deleting) = (Rc::clone(&self.intents), Rc::clone(&self.intents));
            children.push(boxed(
                Stack::new(
                    Axis::Horizontal,
                    8.0,
                    0.0,
                    vec![
                        boxed(Button::new(&tr!("Delete"), move || {
                            push(&deleting, Intent::Command(DELETE_CONTACT))
                        })),
                        boxed(Button::new(&tr!("Edit"), move || {
                            push(&editing, Intent::Command(EDIT_CONTACT))
                        })),
                    ],
                )
                .trailing(),
            ));
        }
        boxed(ScrollView::new(Stack::new(
            Axis::Vertical,
            SPACING,
            PADDING,
            children,
        )))
    }

    /// Shows the library as it is now.
    pub fn refresh(&mut self) {
        self.rebuild_list();
        let rows: Vec<Row> = self
            .shown
            .iter()
            .map(|shown| Row::new(shown.key.clone(), vec![Cell::new(shown.name.clone())]))
            .collect();
        let mut headings = Vec::new();
        let mut letters = Vec::new();
        for (index, shown) in self.shown.iter().enumerate() {
            if letters.last() != Some(&shown.letter) {
                letters.push(shown.letter.clone());
                headings.push(Heading::new(index, shown.letter.clone()));
            }
        }
        let selected = self.selected.clone();
        if let Some(list) = self.ui.widget_as::<Table>(self.list) {
            list.set_rows(rows);
            list.set_headings(headings);
            if let Some(key) = &selected {
                list.select(std::slice::from_ref(key));
            }
        }
        if let Some(index) = self.ui.widget_as::<LetterIndex>(self.index) {
            index.set_letters(letters);
        }
        let (rows, headings, expanded) = self.sidebar_rows();
        let scope = self.scope.clone();
        if let Some(sidebar) = self.ui.widget_as::<Table>(self.sidebar) {
            sidebar.set_rows(rows);
            sidebar.set_headings(headings);
            sidebar.set_expanded_rows(expanded);
            sidebar.select(&[scope]);
        }
        if self.editor.is_none() {
            let detail = match self.selected_found() {
                Some(found) => self.detail(&found),
                None => self.empty_state(),
            };
            self.set_detail(detail);
        }
        let tools = self.tools();
        if let Some(panes) = self.ui.widget_as::<Panes>(self.panes) {
            panes.set_tools(tools);
        }
    }

    fn set_detail(&mut self, detail: Box<dyn Widget>) {
        if let Some(panes) = self.ui.widget_as::<Panes>(self.panes) {
            panes.set_detail(detail);
        }
    }

    pub fn select_uid(&mut self, uid: &str) {
        self.scope = ALL.to_owned();
        self.query.clear();
        self.rebuild_list();
        if let Some(shown) = self.shown.iter().find(|shown| {
            self.library
                .card(&shown.found)
                .is_some_and(|card| card.uid() == uid)
        }) {
            self.selected = Some(shown.key.clone());
        }
        self.refresh();
    }

    fn default_book(&self) -> usize {
        let scoped = self
            .scope
            .strip_prefix("book:")
            .or_else(|| {
                self.scope
                    .strip_prefix("group:")
                    .and_then(|rest| rest.split(':').next())
            })
            .and_then(|id| self.library.book_index(id))
            .filter(|book| self.writable(*book));
        scoped
            .or_else(|| {
                self.library
                    .books
                    .iter()
                    .position(|book| book.info.account == LOCAL_ACCOUNT)
            })
            .unwrap_or(0)
    }

    fn writable_books(&self) -> Vec<(usize, String)> {
        (0..self.library.books.len())
            .filter(|book| self.writable(*book))
            .map(|book| (book, self.book_name(book)))
            .collect()
    }

    /// Opens the editor on a new contact, filled with `contact`.
    pub fn new_contact(&mut self, contact: Contact) {
        let book = self.default_book();
        let groups = match self.scope.strip_prefix("group:") {
            Some(rest) => rest
                .split_once(':')
                .map(|(_, group)| vec![group.to_owned()])
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let editor = Editor::new(None, contact.clone(), self.writable_books(), book, groups);
        self.open_editor(editor, &contact);
    }

    fn open_editor(&mut self, mut editor: Editor, contact: &Contact) {
        let photo = photo_image(editor.photo.as_ref());
        let widget = editor.build(contact, &self.signals, &self.formats, photo);
        let focus = editor.given_field();
        self.editor = Some(editor);
        self.set_detail(widget);
        self.ui.set_focus(focus);
        let tools = self.tools();
        if let Some(panes) = self.ui.widget_as::<Panes>(self.panes) {
            panes.set_tools(tools);
        }
    }

    fn edit_selected(&mut self) {
        let Some(found) = self
            .selected_found()
            .filter(|found| self.writable(found.book))
        else {
            return;
        };
        let Some(card) = self.library.card(&found) else {
            return;
        };
        let contact = card.contact();
        let groups = groups_of(&self.library, found.book, &contact, &card.uid());
        let editor = Editor::new(
            self.selected.clone(),
            contact.clone(),
            self.writable_books(),
            found.book,
            groups,
        );
        self.open_editor(editor, &contact);
    }

    /// Puts the chosen picture into the contact being edited, made small enough for a card.
    pub fn set_photo(&mut self, photo: Photo) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let contact = editor.read(&self.ui);
        editor.photo = Some(photo);
        let contact = Contact {
            photo: editor.photo.clone(),
            ..contact
        };
        self.open_editor(editor, &contact);
    }

    fn record(&mut self, steps: Vec<Step>) {
        if !steps.is_empty() {
            self.undo.push(steps);
            self.redo.clear();
        }
    }

    fn data_at(&self, book: usize, href: &str) -> Option<String> {
        self.library
            .books
            .get(book)?
            .cards
            .iter()
            .find_map(|stored| {
                (stored.href == href && stored.shown().is_some()).then(|| stored.data.clone())
            })
    }

    fn step_for(&self, book: usize, href: &str) -> Step {
        Step {
            book: self.library.books[book].info.id.clone(),
            href: href.to_owned(),
            before: self.data_at(book, href),
        }
    }

    fn href_of(&self, book: usize, card: usize) -> Option<String> {
        Some(self.library.books.get(book)?.cards.get(card)?.href.clone())
    }

    fn sync_if_remote(&mut self, book: usize) {
        if self
            .library
            .books
            .get(book)
            .is_some_and(|book| book.info.url.is_some())
        {
            self.requests.push(Request::Sync);
        }
    }

    fn set_groups(&mut self, book: usize, uid: &str, groups: &[String], steps: &mut Vec<Step>) {
        if !self.library.books[book].uses_group_cards() {
            return;
        }
        let names = book_groups(&self.library, book);
        let group_cards: Vec<(usize, Card)> = self.library.books[book]
            .cards
            .iter()
            .enumerate()
            .filter_map(|(index, stored)| {
                stored
                    .shown()
                    .filter(|card| card.kind() == Kind::Group)
                    .map(|card| (index, card.clone()))
            })
            .collect();
        for (index, mut card) in group_cards {
            let name = display_name(&card.contact());
            let mut members = card.members();
            let wanted = groups.contains(&name);
            let has = members.iter().any(|member| member == uid);
            if wanted == has {
                continue;
            }
            match wanted {
                true => members.push(uid.to_owned()),
                false => members.retain(|member| member != uid),
            }
            card.set_members(&members);
            if let Some(href) = self.href_of(book, index) {
                steps.push(self.step_for(book, &href));
            }
            self.library.put_card(book, card);
        }
        for group in groups.iter().filter(|group| !names.contains(group)) {
            let mut card = Card::new_group(&bande_dav::new_uid(), group);
            card.set_members(&[uid.to_owned()]);
            if let Some(index) = self.library.put_card(book, card)
                && let Some(href) = self.href_of(book, index)
            {
                steps.push(Step {
                    book: self.library.books[book].info.id.clone(),
                    href,
                    before: None,
                });
            }
        }
    }

    fn save_editor(&mut self) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let mut contact = editor.read(&self.ui);
        if Editor::empty(&contact) {
            self.refresh();
            return;
        }
        let book = editor.book;
        let uses_cards = self
            .library
            .books
            .get(book)
            .is_some_and(|book| book.uses_group_cards());
        if !uses_cards {
            contact.categories = editor.groups.clone();
        }
        let mut steps = Vec::new();
        let existing = editor
            .key
            .as_ref()
            .and_then(|key| self.shown.iter().find(|shown| &shown.key == key))
            .map(|shown| shown.found.clone());
        let created = existing.is_none();
        let (index, uid) = match existing {
            Some(found) => {
                let uid = self.library.card(&found).map(Card::uid).unwrap_or_default();
                if let Some(href) = self.href_of(found.book, found.card) {
                    steps.push(self.step_for(found.book, &href));
                }
                (self.library.edit(&found, &contact), uid)
            }
            None => {
                let uid = bande_dav::new_uid();
                (self.library.put_card(book, Card::new(&uid, &contact)), uid)
            }
        };
        if let Some(index) = index {
            if created && let Some(href) = self.href_of(book, index) {
                steps.push(Step {
                    book: self.library.books[book].info.id.clone(),
                    href,
                    before: None,
                });
            }
            self.selected = key_of(&self.library, &Found { book, card: index });
        }
        let groups = editor.groups.clone();
        self.set_groups(book, &uid, &groups, &mut steps);
        self.record(steps);
        self.sync_if_remote(book);
        self.refresh();
    }

    fn delete(&mut self, found: Found) {
        let Some(href) = self.href_of(found.book, found.card) else {
            return;
        };
        let step = self.step_for(found.book, &href);
        if self.library.remove_card(&found).is_some() {
            self.record(vec![step]);
            self.selected = None;
            self.sync_if_remote(found.book);
        }
        self.refresh();
    }

    fn apply_steps(&mut self, steps: Vec<Step>) -> Vec<Step> {
        let mut inverse = Vec::new();
        for step in steps.into_iter().rev() {
            let Some(book) = self.library.book_index(&step.book) else {
                continue;
            };
            inverse.push(Step {
                before: self.data_at(book, &step.href),
                ..step.clone()
            });
            match step.before {
                Some(data) => {
                    self.library.restore(book, &step.href, data);
                }
                None => {
                    let at = self.library.books[book]
                        .cards
                        .iter()
                        .position(|stored| stored.href == step.href);
                    if let Some(card) = at {
                        self.library.remove_card(&Found { book, card });
                    }
                }
            }
            self.sync_if_remote(book);
        }
        inverse.reverse();
        inverse
    }

    fn undo(&mut self) {
        if let Some(steps) = self.undo.pop() {
            let inverse = self.apply_steps(steps);
            self.redo.push(inverse);
        }
        self.refresh();
    }

    fn redo(&mut self) {
        if let Some(steps) = self.redo.pop() {
            let inverse = self.apply_steps(steps);
            self.undo.push(inverse);
        }
        self.refresh();
    }

    /// The contacts of each writable book that look like one person: the same name, or an
    /// address in common.
    fn duplicates(&self) -> Vec<Vec<Found>> {
        let mut sets: Vec<Vec<Found>> = Vec::new();
        for book in 0..self.library.books.len() {
            if !self.writable(book) {
                continue;
            }
            let people: Vec<(Found, Contact)> = self
                .library
                .contacts()
                .into_iter()
                .filter(|(found, card)| found.book == book && card.kind() == Kind::Individual)
                .map(|(found, card)| (found, card.contact()))
                .collect();
            let mut parent: Vec<usize> = (0..people.len()).collect();
            fn root(parent: &mut [usize], mut at: usize) -> usize {
                while parent[at] != at {
                    parent[at] = parent[parent[at]];
                    at = parent[at];
                }
                at
            }
            let mut seen: HashMap<String, usize> = HashMap::new();
            for (index, (_, contact)) in people.iter().enumerate() {
                let name = normalised(&display_name(contact));
                let keys = std::iter::once(format!("name:{name}"))
                    .filter(|_| !name.is_empty())
                    .chain(
                        contact
                            .emails
                            .iter()
                            .map(|email| format!("email:{}", email.value.to_lowercase())),
                    );
                for key in keys {
                    match seen.get(&key) {
                        Some(&other) => {
                            let (a, b) = (root(&mut parent, index), root(&mut parent, other));
                            parent[a] = b;
                        }
                        None => {
                            seen.insert(key, index);
                        }
                    }
                }
            }
            let mut groups: HashMap<usize, Vec<Found>> = HashMap::new();
            for (index, (found, _)) in people.iter().enumerate() {
                groups
                    .entry(root(&mut parent, index))
                    .or_default()
                    .push(found.clone());
            }
            sets.extend(groups.into_values().filter(|set| set.len() > 1));
        }
        sets
    }

    fn ask_merge(&mut self) {
        let sets = self.duplicates();
        if sets.is_empty() {
            return;
        }
        let names: Vec<String> = sets
            .iter()
            .filter_map(|set| self.library.card(&set[0]))
            .map(|card| display_name(&card.contact()))
            .take(5)
            .collect();
        let count = sets.iter().map(Vec::len).sum::<usize>();
        let title = trn!(
            "Merge {count} duplicate contact?",
            "Merge {count} duplicate contacts?",
            count = count
        );
        let message = tr!(
            "Contacts that share a name or an email address become one, with every address, number and note of each: {names}.",
            names = names.join(", ")
        );
        let intents = Rc::clone(&self.intents);
        self.ui.present_sheet(
            Alert::new(&title, &message, move |action| {
                push(&intents, Intent::Confirm(action.to_owned()))
            })
            .cancel(&tr!("Not Now"), "cancel")
            .primary(&tr!("Merge"), "merge"),
        );
    }

    fn merge(&mut self) {
        let mut steps = Vec::new();
        let mut books = BTreeSet::new();
        for set in self.duplicates() {
            let Some((first, rest)) = set.split_first() else {
                continue;
            };
            let Some(base) = self.library.card(first).map(Card::contact) else {
                continue;
            };
            let mut merged = base.clone();
            let mut removed = Vec::new();
            for found in rest {
                let Some(other) = self.library.card(found).map(Card::contact) else {
                    continue;
                };
                merge_into(&mut merged, other);
                if let Some(href) = self.href_of(found.book, found.card) {
                    removed.push(href);
                }
            }
            if let Some(href) = self.href_of(first.book, first.card) {
                steps.push(self.step_for(first.book, &href));
            }
            self.library.edit(first, &merged);
            for href in removed {
                steps.push(self.step_for(first.book, &href));
                let at = self.library.books[first.book]
                    .cards
                    .iter()
                    .position(|stored| stored.href == href);
                if let Some(card) = at {
                    self.library.remove_card(&Found {
                        book: first.book,
                        card,
                    });
                }
            }
            books.insert(first.book);
        }
        self.record(steps);
        for book in books {
            self.sync_if_remote(book);
        }
        self.selected = None;
        self.refresh();
    }

    /// Offers to import the cards of an opened or dropped file.
    pub fn offer_import(&mut self, name: &str, cards: Vec<Card>, skipped: usize) {
        if cards.is_empty() {
            let message = match skipped {
                0 => tr!("There are no contacts in “{name}”.", name = name.to_owned()),
                _ => tr!(
                    "The contacts in “{name}” could not be read.",
                    name = name.to_owned()
                ),
            };
            self.alert(&tr!("Nothing to Import"), &message);
            return;
        }
        let books = self.writable_books();
        let names: Vec<String> = books.iter().map(|(_, name)| name.clone()).collect();
        let default = self.default_book();
        let selected = books
            .iter()
            .position(|(book, _)| *book == default)
            .unwrap_or(0);
        let count = cards.len();
        let title = trn!(
            "Import {count} contact from “{name}”?",
            "Import {count} contacts from “{name}”?",
            count = count,
            name = name.to_owned()
        );
        let message = match skipped {
            0 => tr!("Contacts that are already there are updated."),
            _ => trn!(
                "{skipped} contact in the file could not be read and is left out. Contacts that are already there are updated.",
                "{skipped} contacts in the file could not be read and are left out. Contacts that are already there are updated.",
                skipped = skipped
            ),
        };
        let picker = PopUpButton::new(&tr!("Address book"), &names, selected, |_| {});
        let picker_id = picker.id();
        self.pending_import = cards;
        let intents = Rc::clone(&self.intents);
        let mut alert = Alert::new(&title, &message, move |action| {
            push(&intents, Intent::Import(action == "import"))
        });
        if books.len() > 1 {
            alert = alert.accessory(Form::new(
                &title,
                vec![FormLine::Row(tr!("Into"), boxed(picker))],
            ));
        }
        self.import_picker = Some((picker_id, books));
        self.ui.present_sheet(
            alert
                .cancel(&tr!("Cancel"), "cancel")
                .primary(&tr!("Import"), "import"),
        );
    }

    fn import(&mut self) {
        let books = self.import_picker.take();
        let book = books
            .and_then(|(picker, books)| {
                let picked = self
                    .ui
                    .widget_ref::<PopUpButton>(picker)
                    .map_or(0, PopUpButton::selected);
                books.get(picked).map(|(book, _)| *book)
            })
            .unwrap_or_else(|| self.default_book());
        let mut steps = Vec::new();
        for mut card in std::mem::take(&mut self.pending_import) {
            if card.uid().is_empty() {
                card.set_uid(&bande_dav::new_uid());
            }
            let before = self.library.books[book].find(&card.uid());
            if let Some(index) = before
                && let Some(href) = self.href_of(book, index)
            {
                steps.push(self.step_for(book, &href));
            }
            if let Some(index) = self.library.put_card(book, card)
                && before.is_none()
                && let Some(href) = self.href_of(book, index)
            {
                steps.push(Step {
                    book: self.library.books[book].info.id.clone(),
                    href,
                    before: None,
                });
            }
        }
        self.record(steps);
        self.sync_if_remote(book);
        self.scope = format!("book:{}", self.library.books[book].info.id);
        self.refresh();
    }

    /// The `.vcf` text of the contacts the list shows, or of the selected one.
    fn export_text(&self, only_selected: bool) -> String {
        let chosen: Vec<&Card> = match (only_selected, self.selected_found()) {
            (true, Some(found)) => self.library.card(&found).into_iter().collect(),
            _ => self
                .shown
                .iter()
                .filter_map(|shown| self.library.card(&shown.found))
                .collect(),
        };
        bande_address_book::vcard::write_cards(chosen)
    }

    pub fn alert(&mut self, title: &str, message: &str) {
        self.ui
            .present_sheet(Alert::new(title, message, |_| {}).primary(&tr!("OK"), "ok"));
    }

    /// Opens the account sheet.
    pub fn add_account(&mut self) {
        let mut setup = Setup::add(self.calendar_installed);
        setup.present(&mut self.ui, None);
        self.setup = Some(setup);
    }

    fn account_sheet(&mut self, account: &str, password_only: bool) {
        let Some(Account {
            name,
            kind: AccountKind::CardDav { server, user, .. },
            ..
        }) = self.library.account(account).cloned()
        else {
            return;
        };
        let mut setup = match password_only {
            true => Setup::password(account, &name, &user, &server),
            false => Setup::edit(account, &name, &user, &server),
        };
        setup.present(&mut self.ui, None);
        self.setup = Some(setup);
    }

    /// Asks for the password of an account whose server refused the saved one.
    pub fn ask_password(&mut self, account: &str) {
        if self.setup.is_none() {
            self.account_sheet(account, true);
        }
    }

    pub fn setup_found(&mut self, setup: u64, server: Option<String>) {
        if let Some(sheet) = self.setup.as_mut().filter(|sheet| sheet.id() == setup) {
            sheet.found(&mut self.ui, server);
        }
    }

    pub fn setup_signed(
        &mut self,
        setup: u64,
        result: Result<bande_dav::FoundAddressBooks, bande_dav::Error>,
    ) -> Option<Done> {
        let sheet = self.setup.as_mut().filter(|sheet| sheet.id() == setup)?;
        let done = sheet.signed(&mut self.ui, result);
        if done.is_some() {
            self.setup = None;
        }
        done
    }

    fn ask_remove_account(&mut self, account: &str) {
        let Some(found) = self.library.account(account) else {
            return;
        };
        let title = tr!("Remove “{name}”?", name = found.name.clone());
        let intents = Rc::clone(&self.intents);
        let action = format!("remove-account:{account}");
        self.ui.present_sheet(
            Alert::new(
                &title,
                &tr!("Its contacts are removed from this computer and stay on the server."),
                move |chosen| {
                    if chosen == "remove" {
                        push(&intents, Intent::Confirm(action.clone()));
                    }
                },
            )
            .cancel(&tr!("Cancel"), "cancel")
            .destructive(&tr!("Remove"), "remove"),
        );
    }

    fn show_failure(&mut self, account: &str) {
        let Some((_, error)) = self
            .status
            .failed
            .iter()
            .find(|(failed, _)| failed == account)
            .cloned()
        else {
            return;
        };
        let name = self
            .library
            .account(account)
            .map(|account| account.name.clone())
            .unwrap_or_default();
        let intents = Rc::clone(&self.intents);
        let account = account.to_owned();
        self.ui.present_sheet(
            Alert::new(
                &tr!("“{name}” could not be synced", name = name),
                &error,
                move |action| push(&intents, Intent::Confirm(format!("{action}:{account}"))),
            )
            .other(&tr!("Server Settings…"), "settings")
            .cancel(&tr!("Close"), "close")
            .primary(&tr!("Try Again"), "retry"),
        );
    }

    /// Takes the setup sheet's result when it finished on its own.
    pub fn take_done(&mut self) -> Option<Done> {
        let setup = self.setup.as_mut()?;
        let (job, done, finished) = setup.update(&mut self.ui);
        if let Some(job) = job {
            self.requests.push(Request::Run(job));
        }
        if finished {
            self.setup = None;
        }
        done
    }

    pub fn command(&mut self, command: &str) {
        match command {
            NEW_CONTACT if self.editor.is_none() => self.new_contact(Contact::default()),
            EDIT_CONTACT => self.edit_selected(),
            DELETE_CONTACT => {
                if let Some(found) = self
                    .selected_found()
                    .filter(|found| self.writable(found.book))
                {
                    self.delete(found);
                }
            }
            IMPORT => self.requests.push(Request::ChooseImport),
            EXPORT => {
                let text = self.export_text(false);
                self.requests.push(Request::ChooseExport(text));
            }
            ADD_ACCOUNT => self.add_account(),
            REFRESH => self.requests.push(Request::Sync),
            MERGE => self.ask_merge(),
            FIND => self.ui.set_focus(self.search),
            UNDO if self.editor.is_none() => self.undo(),
            REDO if self.editor.is_none() => self.redo(),
            _ => {}
        }
    }

    fn jump_to(&mut self, letter: &str) {
        let Some(index) = self.shown.iter().position(|shown| shown.letter == letter) else {
            return;
        };
        let key = self.shown[index].key.clone();
        if let Some(list) = self.ui.widget_as::<Table>(self.list) {
            list.scroll_to_row(index);
            list.select(std::slice::from_ref(&key));
        }
        self.selected = Some(key);
        if self.editor.is_none() {
            self.refresh();
        }
    }

    fn confirm(&mut self, action: &str) {
        match action.split_once(':') {
            _ if action == "merge" => self.merge(),
            Some(("remove-account", account)) => {
                self.library.remove_account(account);
                self.requests
                    .push(Request::Run(Job::Forget(account.to_owned())));
                self.status.failed.retain(|(failed, _)| failed != account);
                self.scope = ALL.to_owned();
                self.refresh();
            }
            Some(("retry", _)) => self.requests.push(Request::Sync),
            Some(("settings", account)) => self.account_sheet(account, false),
            _ => {}
        }
    }

    fn editor_signal(&mut self, signal: Signal) {
        match signal {
            Signal::Done => self.save_editor(),
            Signal::Cancel => {
                self.editor = None;
                self.refresh();
            }
            Signal::ChoosePhoto => self.requests.push(Request::ChoosePhoto),
            Signal::RemovePhoto => {
                if let Some(mut editor) = self.editor.take() {
                    let contact = editor.read(&self.ui);
                    editor.photo = None;
                    let contact = Contact {
                        photo: None,
                        ..contact
                    };
                    self.open_editor(editor, &contact);
                }
            }
            Signal::Add(add) => {
                if let Some(mut editor) = self.editor.take() {
                    let mut contact = editor.read(&self.ui);
                    Editor::with_added(&mut contact, add);
                    self.open_editor(editor, &contact);
                }
            }
        }
    }

    /// Handles what the user did since the last call; says whether anything changed.
    pub fn pump(&mut self) -> bool {
        let signals: Vec<Signal> = self.signals.borrow_mut().drain(..).collect();
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        if signals.is_empty() && intents.is_empty() && self.setup.is_none() {
            return false;
        }
        for signal in signals {
            self.editor_signal(signal);
        }
        for intent in intents {
            match intent {
                Intent::Scope(scope) if scope.starts_with("account:") => {
                    if let Some(account) = scope.strip_prefix("account:") {
                        self.account_sheet(account, false);
                    }
                    self.refresh();
                }
                Intent::Scope(scope) => {
                    if self.scope != scope {
                        self.scope = scope;
                        self.refresh();
                    }
                }
                Intent::Select(key) => {
                    if self.selected.as_ref() != Some(&key) {
                        self.selected = Some(key);
                        if self.editor.is_none() {
                            self.refresh();
                        }
                    }
                }
                Intent::Activate => self.edit_selected(),
                Intent::Search(text) => {
                    self.query = text;
                    self.refresh();
                }
                Intent::Letter(letter) => self.jump_to(&letter),
                Intent::Command(command) => self.command(command),
                Intent::Import(true) => self.import(),
                Intent::Import(false) => {
                    self.pending_import.clear();
                    self.import_picker = None;
                }
                Intent::Open(url) => self.requests.push(Request::Open(url)),
                Intent::Row(action, key) => {
                    self.selected = Some(key);
                    match action.as_str() {
                        "edit" => self.edit_selected(),
                        "delete" => self.command(DELETE_CONTACT),
                        "export" => {
                            let text = self.export_text(true);
                            self.requests.push(Request::ChooseExport(text));
                        }
                        _ => {}
                    }
                    self.refresh();
                }
                Intent::Sidebar(action, _) => match action.split_once(':') {
                    Some(("settings", account)) => self.account_sheet(account, false),
                    Some(("remove", account)) => self.ask_remove_account(account),
                    _ => {}
                },
                Intent::Confirm(action) => self.confirm(&action),
                Intent::Failure(account) => self.show_failure(&account),
            }
        }
        true
    }
}

fn merge_into(merged: &mut Contact, other: Contact) {
    let fill = |mine: &mut String, theirs: String| {
        if mine.trim().is_empty() {
            *mine = theirs;
        }
    };
    fill(&mut merged.name.given, other.name.given);
    fill(&mut merged.name.family, other.name.family);
    fill(&mut merged.organisation, other.organisation);
    fill(&mut merged.title, other.title);
    for email in other.emails {
        if !merged
            .emails
            .iter()
            .any(|mine| mine.value.eq_ignore_ascii_case(&email.value))
        {
            merged.emails.push(email);
        }
    }
    let digits = |text: &str| -> String { text.chars().filter(char::is_ascii_digit).collect() };
    for phone in other.phones {
        if !merged
            .phones
            .iter()
            .any(|mine| digits(&mine.value) == digits(&phone.value))
        {
            merged.phones.push(phone);
        }
    }
    for address in other.addresses {
        if !merged.addresses.contains(&address) {
            merged.addresses.push(address);
        }
    }
    if merged.birthday.is_none() {
        merged.birthday = other.birthday;
    }
    if merged.photo.is_none() {
        merged.photo = other.photo;
    }
    match (merged.note.trim().is_empty(), other.note.trim().is_empty()) {
        (true, _) => merged.note = other.note,
        (false, false) if merged.note != other.note => {
            merged.note = format!("{}\n\n{}", merged.note, other.note)
        }
        _ => {}
    }
    for category in other.categories {
        if !merged.categories.contains(&category) {
            merged.categories.push(category);
        }
    }
}
