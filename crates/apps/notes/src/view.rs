use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use accesskit::TreeUpdate;
use bande_app::View;
use bande_inbox::Outbox;
use bande_keyring::Keyring;
use bande_l10n::tr;
use bande_mail_core::accounts::{self, Account, AccountId};
use bande_mail_core::discover::Settings;
use bande_mail_core::net::Network;
use bande_mail_core::setup::{Outcome, check, find};
use bande_mail_core::sheet::{AccountSheet, Action, Field, Mode, Shared, SheetState};
use bande_mail_core::{KEYRING, keyring_key, keyring_label};
use bande_rich_text::heading;
use bande_toolkit::ui::{
    Alert, BUTTON_HEIGHT, Button, Cell, Column, Context, Event, FreeLayout, Heading, Key, Label,
    ListMarker, Modifiers, ParagraphAttributes, RichText, Row, SearchField, TEXT_FIELD_HEIGHT,
    Table, TextAttributes, TextView, Ui, Widget, WidgetId,
};
use bande_toolkit::{Appearance, Rect, Scene, TextSystem, Window, WindowEvent};
use zeroize::Zeroizing;

use crate::store::{Deleted, Folder, FolderId, LOCAL, NoteId, Store, Summary};
use crate::worker::Event as WorkerEvent;

pub const WINDOW_SIZE: (u32, u32) = (980, 620);
const ICON: &[u8] = include_bytes!("../data/org.bande.Notes.svg");
const SIDEBAR_WIDTH: f32 = 200.0;
const LIST_WIDTH: f32 = 320.0;
const MARGIN: f32 = 8.0;
const BANNER_HEIGHT: f32 = 44.0;
const SETTLE: Duration = Duration::from_secs(1);
const MAX_UNDO: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Signal {
    Folder(String),
    Note(String),
    Search(String),
    EditAnyway,
    FocusEditor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Sync(AccountId),
    Accounts(Option<(AccountId, String)>),
}

struct Setup {
    state: Shared,
    mode: Mode,
    password_field: WidgetId,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

pub struct NotesView {
    ui: Ui,
    host: WidgetId,
    sidebar: WidgetId,
    search: WidgetId,
    list: WidgetId,
    editor: WidgetId,
    empty: WidgetId,
    banner: WidgetId,
    edit_anyway: WidgetId,
    signals: Rc<RefCell<Vec<Signal>>>,
    store: Store,
    accounts: Vec<Account>,
    status: Vec<(AccountId, String)>,
    folders: Vec<Folder>,
    folder: FolderId,
    query: String,
    summaries: Vec<Summary>,
    note: Option<NoteId>,
    locked: bool,
    loaded: Option<(NoteId, i64)>,
    seen: u64,
    settle_at: Option<Instant>,
    undo: Vec<Deleted>,
    setup: Option<Setup>,
    outbox: Outbox<WorkerEvent>,
    network: Network,
    requests: Vec<Request>,
}

fn push(signals: &Rc<RefCell<Vec<Signal>>>, signal: Signal) {
    signals.borrow_mut().push(signal);
}

fn folder_key(id: FolderId) -> String {
    format!("folder:{id}")
}

fn note_key(id: NoteId) -> String {
    format!("note:{id}")
}

fn key_id(key: &str, prefix: &str) -> Option<i64> {
    key.strip_prefix(prefix)?.parse().ok()
}

fn nested(folders: &[&Folder], depth: usize, at: &mut usize) -> Vec<Row> {
    let mut rows = Vec::new();
    while let Some(folder) = folders.get(*at).filter(|folder| folder.depth >= depth) {
        *at += 1;
        let name = match folder.account {
            LOCAL => tr!("Notes"),
            _ if folder.depth == 0 => tr!("Notes"),
            _ => folder.name.clone(),
        };
        let children = nested(folders, folder.depth + 1, at);
        rows.push(Row::new(folder_key(folder.id), vec![Cell::new(name)]).children(children));
    }
    rows
}

impl NotesView {
    pub fn new(
        appearance: Appearance,
        store: Store,
        accounts: Vec<Account>,
        outbox: Outbox<WorkerEvent>,
    ) -> Self {
        let signals = Rc::new(RefCell::new(Vec::new()));
        let mut host = FreeLayout::new();
        let host_id = Widget::id(&host);
        let (folder_signals, note_signals, open_signals, search_signals, edit_signals) = (
            Rc::clone(&signals),
            Rc::clone(&signals),
            Rc::clone(&signals),
            Rc::clone(&signals),
            Rc::clone(&signals),
        );
        let sidebar = host.insert(
            Table::new(vec![Column::new("", SIDEBAR_WIDTH).flexible()])
                .without_header()
                .sidebar()
                .label(&tr!("Folders"))
                .on_selection(move |keys| {
                    if let Some(key) = keys.first() {
                        push(&folder_signals, Signal::Folder(key.clone()));
                    }
                }),
            Rect::default(),
        );
        let search = host.insert(
            SearchField::new(&tr!("Search"), "", move |text| {
                push(&search_signals, Signal::Search(text.to_owned()));
            }),
            Rect::default(),
        );
        let list = host.insert(
            Table::new(vec![
                Column::new(&tr!("Title"), 130.0),
                Column::new(&tr!("Text"), 110.0).flexible(),
                Column::new(&tr!("Modified"), 80.0),
            ])
            .without_header()
            .label(&tr!("Notes"))
            .on_selection(move |keys| {
                if let Some(key) = keys.first() {
                    push(&note_signals, Signal::Note(key.clone()));
                }
            })
            .on_activate(move |_| push(&open_signals, Signal::FocusEditor)),
            Rect::default(),
        );
        let editor = host.insert(
            TextView::new(&tr!("Start writing"), "", |_| {}).borderless(),
            Rect::default(),
        );
        let empty = host.insert(Label::new("").secondary(), Rect::default());
        let banner = host.insert(Label::new("").secondary(), Rect::default());
        let edit_anyway = host.insert(
            Button::new(&tr!("Edit Anyway"), move || {
                push(&edit_signals, Signal::EditAnyway);
            }),
            Rect::default(),
        );
        let mut ui = Ui::new(
            Box::new(host),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        let mut view = Self {
            ui,
            host: host_id,
            sidebar,
            search,
            list,
            editor,
            empty,
            banner,
            edit_anyway,
            signals,
            folder: 0,
            store,
            accounts,
            status: Vec::new(),
            folders: Vec::new(),
            query: String::new(),
            summaries: Vec::new(),
            note: None,
            locked: false,
            loaded: None,
            seen: 0,
            settle_at: None,
            undo: Vec::new(),
            setup: None,
            outbox,
            network: Network::default(),
            requests: Vec::new(),
        };
        view.reload();
        if let Some(local) = view.folders.first().map(|folder| folder.id) {
            view.show_folder(local);
        }
        view.ui.set_focus(list);
        view
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn folders(&self) -> Vec<(FolderId, String)> {
        self.folders
            .iter()
            .map(|folder| (folder.id, self.folder_title(folder)))
            .collect()
    }

    pub fn current_folder(&self) -> FolderId {
        self.folder
    }

    pub fn has_note(&self) -> bool {
        self.note.is_some()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    fn account_name(&self, account: AccountId) -> String {
        match account {
            LOCAL => tr!("On This Computer"),
            _ => self
                .accounts
                .iter()
                .find(|known| known.id == account)
                .map(|known| known.email.clone())
                .unwrap_or_default(),
        }
    }

    fn folder_title(&self, folder: &Folder) -> String {
        let account = self.account_name(folder.account);
        match (folder.account, folder.depth) {
            (LOCAL, _) | (_, 0) => tr!("Notes ({account})", account = account),
            _ => format!("{} ({account})", folder.name),
        }
    }

    pub fn set_accounts(&mut self, accounts: Vec<Account>) {
        self.accounts = accounts;
        self.reload();
    }

    pub fn set_status(&mut self, account: AccountId, status: Option<String>) {
        self.status.retain(|(known, _)| *known != account);
        if let Some(status) = status {
            self.status.push((account, status));
        }
        self.refresh_sidebar();
    }

    fn refresh_sidebar(&mut self) {
        let mut rows = Vec::new();
        let mut headings = Vec::new();
        let mut line = 0;
        let mut accounts = self
            .folders
            .iter()
            .map(|folder| folder.account)
            .collect::<Vec<_>>();
        accounts.dedup();
        for account in accounts {
            let folders = self
                .folders
                .iter()
                .filter(|folder| folder.account == account)
                .collect::<Vec<_>>();
            let title = match self.status.iter().find(|(known, _)| *known == account) {
                Some((_, status)) => tr!(
                    "{account} — {status}",
                    account = self.account_name(account),
                    status = status.clone()
                ),
                None => self.account_name(account),
            };
            headings.push(Heading::new(line, title));
            line += folders.len();
            rows.extend(nested(&folders, 0, &mut 0));
        }
        let keys = self
            .folders
            .iter()
            .map(|folder| folder_key(folder.id))
            .collect::<Vec<_>>();
        let selected = folder_key(self.folder);
        if let Some(sidebar) = self.ui.widget_as::<Table>(self.sidebar) {
            sidebar.set_rows(rows);
            sidebar.set_headings(headings);
            sidebar.set_expanded_rows(keys);
            sidebar.select(&[selected]);
        }
    }

    pub fn reload(&mut self) {
        match self.store.folders() {
            Ok(folders) => self.folders = folders,
            Err(error) => tracing::warn!("cannot read the folders: {error}"),
        }
        if !self.folders.iter().any(|folder| folder.id == self.folder)
            && let Some(first) = self.folders.first()
        {
            self.folder = first.id;
        }
        self.refresh_sidebar();
        self.refresh_list();
        self.refresh_note();
    }

    fn refresh_list(&mut self) {
        self.summaries = self
            .store
            .summaries(self.folder, &self.query)
            .inspect_err(|error| tracing::warn!("cannot list the notes: {error}"))
            .unwrap_or_default();
        let rows = self
            .summaries
            .iter()
            .map(|summary| {
                let title = match summary.title.as_str() {
                    "" => tr!("New Note"),
                    title => title.to_owned(),
                };
                let modified = UNIX_EPOCH + Duration::from_secs(summary.modified.max(0) as u64);
                Row::new(
                    note_key(summary.id),
                    vec![
                        Cell::new(title),
                        Cell::new(summary.snippet.clone()).muted(),
                        Cell::new(bande_fs::format_list_date(modified)).muted(),
                    ],
                )
            })
            .collect();
        let selected = self.note.map(note_key).into_iter().collect::<Vec<_>>();
        if let Some(list) = self.ui.widget_as::<Table>(self.list) {
            list.set_rows(rows);
            list.select(&selected);
        }
        if self
            .note
            .is_some_and(|note| !self.summaries.iter().any(|summary| summary.id == note))
        {
            self.note = None;
        }
    }

    fn editor(&mut self) -> Option<&mut TextView> {
        self.ui.widget_as::<TextView>(self.editor)
    }

    fn revision(&self) -> u64 {
        self.ui
            .widget_ref::<TextView>(self.editor)
            .map_or(0, TextView::revision)
    }

    fn refresh_note(&mut self) {
        if self.settle_at.is_some() {
            return;
        }
        let stored = self
            .note
            .and_then(|id| self.store.stored(id).ok().flatten());
        let version = stored
            .as_ref()
            .map(|stored| (stored.id, stored.note.modified));
        if version.is_none() || version != self.loaded {
            let (rich, locked) = match &stored {
                Some(stored) => (stored.note.rich.clone(), stored.note.extras),
                None => (RichText::default(), false),
            };
            self.loaded = version;
            self.locked = locked;
            if let Some(editor) = self.editor() {
                editor.set_rich_text(&rich);
                editor.set_read_only(locked);
            }
        }
        let locked = self.locked;
        self.seen = self.revision();
        let banner = match locked {
            true => tr!(
                "This note has pictures or attachments that Notes cannot show. Editing it here keeps only its text."
            ),
            false => String::new(),
        };
        let empty = match (self.note, self.summaries.is_empty(), self.query.is_empty()) {
            (Some(_), _, _) => String::new(),
            (None, true, true) => tr!("No Notes"),
            (None, true, false) => tr!("No notes match “{search}”", search = self.query.clone()),
            (None, false, _) => tr!("No Note Selected"),
        };
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            if let Some(label) = host.replace(self.banner, Label::new(&banner).secondary()) {
                self.banner = label;
            }
            if let Some(label) = host.replace(self.empty, Label::heading(&empty)) {
                self.empty = label;
            }
        }
    }

    fn leave_note(&mut self) {
        self.persist();
        let Some(id) = self.note else {
            return;
        };
        let empty = self
            .ui
            .widget_ref::<TextView>(self.editor)
            .is_some_and(|editor| editor.text().trim().is_empty());
        let fresh = self
            .store
            .stored(id)
            .ok()
            .flatten()
            .is_some_and(|stored| stored.uid.is_none());
        if empty && fresh && !self.locked {
            match self.store.delete(id) {
                Ok(_) => self.changed_in(self.folder),
                Err(error) => tracing::warn!("cannot remove the empty note: {error}"),
            }
            self.note = None;
            self.refresh_list();
        }
    }

    fn show_folder(&mut self, folder: FolderId) {
        self.leave_note();
        self.folder = folder;
        self.note = None;
        self.refresh_list();
        self.note = self.summaries.first().map(|summary| summary.id);
        self.refresh_list();
        self.refresh_note();
    }

    fn show_note(&mut self, note: Option<NoteId>) {
        if note == self.note {
            return;
        }
        self.leave_note();
        self.note = note;
        self.refresh_note();
    }

    fn account_of(&self, folder: FolderId) -> AccountId {
        self.folders
            .iter()
            .find(|known| known.id == folder)
            .map_or(LOCAL, |known| known.account)
    }

    fn changed_in(&mut self, folder: FolderId) {
        let account = self.account_of(folder);
        if account != LOCAL {
            self.requests.push(Request::Sync(account));
        }
    }

    fn persist(&mut self) {
        self.settle_at = None;
        let Some(id) = self.note else {
            return;
        };
        let revision = self.revision();
        if revision == self.seen || self.locked {
            return;
        }
        self.seen = revision;
        let Some(rich) = self
            .ui
            .widget_ref::<TextView>(self.editor)
            .map(TextView::rich_text)
        else {
            return;
        };
        let modified = now();
        if let Err(error) = self.store.edit(id, &rich, modified) {
            tracing::warn!("cannot keep the note: {error}");
            return;
        }
        self.loaded = Some((id, modified));
        self.changed_in(self.folder);
        self.refresh_list();
    }

    pub fn flush(&mut self) {
        self.persist();
    }

    pub fn new_note(&mut self) {
        self.leave_note();
        if !self.query.is_empty() {
            self.query.clear();
            let signals = Rc::clone(&self.signals);
            let cleared = SearchField::new(&tr!("Search"), "", move |text| {
                push(&signals, Signal::Search(text.to_owned()));
            });
            if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host)
                && let Some(search) = host.replace(self.search, cleared)
            {
                self.search = search;
            }
        }
        match self.store.create(self.folder, now()) {
            Ok(id) => {
                self.note = Some(id);
                self.refresh_list();
                self.refresh_note();
                self.ui.set_focus(self.editor);
            }
            Err(error) => tracing::warn!("cannot add a note: {error}"),
        }
    }

    pub fn delete_note(&mut self) {
        self.settle_at = None;
        let Some(id) = self.note else {
            return;
        };
        let position = self.summaries.iter().position(|summary| summary.id == id);
        match self.store.delete(id) {
            Ok(Some(deleted)) => {
                self.undo.push(deleted);
                if self.undo.len() > MAX_UNDO {
                    self.undo.remove(0);
                }
            }
            Ok(None) => {}
            Err(error) => return tracing::warn!("cannot delete the note: {error}"),
        }
        self.changed_in(self.folder);
        self.note = None;
        self.refresh_list();
        self.note = position
            .and_then(|position| self.summaries.get(position).or(self.summaries.last()))
            .map(|summary| summary.id);
        self.refresh_list();
        self.refresh_note();
    }

    pub fn undo_delete(&mut self) {
        let Some(deleted) = self.undo.pop() else {
            return;
        };
        match self.store.restore(&deleted) {
            Ok(id) => {
                let folder = self
                    .store
                    .stored(id)
                    .ok()
                    .flatten()
                    .map_or(self.folder, |stored| stored.folder);
                self.changed_in(folder);
                self.folder = folder;
                self.note = Some(id);
                self.reload();
            }
            Err(error) => tracing::warn!("cannot restore the note: {error}"),
        }
    }

    pub fn move_note(&mut self, target: FolderId) {
        self.persist();
        let Some(id) = self.note else {
            return;
        };
        let source = self.folder;
        if let Err(error) = self.store.move_note(id, target) {
            return tracing::warn!("cannot move the note: {error}");
        }
        self.changed_in(source);
        self.changed_in(target);
        self.folder = target;
        self.reload();
    }

    pub fn focus_search(&mut self) {
        self.ui.set_focus(self.search);
    }

    pub fn style(&mut self, command: &str) {
        if self.note.is_none() || self.locked {
            return;
        }
        let Some(editor) = self.editor() else {
            return;
        };
        let attributes = editor.attributes();
        match command {
            "bold" => editor.change_attributes(|span| span.bold = !attributes.bold),
            "italic" => editor.change_attributes(|span| span.italic = !attributes.italic),
            "underline" => editor.change_attributes(|span| span.underline = !attributes.underline),
            "strikethrough" => {
                editor.change_attributes(|span| span.strikethrough = !attributes.strikethrough);
            }
            "list:bullet" | "list:number" | "list:none" => {
                let list = match command {
                    "list:bullet" => Some(ListMarker::Bullet),
                    "list:number" => Some(ListMarker::Number),
                    _ => None,
                };
                editor.change_paragraphs(|paragraph| {
                    *paragraph = ParagraphAttributes {
                        list,
                        ..ParagraphAttributes::default()
                    };
                });
            }
            level => {
                let Some(level) = level
                    .strip_prefix("heading:")
                    .and_then(|level| level.parse::<usize>().ok())
                else {
                    return;
                };
                let styled = match level {
                    0 => TextAttributes::default(),
                    level => heading(level),
                };
                let selection = editor.selection();
                let text = editor.text();
                let start = text[..selection.start].rfind('\n').map_or(0, |at| at + 1);
                let end = text[selection.end..]
                    .find('\n')
                    .map_or(text.len(), |at| selection.end + at);
                editor.select(start..end);
                editor.change_attributes(|span| {
                    span.bold = styled.bold;
                    span.size = styled.size;
                });
                editor.select(selection);
            }
        }
        self.settle_at = Some(Instant::now() + SETTLE);
    }

    pub fn attributes(&self) -> (TextAttributes, ParagraphAttributes) {
        self.ui
            .widget_ref::<TextView>(self.editor)
            .filter(|_| self.note.is_some())
            .map(|editor| (editor.attributes(), editor.paragraph_attributes()))
            .unwrap_or_default()
    }

    pub fn present_setup(&mut self, mode: Mode) {
        let state: Shared = Rc::new(RefCell::new(SheetState::default()));
        if let Some(previous) = self.setup.take() {
            {
                let previous = previous.state.borrow();
                let mut fresh = state.borrow_mut();
                fresh.values = previous.values.clone();
                fresh.password = previous.password.clone();
            }
            previous.state.borrow_mut().closed = true;
        }
        let sheet = AccountSheet::new(Rc::clone(&state), mode.clone(), ICON, false);
        let password_field = sheet.password_field();
        self.ui.present_sheet(sheet);
        self.setup = Some(Setup {
            state,
            mode,
            password_field,
        });
    }

    fn close_setup(&mut self) {
        if let Some(setup) = self.setup.take() {
            setup.state.borrow_mut().closed = true;
        }
    }

    fn submit_password(&mut self, field: WidgetId) {
        let previous = self.ui.focus();
        self.ui.set_focus(field);
        self.ui.handle(&Event::Key {
            key: Key::Enter,
            modifiers: Modifiers::default(),
        });
        match previous {
            Some(previous) => self.ui.set_focus(previous),
            None => self.ui.clear_focus(),
        }
    }

    fn sheet_action(&mut self) {
        let Some(setup) = &self.setup else {
            return;
        };
        let (state, mode, field) = (
            Rc::clone(&setup.state),
            setup.mode.clone(),
            setup.password_field,
        );
        let Some(action) = state.borrow_mut().action.take() else {
            return;
        };
        if state.borrow().busy && action != Action::Cancel {
            return;
        }
        if action != Action::Cancel && mode != Mode::Kind {
            self.submit_password(field);
            state.borrow_mut().action = None;
        }
        match (action, mode) {
            (Action::Cancel, _) => self.close_setup(),
            (Action::Primary, Mode::Kind) => self.present_setup(Mode::Address),
            (Action::Primary, Mode::Address) => self.find_servers(&state),
            (Action::Primary, Mode::Servers) => self.sign_in(&state),
            (Action::Primary, Mode::Password(email)) => self.renew_password(&state, &email),
            (Action::Other, Mode::Address) => self.present_setup(Mode::Servers),
            (Action::Other, Mode::Servers | Mode::Password(_)) => self.present_setup(Mode::Address),
            (Action::Other, Mode::Kind) => {}
        }
    }

    fn start_check(&self, state: &Shared, status: String) -> bool {
        let problem = {
            let state = state.borrow();
            let email = state.value(Field::Email);
            if state.value(Field::Name).is_empty() || bande_discovery::domain_of(&email).is_none() {
                Some(tr!("Enter your name and email address."))
            } else if state.password.is_empty() {
                Some(tr!("Enter your password."))
            } else {
                None
            }
        };
        let mut state = state.borrow_mut();
        match problem {
            Some(problem) => {
                state.status = Some((problem, true));
                false
            }
            None => {
                state.busy = true;
                state.status = Some((status, false));
                true
            }
        }
    }

    fn find_servers(&mut self, state: &Shared) {
        if !self.start_check(state, tr!("Looking up the servers…")) {
            return;
        }
        let email = state.borrow().value(Field::Email);
        let (network, outbox) = (self.network.clone(), self.outbox.clone());
        std::thread::spawn(move || {
            outbox.send(WorkerEvent::Found(Box::new(find(&email, &network))));
        });
    }

    pub fn servers_found(&mut self, found: bande_mail_core::setup::Found) {
        if self.setup.as_ref().map(|setup| &setup.mode) != Some(&Mode::Address) {
            return;
        }
        if let Some((settings, _)) = &found.settings
            && let Some(setup) = &self.setup
        {
            setup.state.borrow_mut().fill(settings);
        }
        self.present_setup(Mode::Servers);
        if let Some(setup) = &self.setup {
            setup.state.borrow_mut().status = Some((found.message(), false));
        }
    }

    fn sign_in(&mut self, state: &Shared) {
        let Some(servers) = state.borrow().servers() else {
            state.borrow_mut().status = Some((tr!("Enter both server names."), true));
            return;
        };
        if !self.start_check(state, tr!("Checking the account…")) {
            return;
        }
        let password = state.borrow().password.to_string();
        let (network, outbox) = (self.network.clone(), self.outbox.clone());
        std::thread::spawn(move || {
            outbox.send(WorkerEvent::Checked(Box::new(check(
                &servers, &password, &network,
            ))));
        });
    }

    pub fn setup_checked(&mut self, outcome: Outcome) {
        let Some(setup) = &self.setup else {
            return;
        };
        let state = Rc::clone(&setup.state);
        state.borrow_mut().busy = false;
        match outcome {
            Outcome::Ready(settings) => self.add_account(&state, settings),
            Outcome::Failed(_, problem) => {
                state.borrow_mut().status = Some((problem.message(), true));
            }
        }
    }

    fn add_account(&mut self, state: &Shared, settings: Settings) {
        let (name, email, password) = {
            let state = state.borrow();
            (
                state.value(Field::Name),
                state.value(Field::Email),
                Zeroizing::new(state.password.to_string()),
            )
        };
        let account = Account {
            id: 0,
            name,
            email,
            user: settings.user,
            imap: settings.imap,
            smtp: settings.smtp,
            signature: String::new(),
            enabled: true,
        };
        let added = accounts::open(&accounts::store_path()).and_then(|mail| {
            let id = match accounts::list(&mail)?
                .into_iter()
                .find(|known| known.email.eq_ignore_ascii_case(&account.email))
            {
                Some(known) => known.id,
                None => accounts::add(&mail, &account)?,
            };
            accounts::set_notes(&mail, id, true)?;
            Ok(id)
        });
        match added {
            Ok(id) => {
                let (email, host, saved) = (
                    account.email.clone(),
                    account.imap.host.clone(),
                    password.clone(),
                );
                std::thread::spawn(move || {
                    if let Err(error) = Keyring::open(KEYRING).and_then(|keyring| {
                        keyring.save(&keyring_key(&email, &host), &keyring_label(&email), &saved)
                    }) {
                        tracing::warn!("cannot save the password: {error}");
                    }
                });
                self.requests
                    .push(Request::Accounts(Some((id, password.to_string()))));
                self.close_setup();
            }
            Err(error) => state.borrow_mut().status = Some((error.to_string(), true)),
        }
    }

    fn renew_password(&mut self, state: &Shared, email: &str) {
        let password = Zeroizing::new(state.borrow().password.to_string());
        if password.is_empty() {
            state.borrow_mut().status = Some((tr!("Enter your password."), true));
            return;
        }
        let Some(account) = self
            .accounts
            .iter()
            .find(|known| known.email == email)
            .cloned()
        else {
            return self.close_setup();
        };
        let saved = password.clone();
        std::thread::spawn(move || {
            if let Err(error) = Keyring::open(KEYRING).and_then(|keyring| {
                keyring.save(
                    &keyring_key(&account.email, &account.imap.host),
                    &keyring_label(&account.email),
                    &saved,
                )
            }) {
                tracing::warn!("cannot save the password: {error}");
            }
        });
        self.requests
            .push(Request::Accounts(Some((account.id, password.to_string()))));
        self.close_setup();
    }

    pub fn login_failed(&mut self, account: AccountId) {
        if self.ui.has_sheet() {
            return;
        }
        if let Some(email) = self
            .accounts
            .iter()
            .find(|known| known.id == account)
            .map(|known| known.email.clone())
        {
            self.present_setup(Mode::Password(email));
        }
    }

    pub fn tell(&mut self, title: &str, message: &str) {
        if self.ui.has_sheet() {
            return;
        }
        let alert = Alert::new(title, message, |_| {}).primary(&tr!("OK"), "ok");
        self.ui.present_sheet(alert);
    }

    fn take_signals(&mut self) -> bool {
        let signals = std::mem::take(&mut *self.signals.borrow_mut());
        let any = !signals.is_empty();
        for signal in signals {
            match signal {
                Signal::Folder(key) => {
                    if let Some(folder) = key_id(&key, "folder:").filter(|id| *id != self.folder) {
                        self.show_folder(folder);
                    }
                }
                Signal::Note(key) => self.show_note(key_id(&key, "note:")),
                Signal::Search(text) => {
                    self.persist();
                    self.query = text;
                    self.refresh_list();
                    if self.note.is_none() {
                        self.note = self.summaries.first().map(|summary| summary.id);
                        self.refresh_list();
                    }
                    self.refresh_note();
                }
                Signal::EditAnyway => {
                    self.locked = false;
                    if let Some(editor) = self.editor() {
                        editor.set_read_only(false);
                    }
                    if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host)
                        && let Some(label) = host.replace(self.banner, Label::new(""))
                    {
                        self.banner = label;
                    }
                    self.ui.set_focus(self.editor);
                }
                Signal::FocusEditor => {
                    if self.note.is_some() {
                        self.ui.set_focus(self.editor);
                    }
                }
            }
        }
        any
    }

    fn layout(&mut self, width: f32, height: f32) {
        let list_x = SIDEBAR_WIDTH;
        let list_width = LIST_WIDTH.min((width - SIDEBAR_WIDTH).max(0.0));
        let editor_x = list_x + list_width;
        let editor_width = (width - editor_x).max(0.0);
        let search_height = TEXT_FIELD_HEIGHT + 2.0 * MARGIN;
        let banner = match self.locked {
            true => BANNER_HEIGHT,
            false => 0.0,
        };
        let shown = self.note.is_some();
        let mut frames = vec![
            (self.sidebar, Rect::new(0.0, 0.0, SIDEBAR_WIDTH, height)),
            (
                self.search,
                Rect::new(
                    list_x + MARGIN,
                    MARGIN,
                    (list_width - 2.0 * MARGIN).max(0.0),
                    TEXT_FIELD_HEIGHT,
                ),
            ),
            (
                self.list,
                Rect::new(
                    list_x,
                    search_height,
                    list_width,
                    (height - search_height).max(0.0),
                ),
            ),
        ];
        let button_width = 120.0;
        frames.push((
            self.banner,
            match self.locked {
                true => Rect::new(
                    editor_x + 2.0 * MARGIN,
                    MARGIN,
                    (editor_width - button_width - 5.0 * MARGIN).max(0.0),
                    BANNER_HEIGHT - 2.0 * MARGIN,
                ),
                false => Rect::default(),
            },
        ));
        frames.push((
            self.edit_anyway,
            match self.locked {
                true => Rect::new(
                    editor_x + editor_width - button_width - 2.0 * MARGIN,
                    (BANNER_HEIGHT - BUTTON_HEIGHT) / 2.0,
                    button_width,
                    BUTTON_HEIGHT,
                ),
                false => Rect::default(),
            },
        ));
        frames.push((
            self.editor,
            match shown {
                true => Rect::new(
                    editor_x + MARGIN,
                    banner,
                    (editor_width - MARGIN).max(0.0),
                    (height - banner).max(0.0),
                ),
                false => Rect::default(),
            },
        ));
        frames.push((
            self.empty,
            match shown {
                true => Rect::default(),
                false => Rect::new(
                    editor_x + 3.0 * MARGIN,
                    height / 2.0 - 16.0,
                    (editor_width - 6.0 * MARGIN).max(0.0),
                    32.0,
                ),
            },
        ));
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            for (id, frame) in frames {
                host.set_frame(id, frame);
            }
        }
    }

    fn key(&mut self, event: &Event) -> Option<bool> {
        let Event::Key { key, modifiers } = event else {
            return None;
        };
        if self.ui.has_sheet() {
            return None;
        }
        let focus = self.ui.focus();
        let plain = *modifiers == Modifiers::default();
        let control = *modifiers
            == Modifiers {
                control: true,
                ..Modifiers::default()
            };
        match key {
            Key::Delete | Key::Backspace if plain && focus == Some(self.list) => {
                self.delete_note();
                Some(true)
            }
            Key::Character(character)
                if control
                    && *character == 'z'
                    && focus != Some(self.editor)
                    && self.can_undo() =>
            {
                self.undo_delete();
                Some(true)
            }
            Key::Escape if plain && focus == Some(self.editor) => {
                self.persist();
                self.ui.set_focus(self.list);
                Some(true)
            }
            _ => None,
        }
    }

    fn after_event(&mut self) {
        self.sheet_action();
        if self.note.is_some()
            && !self.locked
            && self.revision() != self.seen
            && self.settle_at.is_none()
        {
            self.settle_at = Some(Instant::now() + SETTLE);
        }
    }
}

impl View for NotesView {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.layout(width as f32, height as f32);
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        if let WindowEvent::Input(input) = &event
            && let Some(handled) = self.key(input)
        {
            window.request_redraw();
            return handled;
        }
        let handled = window
            .drive(&mut self.ui, event)
            .inspect_err(|error| tracing::warn!("{error}"))
            .is_ok_and(|unhandled| unhandled.is_none());
        let signalled = self.take_signals();
        self.after_event();
        if signalled {
            window.request_redraw();
        }
        handled || signalled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
        match active {
            true => {
                self.outbox.send(WorkerEvent::AccountsChanged);
            }
            false => self.persist(),
        }
    }

    fn deadline(&self) -> Option<Instant> {
        [self.settle_at, self.ui.deadline()]
            .into_iter()
            .flatten()
            .min()
    }

    fn tick(&mut self, _window: &mut Window, now: Instant) -> bool {
        if self.settle_at.is_some_and(|at| at <= now) {
            self.persist();
        }
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}
