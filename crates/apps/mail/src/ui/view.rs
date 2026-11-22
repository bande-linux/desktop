use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bande_l10n::{Text, text, tr, trn};

use crate::compose::{Draft, Original, Outgoing, forward, reply, signed};
use crate::discover::Settings;
use crate::error::Result;
use crate::imap::{Flag, Role};
use crate::message::{Address, Content, Envelope, parse_addresses};
use crate::net::Network;
use crate::settings::{MailSettings, SortBy};
use crate::setup::{Found, Outcome, check, find};
use crate::store::{Account, AccountId, Mailbox, MailboxId, MessageId, RuleAction, Store, Summary};
use crate::ui::compose_view::attachable;
use crate::ui::format::full_date;
use crate::ui::reader::{Entry, Reading, Respond, Target, Thread};
use crate::ui::reader_pane::{
    ReaderPane, address_menu, attachment_menu, fetch_remote, link_alert, link_menu,
};
use crate::ui::setup_sheet::{AccountSheet, Action, Field, Mode, Shared, SheetState};
use crate::ui::sidebar::{Place, Row, rows};
use crate::ui::symbols::Symbols;
use crate::ui::toolbar::{Signal, Signals, Toolbar};
use crate::worker::{Command, Event as MailEvent, MailboxChange, Mailroom, Status, Worker};
use accesskit::{ActionRequest, TreeUpdate};
use bande_app::{Dropping, FileDrop};
use bande_app_settings::{Stored, settings_folder};
use bande_datetime::DateFormats;
use bande_keyring::Keyring;
use bande_toolkit::ui::{
    Alert, ClipboardRequest, Context, Cursor, Event, FreeLayout, Key, Menu, MenuItem, Modifiers,
    Pointer, PointerButton, ScrollState, TextField, Ui, Widget, WidgetId, opens_menu,
    past_drag_slop, shortcut_pressed,
};
use bande_toolkit::{
    Appearance, Header, Palette, Point, Rect, SHEET_INSET, TextSystem, WINDOW_HEADER_HEIGHT,
    Window, WindowEvent,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use zeroize::Zeroizing;

pub(crate) mod security;

pub const SHORTCUTS: [(&str, &str); 9] = [
    ("compose", "Ctrl+N"),
    ("get-mail", "F5"),
    ("find", "Ctrl+F"),
    ("reply", "Ctrl+R"),
    ("reply-all", "Shift+Ctrl+R"),
    ("forward", "Shift+Ctrl+F"),
    ("archive", "Shift+Ctrl+A"),
    ("delete", "Delete"),
    ("send", "Ctrl+Enter"),
];
pub const SIDEBAR_WIDTH: f32 = 220.0;
pub const LIST_WIDTH: f32 = 340.0;
pub const TOOLBAR_HEIGHT: f32 = WINDOW_HEADER_HEIGHT as f32;
const NOTIFIED_AT_ONCE: usize = 3;
const UNDO_TRIES: u8 = 3;
const MOVED_IN_KEPT: Duration = Duration::from_secs(10 * 60);
const UNDO_LEVELS: usize = 20;
const OPENED_UNREAD: usize = 10;
const FLAG_COLORS: [Text; 7] = [
    text!("Red"),
    text!("Orange"),
    text!("Yellow"),
    text!("Green"),
    text!("Blue"),
    text!("Purple"),
    text!("Gray"),
];
const CORRESPONDENTS_READ: usize = 5000;
const LIST_PAGE: usize = 500;
const LIST_AHEAD: f32 = 2000.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Compose(Box<Outgoing>),
    OpenLink(String),
    OpenAttachment(PathBuf),
    SaveAttachment(String, Vec<u8>),
    Notify {
        summary: String,
        body: String,
        sound: bool,
        message: Option<MessageId>,
    },
    NotSent {
        error: String,
        outgoing: Box<Outgoing>,
    },
    ShowMenu(Menu, Point),
    Clipboard(ClipboardRequest),
    OpenSettings,
    Attach(Vec<PathBuf>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Place(usize),
    Message(MessageId),
    Conversation(MessageId),
    Reader(Target),
    Offline(AccountId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    Sidebar,
    List,
    Reader,
}

#[derive(Debug, Default)]
pub(crate) struct Conversations {
    pub(crate) earlier: HashMap<MessageId, (String, Vec<Summary>)>,
    pub(crate) children: HashSet<MessageId>,
    expanded: HashSet<String>,
}

impl Conversations {
    fn group(&mut self, all: Vec<Summary>) -> Vec<Summary> {
        self.earlier.clear();
        self.children.clear();
        let mut order: Vec<Vec<Summary>> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for summary in all {
            match summary
                .thread
                .clone()
                .and_then(|key| index.get(&key).copied())
            {
                Some(group) => order[group].push(summary),
                None => {
                    if let Some(key) = summary.thread.clone() {
                        index.insert(key, order.len());
                    }
                    order.push(vec![summary]);
                }
            }
        }
        let mut shown = Vec::new();
        for mut group in order {
            let head = group.remove(0);
            let key = head.thread.clone().unwrap_or_default();
            if !group.is_empty() {
                if self.expanded.contains(&key) {
                    self.children.extend(group.iter().map(|summary| summary.id));
                    shown.push(head.clone());
                    shown.extend(group.iter().cloned());
                } else {
                    shown.push(head.clone());
                }
                self.earlier.insert(head.id, (key, group));
            } else {
                shown.push(head);
            }
        }
        shown
    }

    pub(crate) fn collapsed(&self, id: MessageId) -> Option<&[Summary]> {
        self.earlier
            .get(&id)
            .filter(|(key, _)| !self.expanded.contains(key))
            .map(|(_, earlier)| earlier.as_slice())
    }

    pub(crate) fn count(&self, id: MessageId) -> Option<usize> {
        self.earlier.get(&id).map(|(_, earlier)| earlier.len() + 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Drag {
    pub(crate) from: Point,
    pub(crate) at: Point,
    pub(crate) moving: bool,
    pub(crate) over: Option<usize>,
    collapse: Option<MessageId>,
}

#[derive(Debug, Clone)]
struct Moved {
    message_id: String,
    from: Mailbox,
    to: MailboxId,
    tries: u8,
}

pub(crate) struct SetupSheet {
    pub(crate) state: Shared,
    pub(crate) mode: Mode,
    pub(crate) password_field: WidgetId,
}

pub struct MailView {
    drop: FileDrop,
    pub(crate) store: Store,
    store_path: PathBuf,
    pub(crate) accounts: Vec<(Account, Vec<Mailbox>)>,
    workers: Vec<Worker>,
    pub(crate) statuses: Vec<(AccountId, Status)>,
    pub(crate) failures: Vec<(AccountId, String)>,
    retrying: Vec<AccountId>,
    mailroom: Mailroom,
    network: Network,
    pub(crate) sidebar: Vec<Row>,
    pub(crate) place: Place,
    pub(crate) messages: Vec<Summary>,
    listed: usize,
    listed_all: bool,
    pub(crate) selected: Option<MessageId>,
    pub(crate) also: Vec<MessageId>,
    pub(crate) reader: ReaderPane,
    pub(crate) search: String,
    pub(crate) ui: Ui,
    pub(crate) host: WidgetId,
    pub(crate) toolbar: Toolbar,
    pub(crate) signals: Signals,
    pub(crate) palette: Palette,
    pub(crate) symbols: Symbols,
    pub(crate) size: (f32, f32),
    pub(crate) sheet_inset: f32,
    pub(crate) scrolls: [ScrollState; 2],
    pub(crate) hits: Vec<(Rect, Hit)>,
    pub(crate) hover: Option<Hit>,
    pub(crate) focus: Pane,
    pub(crate) access: WidgetId,
    pub(crate) zone: TimeZone,
    pub(crate) formats: DateFormats,
    pub(crate) now: Option<Timestamp>,
    requests: Vec<Request>,
    setup: Option<SetupSheet>,
    pub(crate) settings: MailSettings,
    held: Vec<(AccountId, Command)>,
    undo: Vec<Vec<Moved>>,
    undoing: Vec<Moved>,
    moved_in: HashMap<(MailboxId, String), Instant>,
    pub(crate) drag: Option<Drag>,
    pub(crate) conversations: Conversations,
}

fn keyring_password(email: String, host: String) -> crate::worker::PasswordSource {
    Box::new(move || {
        Ok(Keyring::open(crate::KEYRING)?.password(&crate::keyring_key(&email, &host))?)
    })
}

impl MailView {
    pub fn new(
        store_path: PathBuf,
        settings: MailSettings,
        palette: Palette,
        text: TextSystem,
    ) -> Result<Self> {
        let store = Store::open(&store_path)?;
        let signals: Signals = Rc::new(RefCell::new(Vec::new()));
        let host = FreeLayout::new();
        let host_id = Widget::id(&host);
        let mut view = Self {
            drop: FileDrop::default(),
            store,
            store_path,
            accounts: Vec::new(),
            workers: Vec::new(),
            statuses: Vec::new(),
            failures: Vec::new(),
            retrying: Vec::new(),
            mailroom: Mailroom::new()?,
            network: Network::default(),
            sidebar: Vec::new(),
            place: Place::All(Role::Inbox),
            messages: Vec::new(),
            listed: LIST_PAGE,
            listed_all: true,
            selected: None,
            also: Vec::new(),
            reader: ReaderPane::default(),
            search: String::new(),
            ui: Ui::new(Box::new(host), Context::new(text, palette)),
            host: host_id,
            toolbar: Toolbar::default(),
            signals,
            palette,
            symbols: Symbols::default(),
            size: (0.0, 0.0),
            sheet_inset: SHEET_INSET as f32,
            scrolls: Default::default(),
            hits: Vec::new(),
            hover: None,
            focus: Pane::List,
            access: WidgetId::next(),
            zone: TimeZone::system(),
            formats: DateFormats::from_env(),
            now: None,
            requests: Vec::new(),
            setup: None,
            settings,
            held: Vec::new(),
            undo: Vec::new(),
            undoing: Vec::new(),
            moved_in: HashMap::new(),
            drag: None,
            conversations: Conversations::default(),
        };
        view.insert_toolbar();
        view.ui.set_sheet_top(TOOLBAR_HEIGHT);
        view.reload();
        Ok(view)
    }

    pub fn start(&mut self) {
        for account in self.accounts() {
            self.spawn_worker(account);
        }
        if self.accounts.is_empty() {
            self.present_setup(Mode::Kind);
        }
    }

    fn spawn_worker(&mut self, account: Account) {
        let source = keyring_password(account.email.clone(), account.imap.host.clone());
        self.workers.push(Worker::spawn(
            account,
            self.store_path.clone(),
            source,
            self.network.clone(),
            self.mailroom.notifier(),
        ));
    }

    pub fn settings(&self) -> &MailSettings {
        &self.settings
    }

    pub fn set_settings(&mut self, settings: MailSettings) {
        let regroup = settings.conversations != self.settings.conversations;
        let reread = (settings.plain_text, settings.fixed_width)
            != (self.settings.plain_text, self.settings.fixed_width);
        self.settings = settings;
        self.reader.layout = None;
        if regroup {
            self.reload();
        }
        if reread {
            let expanded = self
                .reader
                .thread
                .iter()
                .flat_map(|thread| &thread.entries)
                .filter(|entry| entry.reading.is_some())
                .map(|entry| entry.summary.id)
                .collect::<Vec<_>>();
            for id in expanded {
                self.read_entry(id);
            }
        }
    }

    fn keep_settings(&mut self) {
        if let Some(folder) = settings_folder()
            && let Err(error) = self.settings.save(&folder)
        {
            tracing::warn!("cannot save the settings: {error}");
        }
        self.reload();
    }

    pub fn toggle_conversation(&mut self, id: MessageId) -> bool {
        let head = self
            .conversations
            .earlier
            .iter()
            .find(|(head, (_, earlier))| {
                **head == id || earlier.iter().any(|summary| summary.id == id)
            })
            .map(|(head, (key, _))| (*head, key.clone()));
        let Some((head, key)) = head else {
            return false;
        };
        if !self.conversations.expanded.remove(&key) {
            self.conversations.expanded.insert(key);
        } else if self.selected != Some(head) && self.is_chosen(id) {
            self.select(head);
        }
        self.reload();
        true
    }

    pub fn accounts_changed(&mut self) {
        let stored = self
            .store
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .filter(|account| account.enabled)
            .collect::<Vec<_>>();
        let known = self.accounts();
        let connects_alike = |a: &Account, b: &Account| {
            (&a.email, &a.user, &a.imap, &a.smtp) == (&b.email, &b.user, &b.imap, &b.smtp)
        };
        let restarted = stored
            .iter()
            .filter(|account| {
                !known
                    .iter()
                    .any(|old| old.id == account.id && connects_alike(old, account))
            })
            .cloned()
            .collect::<Vec<_>>();
        self.workers.retain(|worker| {
            stored.iter().any(|account| account.id == worker.account)
                && !restarted.iter().any(|account| account.id == worker.account)
        });
        self.statuses
            .retain(|(id, _)| stored.iter().any(|account| account.id == *id));
        let same = restarted.is_empty() && stored.len() == known.len();
        for account in restarted {
            self.spawn_worker(account);
        }
        self.release_held();
        match same {
            true => self.refresh_sidebar(),
            false => self.reload(),
        }
    }

    pub fn has_accounts(&self) -> bool {
        !self.accounts.is_empty()
    }

    pub fn accounts(&self) -> Vec<Account> {
        self.accounts
            .iter()
            .map(|(account, _)| account.clone())
            .collect()
    }

    pub fn correspondents(&self) -> Vec<crate::message::Address> {
        let mut found = self
            .store
            .correspondents(CORRESPONDENTS_READ)
            .inspect_err(|error| tracing::warn!("cannot read known addresses: {error}"))
            .unwrap_or_default();
        found.retain(|address| {
            !self
                .accounts
                .iter()
                .any(|(account, _)| account.email.eq_ignore_ascii_case(&address.email))
        });
        found
    }

    pub fn fds(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.mailroom.fd()]
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn messages(&self) -> &[Summary] {
        &self.messages
    }

    pub fn printable(&self) -> Option<(String, String)> {
        let (summary, reading) = self.reader.focused()?;
        let envelope = &reading.content.envelope;
        let names = |list: &[crate::message::Address]| {
            list.iter()
                .map(crate::message::Address::full)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut lines = vec![envelope.subject.clone(), names(&envelope.from)];
        lines.extend(
            envelope
                .date
                .map(|date| full_date(date, &self.zone, &self.formats)),
        );
        lines.push(tr!("To: {names}", names = names(&envelope.to)));
        if !envelope.cc.is_empty() {
            lines.push(tr!("Cc: {names}", names = names(&envelope.cc)));
        }
        let text = format!("{}\n\n{}", lines.join("\n"), reading.content.text());
        let title = match summary.subject.is_empty() {
            true => tr!("No Subject"),
            false => summary.subject.clone(),
        };
        Some((title, text))
    }

    pub fn unread_inbox(&self) -> usize {
        self.sidebar
            .iter()
            .find_map(|row| match row {
                Row::Place {
                    place: Place::All(Role::Inbox),
                    unread,
                    ..
                } => Some(*unread),
                _ => None,
            })
            .unwrap_or(0)
    }

    pub fn selected(&self) -> Option<MessageId> {
        self.selected
    }

    pub fn place(&self) -> &Place {
        &self.place
    }

    pub fn sidebar(&self) -> &[Row] {
        &self.sidebar
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.ui.set_appearance(appearance);
        self.reader.layout = None;
    }

    pub fn set_window_active(&mut self, active: bool) -> bool {
        self.ui.set_window_active(active)
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
        self.ui.context().palette = palette;
        self.reader.layout = None;
    }

    pub fn set_full_screen(&mut self, full_screen: bool) {
        self.sheet_inset = match full_screen {
            true => 0.0,
            false => SHEET_INSET as f32,
        };
    }

    pub fn title(&self) -> String {
        self.sidebar
            .iter()
            .find_map(|row| match row {
                Row::Place { place, title, .. } if *place == self.place => Some(title.clone()),
                _ => None,
            })
            .unwrap_or_else(|| tr!("Mail"))
    }

    pub fn header(&self) -> Header {
        self.toolbar_header()
    }

    pub fn cursor(&self) -> Cursor {
        match &self.hover {
            Some(Hit::Conversation(_) | Hit::Offline(_)) => Cursor::Pointer,
            Some(Hit::Reader(Target::Card(_))) => Cursor::Text,
            Some(Hit::Reader(_)) => Cursor::Pointer,
            _ => self.ui.cursor(),
        }
    }

    pub fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.ui.deadline()
    }

    pub fn accessibility(&self) -> TreeUpdate {
        let mut tree = self.ui.accessibility();
        self.describe_panes(&mut tree);
        tree
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        let handled = self
            .perform_on_panes(request)
            .unwrap_or_else(|| self.ui.perform(request));
        self.after_ui();
        handled
    }

    pub fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }

    pub fn pasted(&mut self, text: &str) -> bool {
        let handled = self.ui.pasted(text);
        self.after_ui();
        handled
    }

    pub fn dragged(&mut self, window: &mut Window, event: &WindowEvent) -> bool {
        match self.drop.event(window, event, attachable) {
            Dropping::Over => true,
            Dropping::Dropped(paths) => {
                self.requests.push(Request::Attach(paths));
                true
            }
            Dropping::Other => {
                let handled = match window.drive(&mut self.ui, event.clone()) {
                    Ok(unhandled) => unhandled.is_none(),
                    Err(error) => {
                        tracing::warn!("cannot take the dragged text: {error}");
                        false
                    }
                };
                self.after_ui();
                handled
            }
        }
    }

    pub fn start_text_drag(&mut self, window: &mut Window) {
        if let Err(error) = window.start_text_drag(&mut self.ui) {
            tracing::warn!("cannot drag the selected text: {error}");
        }
    }

    pub fn menu_closed(&mut self) -> bool {
        self.ui.menu_closed()
    }

    pub(crate) fn account(&self, id: AccountId) -> Option<&Account> {
        self.accounts
            .iter()
            .map(|(account, _)| account)
            .find(|account| account.id == id)
    }

    fn worker(&self, account: AccountId) -> Option<&Worker> {
        self.workers.iter().find(|worker| worker.account == account)
    }

    pub fn send_command(&self, account: AccountId, command: Command) {
        if let Some(worker) = self.worker(account) {
            worker.send(command);
        }
    }

    fn flush(&self, accounts: &[AccountId]) {
        for account in accounts {
            self.send_command(*account, Command::Flush);
        }
    }

    fn refresh_sidebar(&mut self) {
        let accounts = self
            .store
            .accounts()
            .inspect_err(|error| tracing::warn!("cannot read the accounts: {error}"))
            .unwrap_or_default();
        self.accounts = accounts
            .into_iter()
            .filter(|account| account.enabled)
            .map(|account| {
                let mailboxes = self
                    .store
                    .mailboxes(account.id)
                    .inspect_err(|error| tracing::warn!("cannot read the mailboxes: {error}"))
                    .unwrap_or_default();
                (account, mailboxes)
            })
            .collect();
        let store = &self.store;
        self.sidebar = rows(&self.accounts, |scope| {
            store
                .unread(scope)
                .inspect_err(|error| tracing::warn!("cannot count unread mail: {error}"))
                .unwrap_or(0)
        });
    }

    pub fn reload(&mut self) {
        self.refresh_sidebar();
        let exists = self
            .sidebar
            .iter()
            .any(|row| matches!(row, Row::Place { place, .. } if *place == self.place));
        if !exists {
            self.place = Place::All(Role::Inbox);
        }
        let all = self
            .store
            .listing(
                &self.place.scope(),
                &self.search,
                self.settings.sort,
                self.settings.ascending,
                self.listed,
            )
            .inspect_err(|error| tracing::warn!("cannot list the messages: {error}"))
            .unwrap_or_default();
        self.listed_all = all.len() < self.listed;
        self.messages = match self.settings.conversations {
            true => self.conversations.group(all),
            false => {
                self.conversations = Conversations {
                    expanded: std::mem::take(&mut self.conversations.expanded),
                    ..Conversations::default()
                };
                all
            }
        };
        let messages = &self.messages;
        self.also
            .retain(|id| messages.iter().any(|summary| summary.id == *id));
        if self
            .selected
            .is_some_and(|id| !self.messages.iter().any(|summary| summary.id == id))
        {
            self.selected = None;
            self.also.clear();
            self.reader.thread = None;
            self.reader.layout = None;
        }
        self.refresh_thread();
    }

    fn conversation_of(&self, summary: &Summary) -> Vec<Summary> {
        match self.settings.conversations {
            true => self
                .store
                .conversation(summary)
                .inspect_err(|error| tracing::warn!("cannot read a conversation: {error}"))
                .unwrap_or_else(|_| vec![summary.clone()]),
            false => vec![summary.clone()],
        }
    }

    fn refresh_thread(&mut self) {
        let Some(thread) = &self.reader.thread else {
            return;
        };
        let Some(focused) = thread
            .entry(thread.focus)
            .map(|entry| entry.summary.clone())
        else {
            return;
        };
        let messages = self.conversation_of(&focused);
        let Some(thread) = self.reader.thread.as_mut() else {
            return;
        };
        let same = messages.len() == thread.entries.len()
            && messages
                .iter()
                .zip(&thread.entries)
                .all(|(summary, entry)| summary.id == entry.summary.id);
        if same {
            for (summary, entry) in messages.into_iter().zip(&mut thread.entries) {
                entry.summary = summary;
            }
            return;
        }
        let mut old = std::mem::take(&mut thread.entries);
        thread.entries = messages
            .into_iter()
            .map(
                |summary| match old.iter().position(|entry| entry.summary.id == summary.id) {
                    Some(index) => Entry {
                        summary,
                        ..old.swap_remove(index)
                    },
                    None => Entry {
                        summary,
                        reading: None,
                        quotes: false,
                    },
                },
            )
            .collect();
        if thread.entry(thread.focus).is_none()
            && let Some(last) = thread.entries.last()
        {
            thread.focus = last.summary.id;
        }
        self.reader.layout = None;
    }

    pub fn choose_place(&mut self, place: Place) {
        if place == self.place {
            return;
        }
        self.place = place;
        self.listed = LIST_PAGE;
        self.selected = None;
        self.also.clear();
        self.reader.thread = None;
        self.reader.layout = None;
        self.scrolls[1] = ScrollState::default();
        self.reload();
    }

    pub fn search_for(&mut self, text: &str) {
        self.search = text.to_owned();
        self.listed = LIST_PAGE;
        self.scrolls[1] = ScrollState::default();
        self.reload();
    }

    pub fn show_message(&mut self, id: MessageId) {
        if let Ok(Some(summary)) = self.store.summary(id) {
            self.choose_place(Place::Mailbox(summary.mailbox));
            self.select(id);
        }
    }

    pub fn select(&mut self, id: MessageId) {
        let Some(summary) = self
            .messages
            .iter()
            .find(|summary| summary.id == id)
            .cloned()
        else {
            return;
        };
        self.selected = Some(id);
        self.also.clear();
        self.focus = Pane::List;
        self.reader.scroll = ScrollState::default();
        let unseen = self.open_thread(&summary);
        if !unseen.is_empty() {
            self.set_flag_on(&unseen, Flag::Seen, true);
        }
        self.reveal_selected();
    }

    fn select_more(&mut self, id: MessageId, range: bool) {
        let Some(anchor) = self.selected else {
            return self.select(id);
        };
        let position = |id: MessageId| self.messages.iter().position(|summary| summary.id == id);
        if range {
            let (Some(from), Some(to)) = (position(anchor), position(id)) else {
                return;
            };
            self.also = self.messages[from.min(to)..=from.max(to)]
                .iter()
                .map(|summary| summary.id)
                .filter(|known| *known != anchor)
                .collect();
        } else if id == anchor {
            if self.also.is_empty() {
                return;
            }
            self.selected = Some(self.also.remove(0));
        } else if let Some(index) = self.also.iter().position(|known| *known == id) {
            self.also.remove(index);
        } else {
            self.also.push(id);
        }
        self.focus = Pane::List;
        match (self.also.is_empty(), self.selected_summary()) {
            (true, Some(summary)) => {
                self.reader.scroll = ScrollState::default();
                self.open_thread(&summary);
            }
            _ => {
                self.reader.thread = None;
                self.reader.layout = None;
            }
        }
    }

    pub(crate) fn is_chosen(&self, id: MessageId) -> bool {
        self.selected == Some(id) || self.also.contains(&id)
    }

    pub(crate) fn chosen(&self) -> Vec<Summary> {
        self.messages
            .iter()
            .filter(|summary| self.is_chosen(summary.id))
            .flat_map(|summary| {
                std::iter::once(summary.clone()).chain(
                    self.conversations
                        .collapsed(summary.id)
                        .unwrap_or_default()
                        .iter()
                        .cloned(),
                )
            })
            .collect()
    }

    fn reader_target(&self) -> Option<&Summary> {
        let thread = self
            .reader
            .thread
            .as_ref()
            .filter(|_| self.focus == Pane::Reader)?;
        thread.entry(thread.focus).map(|entry| &entry.summary)
    }

    pub(crate) fn targets(&self) -> Vec<Summary> {
        match self.reader_target() {
            Some(summary) => vec![summary.clone()],
            None => self.chosen(),
        }
    }

    fn first_target(&self) -> Option<Summary> {
        self.reader_target()
            .cloned()
            .or_else(|| self.selected_summary())
    }

    fn reading_of(&self, summary: &Summary) -> Reading {
        let body = self
            .store
            .body(summary.id)
            .inspect_err(|error| tracing::warn!("cannot read a message: {error}"))
            .ok()
            .flatten();
        match body {
            Some(raw) => Reading::new(Content::parse(&raw), true, &self.settings),
            None => {
                let header = self
                    .store
                    .header(summary.id)
                    .inspect_err(|error| tracing::warn!("cannot read a message's header: {error}"))
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                self.send_command(summary.account, Command::Body(summary.id));
                Reading::new(
                    Content {
                        envelope: Envelope::parse(&header),
                        ..Content::default()
                    },
                    false,
                    &self.settings,
                )
            }
        }
    }

    fn open_thread(&mut self, summary: &Summary) -> Vec<MessageId> {
        let messages = self.conversation_of(summary);
        let focus = messages
            .iter()
            .position(|message| message.id == summary.id)
            .unwrap_or_default();
        let shown = messages
            .iter()
            .enumerate()
            .filter(|(index, message)| {
                message.id == summary.id
                    || (!message.flags.seen && index + OPENED_UNREAD >= focus && *index <= focus)
            })
            .map(|(_, message)| (message.id, message.flags.seen))
            .collect::<Vec<_>>();
        self.reader.thread = Some(Thread {
            entries: messages
                .into_iter()
                .map(|summary| Entry {
                    summary,
                    reading: None,
                    quotes: false,
                })
                .collect(),
            focus: summary.id,
            reveal: true,
        });
        for (id, _) in &shown {
            self.read_entry(*id);
        }
        shown
            .into_iter()
            .filter(|(_, seen)| !seen)
            .map(|(id, _)| id)
            .collect()
    }

    fn read_entry(&mut self, id: MessageId) {
        let Some(summary) = self
            .reader
            .thread
            .as_ref()
            .and_then(|thread| thread.entry(id))
            .map(|entry| entry.summary.clone())
        else {
            return;
        };
        let reading = self.reading_of(&summary);
        let remote = self.loads_remote(&reading);
        let pending = reading.content.security.pending;
        if let Some(entry) = self
            .reader
            .thread
            .as_mut()
            .and_then(|thread| thread.entry_mut(id))
        {
            entry.reading = Some(reading);
        }
        self.reader.layout = None;
        if pending {
            self.unseal(id);
        } else if remote {
            self.load_remote(id);
        }
    }

    fn loads_remote(&self, reading: &Reading) -> bool {
        reading.downloaded
            && reading.document.remote_images > 0
            && (self.settings.load_remote
                || self
                    .store
                    .trusts_remote(&reading.content.envelope.sender().email)
                    .unwrap_or(false))
    }

    fn toggle_entry(&mut self, id: MessageId) {
        let Some(thread) = self.reader.thread.as_mut() else {
            return;
        };
        let Some(entry) = thread.entry_mut(id) else {
            return;
        };
        if entry.reading.take().is_none() {
            let unseen = !entry.summary.flags.seen;
            self.reader.focus_entry(id);
            self.read_entry(id);
            if unseen {
                self.set_flag_on(&[id], Flag::Seen, true);
            }
            return;
        }
        if thread.focus == id
            && let Some(open) = thread
                .entries
                .iter()
                .rev()
                .find(|entry| entry.reading.is_some())
        {
            thread.focus = open.summary.id;
        }
        self.reader.layout = None;
    }

    fn selected_summary(&self) -> Option<Summary> {
        let id = self.selected?;
        self.messages
            .iter()
            .find(|summary| summary.id == id)
            .cloned()
    }

    fn set_flag_on(&mut self, ids: &[MessageId], flag: Flag, value: bool) {
        match self.store.set_flag(ids, flag, value) {
            Ok(accounts) => self.flush(&accounts),
            Err(error) => tracing::warn!("cannot change a flag: {error}"),
        }
        let earlier = self
            .conversations
            .earlier
            .values_mut()
            .flat_map(|(_, earlier)| earlier.iter_mut());
        let entries = self
            .reader
            .thread
            .iter_mut()
            .flat_map(|thread| thread.entries.iter_mut())
            .map(|entry| &mut entry.summary);
        for summary in self
            .messages
            .iter_mut()
            .chain(earlier)
            .chain(entries)
            .filter(|summary| ids.contains(&summary.id))
        {
            summary.flags.set(flag, value);
        }
        self.reader.layout = None;
        self.refresh_sidebar();
    }

    fn toggle(&mut self, flag: Flag, is_set: fn(&Summary) -> bool) {
        let targets = self.targets();
        let Some(first) = self.first_target().or_else(|| targets.first().cloned()) else {
            return;
        };
        let ids = targets.iter().map(|summary| summary.id).collect::<Vec<_>>();
        self.set_flag_on(&ids, flag, !is_set(&first));
    }

    pub fn toggle_flag(&mut self) {
        self.toggle(Flag::Flagged, |summary| summary.flags.flagged);
    }

    fn flag_with(&mut self, color: u8) {
        let ids = self
            .targets()
            .iter()
            .map(|summary| summary.id)
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return;
        }
        self.set_flag_on(&ids, Flag::Flagged, true);
        for (bit, flag) in Flag::COLORS.into_iter().enumerate() {
            self.set_flag_on(&ids, flag, color & (1 << bit) != 0);
        }
    }

    pub fn flag_colors() -> Vec<(String, String)> {
        FLAG_COLORS
            .iter()
            .enumerate()
            .map(|(color, name)| (name.tr(), format!("flag-color:{color}")))
            .collect()
    }

    pub fn mark_all_read(&mut self) {
        let unread = self
            .store
            .summaries(&self.place.scope(), "")
            .inspect_err(|error| tracing::warn!("cannot list the messages: {error}"))
            .unwrap_or_default()
            .into_iter()
            .filter(|summary| !summary.flags.seen)
            .map(|summary| summary.id)
            .collect::<Vec<_>>();
        if !unread.is_empty() {
            self.set_flag_on(&unread, Flag::Seen, true);
        }
    }

    pub fn toggle_read(&mut self) {
        self.toggle(Flag::Seen, |summary| summary.flags.seen);
    }

    fn mailbox_with_role(&self, account: AccountId, role: Role) -> Option<Mailbox> {
        self.accounts
            .iter()
            .find(|(known, _)| known.id == account)
            .and_then(|(_, mailboxes)| {
                mailboxes
                    .iter()
                    .find(|mailbox| mailbox.role == Some(role))
                    .cloned()
            })
    }

    fn mailbox_of(&self, summary: &Summary) -> Option<Mailbox> {
        self.accounts
            .iter()
            .flat_map(|(_, mailboxes)| mailboxes)
            .find(|mailbox| mailbox.id == summary.mailbox)
            .cloned()
    }

    pub fn can_move_to(&self, role: Role) -> bool {
        self.targets().iter().any(|summary| {
            self.mailbox_with_role(summary.account, role)
                .is_some_and(|target| target.id != summary.mailbox)
        })
    }

    fn leave_selected(&mut self, ids: &[MessageId]) {
        if ids.is_empty() {
            return;
        }
        let position = self
            .messages
            .iter()
            .position(|summary| ids.contains(&summary.id));
        self.messages.retain(|summary| !ids.contains(&summary.id));
        self.selected = None;
        self.also.clear();
        self.reader.thread = None;
        self.reader.layout = None;
        let next = position
            .and_then(|position| {
                self.messages.get(position).or_else(|| {
                    position
                        .checked_sub(1)
                        .and_then(|before| self.messages.get(before))
                })
            })
            .map(|summary| summary.id);
        if let Some(next) = next {
            self.select(next);
        }
        self.refresh_sidebar();
    }

    fn leave_targets(&mut self, gone: &[MessageId]) {
        if gone.is_empty() {
            return;
        }
        let Some(row) = self.selected.filter(|_| self.reader_target().is_some()) else {
            return self.leave_selected(gone);
        };
        let members = std::iter::once(row)
            .chain(
                self.conversations
                    .earlier
                    .get(&row)
                    .into_iter()
                    .flat_map(|(_, earlier)| earlier.iter().map(|summary| summary.id)),
            )
            .filter(|id| !gone.contains(id))
            .collect::<Vec<_>>();
        if members.is_empty() {
            return self.leave_selected(&[row]);
        }
        let mut thread = self.reader.thread.take();
        if let Some(thread) = thread.as_mut()
            && let Some(at) = thread
                .entries
                .iter()
                .position(|entry| entry.summary.id == thread.focus)
        {
            let (before, after) = thread.entries.split_at(at);
            if let Some(next) = after
                .iter()
                .chain(before.iter().rev())
                .find(|entry| !gone.contains(&entry.summary.id))
            {
                thread.focus = next.summary.id;
                thread.reveal = true;
            }
        }
        self.reload();
        let conversations = &self.conversations;
        self.selected = self
            .messages
            .iter()
            .find(|summary| {
                members.contains(&summary.id)
                    || conversations
                        .earlier
                        .get(&summary.id)
                        .is_some_and(|(_, earlier)| {
                            earlier.iter().any(|summary| members.contains(&summary.id))
                        })
            })
            .map(|summary| summary.id);
        if self.selected.is_some() {
            self.reader.thread = thread;
            self.reader.layout = None;
            self.refresh_thread();
        }
    }

    fn move_each(&mut self, moves: Vec<(MessageId, Mailbox)>) -> Vec<MessageId> {
        let mut moved = Vec::new();
        let mut done = Vec::new();
        let mut undo = Vec::new();
        for (id, target) in &moves {
            if let Ok(Some(message_id)) = self.store.message_id(*id) {
                self.expect_arrival(target, message_id);
            }
            if let (Some(from), Ok(Some(message_id))) = (
                self.store
                    .summary(*id)
                    .ok()
                    .flatten()
                    .and_then(|summary| self.mailbox_of(&summary)),
                self.store.message_id(*id),
            ) {
                undo.push(Moved {
                    message_id,
                    from,
                    to: target.id,
                    tries: 0,
                });
            }
        }
        if !undo.is_empty() {
            if self.undo.len() == UNDO_LEVELS {
                self.undo.remove(0);
            }
            self.undo.push(undo);
        }
        for (_, target) in &moves {
            if done.contains(&target.id) {
                continue;
            }
            done.push(target.id);
            let ids = moves
                .iter()
                .filter(|(_, known)| known.id == target.id)
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            match self.store.move_messages(&ids, target) {
                Ok(accounts) => {
                    self.flush(&accounts);
                    moved.extend(ids);
                }
                Err(error) => tracing::warn!("cannot move the messages: {error}"),
            }
        }
        moved
    }

    pub fn move_selected(&mut self, role: Role) {
        let moves = self
            .targets()
            .into_iter()
            .filter_map(|summary| {
                let target = self.mailbox_with_role(summary.account, role)?;
                (target.id != summary.mailbox).then_some((summary.id, target))
            })
            .collect();
        let moved = self.move_each(moves);
        self.leave_targets(&moved);
    }

    pub fn move_targets(&self) -> Vec<(String, MailboxId)> {
        let Some(summary) = self.first_target() else {
            return Vec::new();
        };
        self.accounts
            .iter()
            .filter(|(account, _)| account.id == summary.account)
            .flat_map(|(_, mailboxes)| mailboxes)
            .filter(|mailbox| mailbox.selectable && mailbox.id != summary.mailbox)
            .map(|mailbox| {
                let name = match mailbox.delimiter {
                    Some(delimiter) => mailbox.path.replace(delimiter, " / "),
                    None => mailbox.path.clone(),
                };
                let name = match mailbox.role {
                    Some(Role::Inbox) => tr!("Inbox"),
                    _ => name,
                };
                (name, mailbox.id)
            })
            .collect()
    }

    fn move_selected_to(&mut self, target: &Mailbox) {
        let moves = self
            .targets()
            .into_iter()
            .filter(|summary| target.id != summary.mailbox && target.account == summary.account)
            .map(|summary| (summary.id, target.clone()))
            .collect();
        let moved = self.move_each(moves);
        self.leave_targets(&moved);
    }

    pub fn undo(&mut self) {
        let moves = self.undo.pop().unwrap_or_default();
        self.put_back(moves);
    }

    fn expect_arrival(&mut self, mailbox: &Mailbox, message_id: String) {
        let now = Instant::now();
        self.moved_in
            .retain(|_, since| now.duration_since(*since) < MOVED_IN_KEPT);
        if mailbox.role == Some(Role::Inbox) {
            self.moved_in.insert((mailbox.id, message_id), now);
        }
    }

    fn expected_arrival(&mut self, id: MessageId) -> bool {
        let (Ok(Some(summary)), Ok(Some(message_id))) =
            (self.store.summary(id), self.store.message_id(id))
        else {
            return false;
        };
        self.moved_in
            .remove(&(summary.mailbox, message_id))
            .is_some_and(|since| since.elapsed() < MOVED_IN_KEPT)
    }

    fn put_back(&mut self, moves: Vec<Moved>) {
        let mut waiting = Vec::new();
        let mut lost = Vec::new();
        for moved in moves {
            match self.store.find_message(moved.to, &moved.message_id) {
                Ok(Some(id)) => {
                    let from = match self.store.mailbox(moved.from.id) {
                        Ok(Some(from)) => from,
                        Ok(None) => continue,
                        Err(error) => {
                            tracing::warn!("cannot restore the message: {error}");
                            continue;
                        }
                    };
                    self.expect_arrival(&from, moved.message_id.clone());
                    match self.store.move_messages(&[id], &from) {
                        Ok(accounts) => self.flush(&accounts),
                        Err(error) => tracing::warn!("cannot restore the message: {error}"),
                    }
                }
                Ok(None) if moved.tries < UNDO_TRIES => {
                    if moved.tries == 0 && !waiting.contains(&moved.to) {
                        waiting.push(moved.to);
                        self.send_command(moved.from.account, Command::Refresh(moved.to));
                    }
                    self.undoing.push(Moved {
                        tries: moved.tries + 1,
                        ..moved
                    });
                }
                Ok(None) => lost.push(moved.to),
                Err(error) => tracing::warn!("cannot restore the message: {error}"),
            }
        }
        if let Some(mailbox) = lost.first().and_then(|id| self.mailbox(*id)) {
            self.ui.present_sheet(
                Alert::new(
                    &trn!("A Message Could Not Be Restored", "{count} Messages Could Not Be Restored", count = lost.len()),
                    &tr!("Mail did not find it in {mailbox} on the server. It may have been moved or deleted there.", mailbox = mailbox.name),
                    |_| {},
                )
                .primary(&tr!("OK"), "ok"),
            );
        }
        self.reload();
    }

    pub fn delete_selected(&mut self) {
        let targets = self.targets();
        let gone = self.delete(targets);
        self.leave_targets(&gone);
    }

    pub fn discard_draft(&mut self, draft: &Original) {
        let summary = match (
            self.store.summary(draft.id),
            self.store.message_id(draft.id),
        ) {
            (Ok(Some(summary)), Ok(Some(message_id))) if message_id == draft.message_id => summary,
            (Err(error), _) | (_, Err(error)) => {
                return tracing::warn!("cannot discard the draft: {error}");
            }
            _ => return,
        };
        self.delete(vec![summary]);
        self.reload();
    }

    fn delete(&mut self, summaries: Vec<Summary>) -> Vec<MessageId> {
        let mut moves = Vec::new();
        let mut expunged = Vec::new();
        for summary in summaries {
            let in_trash = self
                .mailbox_of(&summary)
                .is_some_and(|mailbox| mailbox.role == Some(Role::Trash));
            match self.mailbox_with_role(summary.account, Role::Trash) {
                Some(trash) if !in_trash => moves.push((summary.id, trash)),
                _ => expunged.push(summary.id),
            }
        }
        let mut gone = self.move_each(moves);
        if !expunged.is_empty() {
            match self.store.expunge(&expunged) {
                Ok(accounts) => {
                    self.flush(&accounts);
                    gone.extend(expunged);
                }
                Err(error) => tracing::warn!("cannot delete the messages: {error}"),
            }
        }
        gone
    }

    fn default_account(&self) -> Option<Account> {
        let account = self
            .selected_summary()
            .map(|summary| summary.account)
            .or(match self.place {
                Place::Mailbox(id) => self
                    .accounts
                    .iter()
                    .find(|(_, mailboxes)| mailboxes.iter().any(|mailbox| mailbox.id == id))
                    .map(|(account, _)| account.id),
                _ => None,
            });
        account
            .and_then(|id| self.account(id).cloned())
            .or_else(|| self.accounts.first().map(|(account, _)| account.clone()))
    }

    fn sending_account(&self) -> Option<Account> {
        self.accounts
            .iter()
            .map(|(account, _)| account)
            .find(|account| account.email.eq_ignore_ascii_case(&self.settings.send_from))
            .cloned()
            .or_else(|| self.default_account())
    }

    pub fn compose(&mut self, mut draft: Draft) {
        let Some(account) = self.sending_account() else {
            self.present_setup(Mode::Kind);
            return;
        };
        if draft.from.email.is_empty() {
            draft.from = Address::from(&account);
        }
        draft.sign = self.signs(account.id);
        draft.body = match draft.body.trim_end() {
            "" => signed("", &account.signature),
            body => format!("{body}{}", signed("", &account.signature)),
        };
        self.requests.push(Request::Compose(Box::new(Outgoing {
            draft,
            answering: None,
            replaces: None,
            unsent: None,
        })));
    }

    pub fn respond(&mut self, all: bool) {
        let Some((summary, reading)) = self.reader.focused() else {
            return;
        };
        let Some(account) = self.account(summary.account) else {
            return;
        };
        let envelope = &reading.content.envelope;
        let attribution = tr!(
            "On {date}, {name} wrote:",
            date = envelope
                .date
                .map(|date| full_date(date, &self.zone, &self.formats))
                .unwrap_or_default(),
            name = envelope.sender().shown().to_owned()
        );
        let quote = self.settings.quote.then_some(attribution.as_str());
        let mut draft = reply(&reading.content, &Address::from(account), all, quote);
        draft.body = signed(&draft.body, &account.signature);
        draft.sign = self.signs(account.id);
        self.requests.push(Request::Compose(Box::new(Outgoing {
            draft,
            answering: Some(Original {
                id: summary.id,
                message_id: envelope.message_id.clone().unwrap_or_default(),
            }),
            replaces: None,
            unsent: None,
        })));
    }

    pub fn continue_draft(&mut self) {
        let Some((summary, reading)) = self.reader.focused() else {
            return;
        };
        let is_draft = summary.flags.draft
            || self
                .mailbox_of(summary)
                .is_some_and(|mailbox| mailbox.role == Some(Role::Drafts));
        if !is_draft || !reading.downloaded {
            return;
        }
        let Some(account) = self.account(summary.account) else {
            return;
        };
        let envelope = &reading.content.envelope;
        let draft = Draft {
            from: Address::from(account),
            to: envelope.to.clone(),
            cc: envelope.cc.clone(),
            bcc: envelope.bcc.clone(),
            subject: envelope.subject.clone(),
            body: reading.content.text().replace("\r\n", "\n"),
            in_reply_to: envelope.references.last().cloned(),
            references: envelope.references.clone(),
            attachments: reading.content.attachments.clone(),
            sign: self.signs(account.id),
            encrypt: reading.content.security.encrypted.is_some(),
        };
        self.requests.push(Request::Compose(Box::new(Outgoing {
            draft,
            answering: envelope.references.last().and_then(|parent| {
                let id = self.store.locate(summary.account, parent, None).ok()??;
                Some(Original {
                    id,
                    message_id: parent.clone(),
                })
            }),
            replaces: Some(Original {
                id: summary.id,
                message_id: envelope.message_id.clone().unwrap_or_default(),
            }),
            unsent: None,
        })));
    }

    pub fn forward_selected(&mut self) {
        let Some((summary, reading)) = self.reader.focused() else {
            return;
        };
        let Some(account) = self.account(summary.account) else {
            return;
        };
        let envelope = &reading.content.envelope;
        let names = |addresses: &[crate::message::Address]| {
            addresses
                .iter()
                .map(crate::message::Address::full)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let heading = tr!(
            "Begin forwarded message:\n\nFrom: {from}\nSubject: {subject}\nDate: {date}\nTo: {to}",
            from = names(&envelope.from),
            date = envelope
                .date
                .map(|date| full_date(date, &self.zone, &self.formats))
                .unwrap_or_default(),
            subject = envelope.subject.clone(),
            to = names(&envelope.to)
        );
        let mut draft = forward(&reading.content, &Address::from(account), &heading);
        draft.body = signed(&draft.body, &account.signature);
        draft.sign = self.signs(account.id);
        self.requests.push(Request::Compose(Box::new(Outgoing {
            draft,
            answering: None,
            replaces: None,
            unsent: None,
        })));
    }

    pub fn get_mail(&mut self) {
        self.retrying = self
            .statuses
            .iter()
            .filter(|(_, status)| *status == Status::Offline)
            .map(|(account, _)| *account)
            .collect();
        for worker in &self.workers {
            worker.send(Command::Sync);
        }
    }

    pub(crate) fn offline(&self, account: AccountId) -> bool {
        self.statuses
            .iter()
            .any(|(known, status)| *known == account && *status == Status::Offline)
    }

    pub(crate) fn failure(&self, account: AccountId) -> String {
        self.failures
            .iter()
            .find(|(known, _)| *known == account)
            .map(|(_, reason)| reason.clone())
            .unwrap_or_else(|| tr!("Mail can’t reach the server."))
    }

    pub(crate) fn show_failure(&mut self, account: AccountId) {
        let Some(email) = self.account(account).map(|account| account.email.clone()) else {
            return;
        };
        if self.ui.has_sheet() {
            return;
        }
        let signals = Rc::clone(&self.signals);
        let alert = Alert::new(
            &tr!("“{account}” is offline", account = email),
            &self.failure(account),
            move |choice| match choice {
                "retry" => signals.borrow_mut().push(Signal::TryAgain(account)),
                "settings" => signals.borrow_mut().push(Signal::ServerSettings),
                _ => {}
            },
        )
        .primary(&tr!("Try Again"), "retry")
        .other(&tr!("Server Settings…"), "settings")
        .cancel(&tr!("Cancel"), "cancel");
        self.ui.present_sheet(alert);
    }

    fn account_for(&self, draft: &Draft) -> Option<AccountId> {
        self.accounts
            .iter()
            .map(|(account, _)| account)
            .find(|account| account.email.eq_ignore_ascii_case(&draft.from.email))
            .map(|account| account.id)
    }

    pub fn recover(&self, mut outgoing: Outgoing) -> Outgoing {
        let account = self.account_for(&outgoing.draft);
        let located = |original: Option<Original>, role: Option<Role>| {
            let original = original?;
            if original.message_id.is_empty() {
                return None;
            }
            if self.store.message_id(original.id).ok().flatten().as_ref()
                == Some(&original.message_id)
            {
                return Some(original);
            }
            let id = self
                .store
                .locate(account?, &original.message_id, role)
                .ok()
                .flatten()?;
            Some(Original { id, ..original })
        };
        outgoing.answering = located(outgoing.answering.take(), None);
        outgoing.replaces = located(outgoing.replaces.take(), Some(Role::Drafts));
        outgoing
    }

    pub fn save_draft(&mut self, outgoing: Outgoing) -> bool {
        self.dispatch(Command::SaveDraft(Box::new(outgoing)))
    }

    pub fn send(&mut self, outgoing: Outgoing) -> bool {
        self.dispatch(Command::Send(Box::new(outgoing)))
    }

    fn dispatch(&mut self, command: Command) -> bool {
        let (Command::Send(outgoing) | Command::SaveDraft(outgoing)) = &command else {
            return false;
        };
        if let Some(account) = self.account_for(&outgoing.draft) {
            self.send_command(account, command);
            return true;
        }
        let email = outgoing.draft.from.email.clone();
        let Some(account) = self
            .store
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .find(|account| account.email.eq_ignore_ascii_case(&email))
        else {
            return false;
        };
        self.held.push((account.id, command));
        self.tell_held(&[account.id]);
        true
    }

    fn release_held(&mut self) {
        let stored = match self.store.accounts() {
            Ok(stored) => stored,
            Err(error) => return tracing::warn!("cannot read the accounts: {error}"),
        };
        for (account, command) in std::mem::take(&mut self.held) {
            if self.worker(account).is_some() {
                self.send_command(account, command);
            } else if stored.iter().any(|known| known.id == account) {
                self.held.push((account, command));
            } else if let Command::Send(outgoing) | Command::SaveDraft(outgoing) = command {
                self.requests.push(Request::NotSent {
                    error: tr!("No account sends from this address any more."),
                    outgoing,
                });
            }
        }
    }

    fn tell_held(&mut self, accounts: &[AccountId]) {
        let stored = self
            .store
            .accounts()
            .inspect_err(|error| tracing::warn!("cannot read the accounts: {error}"))
            .unwrap_or_default();
        for account in stored.iter().filter(|known| accounts.contains(&known.id)) {
            let count = self
                .held
                .iter()
                .filter(|(id, command)| *id == account.id && matches!(command, Command::Send(_)))
                .count();
            if count == 0 {
                continue;
            }
            self.ui.present_sheet(
                Alert::new(
                    &trn!("A Message Is Waiting to Be Sent", "{count} Messages Are Waiting to Be Sent", count = count),
                    &tr!("{email} is turned off. Mail sends what is waiting when you turn the account on again in Mail Settings ▸ Accounts. If you quit Mail first, it opens again as an unsent message.", email = account.email.clone()),
                    |_| {},
                )
                .primary(&tr!("OK"), "ok"),
            );
        }
    }

    pub fn finish(&mut self) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        for worker in self.workers.drain(..) {
            worker.finish(deadline);
        }
        self.reader.remove_unedited_attachments();
    }

    pub fn load_remote(&mut self, id: MessageId) {
        let Some(sources) = self.reader.start_remote(id) else {
            return;
        };
        let notifier = self.mailroom.notifier();
        fetch_remote(sources, self.network.clone(), move |pictures| {
            notifier.send(MailEvent::Pictures(id, pictures));
        });
    }

    pub fn open_attachment(&mut self, index: usize) {
        let Some(summary) = self.reader.focused().map(|(summary, _)| summary.clone()) else {
            return;
        };
        let Some(folder) = bande_xdg::cache_home().map(|cache| {
            cache
                .join("bande-mail/attachments")
                .join(format!("{}-{}", summary.mailbox, summary.uid))
        }) else {
            return;
        };
        if let Some(path) = self.reader.open_attachment(folder, index) {
            self.requests.push(Request::OpenAttachment(path));
        }
    }

    pub fn save_attachment(&mut self, index: usize) {
        if let Err((name, error)) = self.reader.save_to_downloads(index) {
            self.requests.push(Request::Notify {
                summary: tr!(
                    "The attachment couldn’t be saved: {error}",
                    error = error.to_string()
                ),
                body: name,
                sound: false,
                message: None,
            });
        }
    }

    pub fn engine_ready(&mut self) -> bool {
        let events = self.mailroom.take();
        let mut changed = false;
        let mut stale = false;
        let mut held = Vec::new();
        for event in events {
            changed |= !matches!(event, MailEvent::Synced(..));
            match event {
                MailEvent::Status(account, status) => {
                    if status == Status::Syncing {
                        self.failures.retain(|(known, _)| *known != account);
                        self.retrying.retain(|known| *known != account);
                    }
                    self.statuses.retain(|(known, _)| *known != account);
                    self.statuses.push((account, status));
                }
                MailEvent::Changed(_) => stale = true,
                MailEvent::Synced(account, mailbox) => {
                    let (moves, waiting) = std::mem::take(&mut self.undoing)
                        .into_iter()
                        .partition(|moved| moved.from.account == account && moved.to == mailbox);
                    self.undoing = waiting;
                    if !moves.is_empty() {
                        changed = true;
                        self.put_back(moves);
                    }
                }
                MailEvent::Arrived(account, mut ids) => {
                    stale = true;
                    ids.retain(|id| !self.expected_arrival(*id));
                    let left = self.apply_rules(account, ids);
                    self.notify(&left);
                }
                MailEvent::Body(id) => {
                    if self
                        .reader
                        .thread
                        .as_ref()
                        .and_then(|thread| thread.entry(id))
                        .is_some_and(|entry| entry.reading.is_some())
                    {
                        self.read_entry(id);
                    }
                    stale = true;
                }
                MailEvent::LoginFailed(account, _) => {
                    if self.setup.is_none()
                        && let Some(account) = self.account(account)
                    {
                        let email = account.email.clone();
                        self.present_setup(Mode::Password(email));
                    }
                }
                MailEvent::Failed(account, error) => {
                    tracing::warn!("account {account}: {error}");
                    self.failures.retain(|(known, _)| *known != account);
                    self.failures.push((account, error));
                    if let Some(at) = self.retrying.iter().position(|known| *known == account) {
                        self.retrying.remove(at);
                        self.show_failure(account);
                    }
                }
                MailEvent::Sent(_) => {}
                MailEvent::MailboxRefused(reason) => {
                    self.ui.present_sheet(
                        Alert::new(&tr!("The mailbox could not be changed"), &reason, |_| {})
                            .primary(&tr!("OK"), "ok"),
                    );
                }
                MailEvent::NotSent(_, error, outgoing) => {
                    self.requests.push(Request::NotSent { error, outgoing });
                }
                MailEvent::Held(account, command) => {
                    self.held.push((account, *command));
                    held.push(account);
                }
                MailEvent::Setup(outcome) => self.setup_checked(*outcome),
                MailEvent::Found(found) => self.servers_found(*found),
                MailEvent::Opened(id, content) => self.opened(id, *content),
                MailEvent::LookedUp(id, result) => self.looked_up(id, result),
                MailEvent::KeysChanged(_) => {}
                MailEvent::Pictures(id, pictures) => self.reader.add_pictures(id, pictures),
            }
        }
        if !held.is_empty() {
            held.dedup();
            self.release_held();
            self.tell_held(&held);
        }
        if stale {
            self.reload();
        }
        changed
    }

    fn apply_rules_to_chosen(&mut self) {
        let mut accounts: Vec<(AccountId, Vec<MessageId>)> = Vec::new();
        for summary in self.targets() {
            match accounts
                .iter_mut()
                .find(|(account, _)| *account == summary.account)
            {
                Some((_, ids)) => ids.push(summary.id),
                None => accounts.push((summary.account, vec![summary.id])),
            }
        }
        for (account, ids) in accounts {
            self.apply_rules(account, ids);
        }
        self.reload();
    }

    fn apply_rules(&mut self, account: AccountId, mut ids: Vec<MessageId>) -> Vec<MessageId> {
        let rules = self
            .store
            .rules()
            .inspect_err(|error| tracing::warn!("cannot read the rules: {error}"))
            .unwrap_or_default();
        for rule in rules {
            if ids.is_empty() {
                break;
            }
            let matched = self
                .store
                .matching(&ids, &rule)
                .inspect_err(|error| tracing::warn!("cannot match a rule: {error}"))
                .unwrap_or_default();
            if matched.is_empty() {
                continue;
            }
            match rule.action {
                RuleAction::Move(target) => {
                    let Some(target) = self
                        .mailbox(target)
                        .filter(|target| target.account == account)
                    else {
                        continue;
                    };
                    match self.store.move_messages(&matched, &target) {
                        Ok(accounts) => self.flush(&accounts),
                        Err(error) => tracing::warn!("cannot apply a rule: {error}"),
                    }
                    ids.retain(|id| !matched.contains(id));
                }
                RuleAction::Flag => self.set_flag_on(&matched, Flag::Flagged, true),
                RuleAction::Read => {
                    self.set_flag_on(&matched, Flag::Seen, true);
                    ids.retain(|id| !matched.contains(id));
                }
            }
        }
        ids
    }

    fn notify(&mut self, ids: &[MessageId]) {
        if !self.settings.notify {
            return;
        }
        let sound = self.settings.sound;
        let arrived = ids
            .iter()
            .filter_map(|id| {
                self.store
                    .summary(*id)
                    .inspect_err(|error| tracing::warn!("cannot read a new message: {error}"))
                    .ok()
                    .flatten()
            })
            .collect::<Vec<_>>();
        if arrived.len() > NOTIFIED_AT_ONCE {
            self.requests.push(Request::Notify {
                summary: tr!("{count} new messages", count = arrived.len()),
                body: arrived
                    .iter()
                    .take(NOTIFIED_AT_ONCE)
                    .map(|summary| summary.sender.shown().to_owned())
                    .collect::<Vec<_>>()
                    .join(", "),
                sound,
                message: None,
            });
            return;
        }
        for summary in arrived {
            self.requests.push(Request::Notify {
                summary: summary.sender.shown().to_owned(),
                body: match summary.subject.as_str() {
                    "" => tr!("No Subject"),
                    subject => subject.to_owned(),
                },
                sound,
                message: Some(summary.id),
            });
        }
    }

    pub fn has_sheet(&self) -> bool {
        self.ui.has_sheet()
    }

    pub fn present_setup(&mut self, mode: Mode) {
        let state: Shared = Rc::new(RefCell::new(SheetState::default()));
        if let Some(previous) = &self.setup {
            let previous = previous.state.borrow();
            let mut fresh = state.borrow_mut();
            fresh.values = previous.values.clone();
            fresh.password = previous.password.clone();
        }
        if let Some(previous) = self.setup.take() {
            previous.state.borrow_mut().closed = true;
        }
        let sheet = AccountSheet::new(
            Rc::clone(&state),
            mode.clone(),
            include_bytes!("../../data/org.bande.Mail.svg"),
            bande_xdg::is_installed("org.bande.Notes"),
        );
        let password_field = sheet.password_field();
        self.ui.present_sheet(sheet);
        self.setup = Some(SetupSheet {
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
            (Action::Other, Mode::Servers) => self.present_setup(Mode::Address),
            (Action::Other, Mode::Password(_)) => {
                self.close_setup();
                self.requests.push(Request::OpenSettings);
            }
            (Action::Other, Mode::Kind) => {}
        }
    }

    fn renew_password(&mut self, state: &Shared, email: &str) {
        let password = state.borrow().password.clone();
        if password.is_empty() {
            state.borrow_mut().status = Some((tr!("Enter your password."), true));
            return;
        }
        let Some(account) = self
            .accounts
            .iter()
            .map(|(account, _)| account.clone())
            .find(|account| account.email == email)
        else {
            return;
        };
        let host = account.imap.host.clone();
        let saved = password.clone();
        std::thread::spawn(move || {
            if let Err(error) = Keyring::open(crate::KEYRING).and_then(|keyring| {
                keyring.save(
                    &crate::keyring_key(&account.email, &host),
                    &crate::keyring_label(&account.email),
                    &saved,
                )
            }) {
                tracing::warn!("cannot save the password: {error}");
            }
        });
        self.send_command(account.id, Command::Password(password.to_string()));
        self.close_setup();
    }

    fn address_problem(&self, state: &Shared) -> Option<Text> {
        let state = state.borrow();
        let email = state.value(Field::Email);
        if state.value(Field::Name).is_empty() || bande_discovery::domain_of(&email).is_none() {
            Some(text!("Enter your name and email address."))
        } else if state.password.is_empty() {
            Some(text!("Enter your password."))
        } else if self
            .store
            .accounts()
            .unwrap_or_default()
            .iter()
            .any(|account| account.email.eq_ignore_ascii_case(&email))
        {
            Some(text!("This account is already set up."))
        } else {
            None
        }
    }

    fn start_check(&self, state: &Shared, status: String) -> bool {
        if let Some(problem) = self.address_problem(state) {
            state.borrow_mut().status = Some((problem.tr(), true));
            return false;
        }
        let mut state = state.borrow_mut();
        state.busy = true;
        state.status = Some((status, false));
        true
    }

    fn find_servers(&mut self, state: &Shared) {
        if !self.start_check(state, tr!("Looking up the servers…")) {
            return;
        }
        let email = state.borrow().value(Field::Email);
        let network = self.network.clone();
        let notifier = self.mailroom.notifier();
        std::thread::spawn(move || {
            notifier.send(MailEvent::Found(Box::new(find(&email, &network))));
        });
    }

    fn servers_found(&mut self, found: Found) {
        if self.setup.as_ref().map(|setup| &setup.mode) != Some(&Mode::Address) {
            return;
        }
        if let Some((settings, _)) = &found.settings
            && let Some(setup) = &self.setup
        {
            setup.state.borrow_mut().fill(settings);
        }
        self.present_setup(Mode::Servers);
        let message = found.message();
        if let Some(setup) = &self.setup {
            setup.state.borrow_mut().status = Some((message, false));
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
        let (email, password) = {
            let state = state.borrow();
            (state.value(Field::Email), state.password.to_string())
        };
        let network = self.network.clone();
        let notifier = self.mailroom.notifier();
        std::thread::spawn(move || {
            let outcome = check(&servers, &password, &network);
            if let Outcome::Ready(settings) = &outcome
                && let Err(error) = Keyring::open(crate::KEYRING).and_then(|keyring| {
                    keyring.save(
                        &crate::keyring_key(&email, &settings.imap.host),
                        &crate::keyring_label(&email),
                        &password,
                    )
                })
            {
                tracing::warn!("cannot save the password: {error}");
            }
            notifier.send(MailEvent::Setup(Box::new(outcome)));
        });
    }

    fn setup_checked(&mut self, outcome: Outcome) {
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
                state.password.clone(),
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
        let id = match self.store.add_account(&account) {
            Ok(id) => id,
            Err(error) => {
                state.borrow_mut().status = Some((error.to_string(), true));
                return;
            }
        };
        if state.borrow().notes()
            && bande_xdg::is_installed("org.bande.Notes")
            && let Err(error) = self.store.set_notes(id, true)
        {
            tracing::warn!("cannot use {} for Notes: {error}", account.email);
        }
        let account = Account { id, ..account };
        let first = Zeroizing::new(password.to_string());
        let mut handed = false;
        let fallback = keyring_password(account.email.clone(), account.imap.host.clone());
        let mut fallback = Some(fallback);
        let source: crate::worker::PasswordSource = Box::new(move || {
            if !handed {
                handed = true;
                return Ok(Some(first.to_string()));
            }
            match fallback.as_mut() {
                Some(fallback) => fallback(),
                None => Ok(None),
            }
        });
        self.workers.push(Worker::spawn(
            account,
            self.store_path.clone(),
            source,
            self.network.clone(),
            self.mailroom.notifier(),
        ));
        self.close_setup();
        self.reload();
    }

    pub fn remove_account(&mut self, id: AccountId) {
        let Some(account) = self
            .store
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .find(|account| account.id == id)
        else {
            return;
        };
        self.workers.retain(|worker| worker.account != id);
        std::thread::spawn(crate::offer::forget_in_search);
        if let Err(error) = self.store.remove_account(id) {
            tracing::warn!("cannot remove {}: {error}", account.email);
        }
        self.release_held();
        std::thread::spawn(move || {
            if let Err(error) = Keyring::open(crate::KEYRING).and_then(|keyring| {
                keyring.forget(&crate::keyring_key(&account.email, &account.imap.host))
            }) {
                tracing::warn!("cannot forget the password: {error}");
            }
        });
        self.reload();
    }

    pub(crate) fn after_ui(&mut self) {
        self.sheet_action();
        let signals = std::mem::take(&mut *self.signals.borrow_mut());
        for signal in signals {
            match signal {
                Signal::GetMail => self.get_mail(),
                Signal::Compose => self.compose(Draft::default()),
                Signal::Archive => self.move_selected(Role::Archive),
                Signal::Delete => self.delete_selected(),
                Signal::Junk => self.move_selected(Role::Junk),
                Signal::Reply => self.respond(false),
                Signal::ReplyAll => self.respond(true),
                Signal::Forward => self.forward_selected(),
                Signal::Flag => self.toggle_flag(),
                Signal::Search(text) => self.search_for(&text),
                signal @ (Signal::NewMailbox(..)
                | Signal::RenameMailbox(..)
                | Signal::DeleteMailbox(_)) => self.change_mailboxes(signal),
                Signal::TryAgain(account) => {
                    self.retrying.push(account);
                    self.send_command(account, Command::Sync);
                }
                Signal::ServerSettings => self.requests.push(Request::OpenSettings),
                Signal::OpenLink(url) => self.requests.push(Request::OpenLink(url)),
                Signal::TrustIssuer(id) => self.trust_issuer(id),
            }
        }
        if let Some(request) = self.ui.take_clipboard_request() {
            self.requests.push(Request::Clipboard(request));
        }
        if let Some((menu, at)) = self.ui.take_menu_request() {
            self.requests.push(Request::ShowMenu(menu, at));
        }
        self.refresh_toolbar();
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        let handled = self.route(event);
        self.after_ui();
        self.list_more();
        handled
    }

    fn list_more(&mut self) {
        let scroll = &self.scrolls[1];
        if self.listed_all
            || scroll.limit() <= 0.0
            || scroll.offset(self.ui.context()) < scroll.limit() - LIST_AHEAD
        {
            return;
        }
        self.listed += LIST_PAGE;
        self.reload();
    }

    fn route(&mut self, event: &Event) -> bool {
        if self.ui.has_sheet() {
            return self.ui.handle(event);
        }
        match event {
            Event::Key { key, modifiers } => self.key(*key, *modifiers),
            Event::PointerDown(pointer) => {
                if self.over_control(pointer.position) {
                    return self.ui.handle(event);
                }
                self.ui.clear_focus();
                self.click(pointer)
            }
            Event::PointerMove(pointer) if self.reader.selecting => {
                let frame = self.reader_frame();
                self.reader
                    .extend_selection(self.ui.context(), frame, pointer.position)
            }
            Event::PointerUp(_) if self.reader.selecting => {
                if let Some(text) = self.reader.finish_selection() {
                    self.requests
                        .push(Request::Clipboard(ClipboardRequest::SelectPrimary(text)));
                }
                true
            }
            Event::PointerUp(_) if self.drag.is_some() => {
                self.drop_messages();
                true
            }
            Event::PointerUp(_) => self.ui.handle(event),
            Event::PointerMove(pointer) if self.drag.is_some() => self.drag_to(pointer.position),
            Event::PointerMove(pointer) | Event::PointerEnter(pointer) => {
                let ui = self.ui.handle(event);
                let hover = self.hit(pointer.position);
                let changed = hover != self.hover;
                self.hover = hover;
                ui | changed
            }
            Event::PointerLeave => {
                self.ui.handle(event) | self.hover.take().is_some() | self.drag.take().is_some()
            }
            Event::Scroll(scroll) => {
                self.ui.handle(event)
                    || (!self.over_control(scroll.position) && self.scroll(scroll))
            }
            _ => self.ui.handle(event),
        }
    }

    fn drop_target(&self, index: usize) -> bool {
        match self.sidebar.get(index) {
            Some(Row::Place { place, .. }) if *place == self.place => false,
            Some(Row::Place {
                place: Place::Mailbox(id),
                ..
            }) => self
                .accounts
                .iter()
                .flat_map(|(_, mailboxes)| mailboxes)
                .any(|mailbox| mailbox.id == *id && mailbox.selectable),
            Some(Row::Place {
                place: Place::All(role),
                ..
            }) => *role != Role::Drafts && *role != Role::Sent,
            _ => false,
        }
    }

    fn drag_to(&mut self, at: Point) -> bool {
        let over = match self.hit(at) {
            Some(Hit::Place(index)) if self.drop_target(index) => Some(index),
            _ => None,
        };
        let Some(drag) = self.drag.as_mut() else {
            return false;
        };
        let moving = drag.moving || past_drag_slop(drag.from, at);
        let changed = moving && (drag.at != at || drag.over != over || !drag.moving);
        *drag = Drag {
            at,
            moving,
            over: over.filter(|_| moving),
            ..*drag
        };
        changed
    }

    fn drop_messages(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if let (false, Some(id)) = (drag.moving, drag.collapse) {
            self.select(id);
        }
        let (true, Some(index)) = (drag.moving, drag.over) else {
            return;
        };
        match self.sidebar.get(index).cloned() {
            Some(Row::Place {
                place: Place::Mailbox(id),
                ..
            }) => {
                let target = self
                    .accounts
                    .iter()
                    .flat_map(|(_, mailboxes)| mailboxes)
                    .find(|mailbox| mailbox.id == id)
                    .cloned();
                if let Some(target) = target {
                    self.move_selected_to(&target);
                }
            }
            Some(Row::Place {
                place: Place::All(Role::Trash),
                ..
            }) => self.delete_selected(),
            Some(Row::Place {
                place: Place::All(role),
                ..
            }) => self.move_selected(role),
            _ => {}
        }
    }

    fn search_focused(&self) -> bool {
        self.ui.focus() == Some(self.toolbar.search)
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        if modifiers.logo {
            return false;
        }
        let bare = !modifiers.control && !modifiers.alt;
        if key == Key::Function(6) && bare {
            return self.cycle_pane(!modifiers.shift);
        }
        if self.ui.focus().is_some() {
            if key == Key::Escape {
                self.ui.clear_focus();
                return true;
            }
            if self.search_focused() || key == Key::Tab {
                return self.ui.handle(&Event::Key { key, modifiers });
            }
        }
        if key == Key::Tab && bare {
            return (self.focus == Pane::Reader && self.step_reader_target(!modifiers.shift))
                || self.cycle_pane(!modifiers.shift);
        }
        if self.focus == Pane::Reader && modifiers.control && !modifiers.shift && !modifiers.alt {
            match key {
                Key::Character('c' | 'C') => return self.copy_selection(),
                Key::Character('a' | 'A') => {
                    self.reader.select_entry_text();
                    return true;
                }
                _ => {}
            }
        }
        let event = Event::Key { key, modifiers };
        if let Some((action, _)) = SHORTCUTS
            .iter()
            .find(|(_, shortcut)| shortcut_pressed(&event, shortcut))
        {
            return self.menu_action(action);
        }
        let plain = !modifiers.control;
        match key {
            Key::Character('z' | 'Z') if modifiers.control && !modifiers.shift => {
                self.undo();
                true
            }
            Key::Character('a' | 'A') if modifiers.control && !modifiers.shift => {
                self.select_all();
                true
            }
            Key::Up | Key::Down | Key::Home | Key::End | Key::PageUp | Key::PageDown
                if plain && self.focus != Pane::List =>
            {
                self.move_in_pane(key)
            }
            Key::Home if plain => self.step(isize::MIN),
            Key::End if plain => self.step(isize::MAX),
            Key::PageUp | Key::PageDown if plain => {
                let page = self.rows_per_page();
                self.step(if key == Key::PageUp { -page } else { page })
            }
            Key::Up if plain && modifiers.shift => self.extend(-1),
            Key::Down if plain && modifiers.shift => self.extend(1),
            Key::Enter if plain && self.focus == Pane::Sidebar => {
                self.focus = Pane::List;
                true
            }
            _ if opens_menu(key, modifiers) => self.keyboard_menu(),
            Key::Escape if self.focus == Pane::Reader && self.reader.target.is_some() => {
                self.reader.target = None;
                true
            }
            Key::Enter | Key::Space
                if plain && self.focus == Pane::Reader && self.reader.target.is_some() =>
            {
                self.press_reader_target(false)
            }
            Key::Backspace | Key::Delete if self.focus == Pane::Sidebar => false,
            Key::Right | Key::Left if plain && !modifiers.shift => {
                let expanded = self.selected.is_some_and(|id| {
                    self.conversations.children.contains(&id)
                        || self
                            .conversations
                            .earlier
                            .get(&id)
                            .is_some_and(|(key, _)| self.conversations.expanded.contains(key))
                });
                match (key == Key::Right, expanded, self.selected) {
                    (true, false, Some(id)) | (false, true, Some(id)) => {
                        self.toggle_conversation(id)
                    }
                    _ => false,
                }
            }
            Key::Up if plain => self.step(-1),
            Key::Down if plain => self.step(1),
            Key::Backspace | Key::Delete => {
                self.delete_selected();
                true
            }
            Key::Space if plain => self.page_reader(),
            Key::Enter if plain => {
                self.continue_draft();
                true
            }
            _ => self.ui.handle(&event),
        }
    }

    fn cycle_pane(&mut self, forward: bool) -> bool {
        const PANES: [Pane; 3] = [Pane::Sidebar, Pane::List, Pane::Reader];
        if self.ui.focus().is_some() {
            self.ui.clear_focus();
            self.focus = match forward {
                true => Pane::Sidebar,
                false => Pane::Reader,
            };
            if !forward && let Some(&last) = self.reader.stops().last() {
                self.reader.set_target(self.ui.context(), last);
            }
            return true;
        }
        let index = PANES
            .iter()
            .position(|pane| *pane == self.focus)
            .unwrap_or(1);
        let next = match forward {
            true => Some(index + 1).filter(|next| *next < PANES.len()),
            false => index.checked_sub(1),
        };
        match next {
            Some(next) => self.focus = PANES[next],
            None => {
                let modifiers = Modifiers {
                    shift: !forward,
                    ..Modifiers::default()
                };
                self.ui.handle(&Event::Key {
                    key: Key::Tab,
                    modifiers,
                });
            }
        }
        true
    }

    fn move_in_pane(&mut self, key: Key) -> bool {
        match self.focus {
            Pane::Sidebar => {
                let page = self.sidebar_rows_per_page();
                self.step_place(match key {
                    Key::Up => -1,
                    Key::Down => 1,
                    Key::Home => isize::MIN,
                    Key::End => isize::MAX,
                    Key::PageUp => -page,
                    _ => page,
                })
            }
            Pane::Reader => self.scroll_reader(key),
            Pane::List => false,
        }
    }

    fn step_reader_target(&mut self, forward: bool) -> bool {
        self.reader.step_target(self.ui.context(), forward)
    }

    fn press_reader_target(&mut self, secondary: bool) -> bool {
        let frame = self.reader_frame();
        let Some((target, pointer)) =
            self.reader
                .pressed_target(self.ui.context(), frame, secondary)
        else {
            return false;
        };
        self.click_reader(target, &pointer);
        true
    }

    fn step_place(&mut self, by: isize) -> bool {
        let places = self
            .sidebar
            .iter()
            .enumerate()
            .filter_map(|(index, row)| match row {
                Row::Place { place, .. } => Some((index, place.clone())),
                Row::Heading(_) => None,
            })
            .collect::<Vec<_>>();
        if places.is_empty() {
            return false;
        }
        let next = places
            .iter()
            .position(|(_, place)| *place == self.place)
            .map_or(0, |current| {
                current.saturating_add_signed(by).min(places.len() - 1)
            });
        let (index, place) = places[next].clone();
        if place == self.place {
            return false;
        }
        self.choose_place(place);
        self.focus = Pane::Sidebar;
        self.reveal_place(index);
        true
    }

    fn keyboard_menu(&mut self) -> bool {
        let shown = match self.focus {
            Pane::Sidebar => self.sidebar.iter().enumerate().find_map(|(index, row)| {
                let Row::Place { place, .. } = row else {
                    return None;
                };
                (*place == self.place)
                    .then(|| self.mailbox_menu(place))
                    .flatten()
                    .map(|menu| (Hit::Place(index), menu))
            }),
            Pane::List => self
                .selected
                .map(|id| (Hit::Message(id), self.message_menu())),
            Pane::Reader => {
                if self.press_reader_target(true) {
                    return true;
                }
                let frame = self.reader_frame();
                let menu = self.reader.text_menu();
                self.requests.push(Request::ShowMenu(
                    menu,
                    Point::new(frame.x + frame.width / 2.0, frame.y + frame.height / 3.0),
                ));
                return true;
            }
        };
        let Some((rect, menu)) = shown.and_then(|(wanted, menu)| {
            self.hits
                .iter()
                .find(|(_, hit)| *hit == wanted)
                .map(|(rect, _)| (*rect, menu))
        }) else {
            return false;
        };
        self.requests.push(Request::ShowMenu(
            menu,
            Point::new(rect.x + rect.width / 2.0, rect.y + rect.height),
        ));
        true
    }

    fn select_all(&mut self) {
        let Some(first) = self
            .selected
            .or_else(|| self.messages.first().map(|summary| summary.id))
        else {
            return;
        };
        if self.selected.is_none() {
            self.select(first);
        }
        self.focus = Pane::List;
        self.also = self
            .messages
            .iter()
            .map(|summary| summary.id)
            .filter(|id| *id != first)
            .collect();
        if !self.also.is_empty() {
            self.reader.thread = None;
            self.reader.layout = None;
        }
    }

    fn extend(&mut self, by: isize) -> bool {
        let chosen = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, summary)| self.is_chosen(summary.id))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let (Some(first), Some(last)) = (chosen.first(), chosen.last()) else {
            return self.step(by);
        };
        let next = match by < 0 {
            true => first.checked_sub(1),
            false => Some(last + 1).filter(|next| *next < self.messages.len()),
        };
        let Some(id) = next.map(|next| self.messages[next].id) else {
            return false;
        };
        self.select_more(id, false);
        true
    }

    fn step(&mut self, by: isize) -> bool {
        if self.messages.is_empty() {
            return false;
        }
        let current = self
            .selected
            .and_then(|id| self.messages.iter().position(|summary| summary.id == id));
        let next = match current {
            Some(current) => current
                .saturating_add_signed(by)
                .min(self.messages.len() - 1),
            None => 0,
        };
        let id = self.messages[next].id;
        if Some(id) == self.selected {
            return false;
        }
        self.select(id);
        true
    }

    fn click(&mut self, pointer: &Pointer) -> bool {
        let Some(hit) = self.hit(pointer.position) else {
            return false;
        };
        let secondary = pointer.button == PointerButton::Secondary;
        match hit {
            Hit::Place(index) => {
                self.focus = Pane::Sidebar;
                if let Some(Row::Place { place, .. }) = self.sidebar.get(index).cloned() {
                    if secondary && let Some(menu) = self.mailbox_menu(&place) {
                        self.requests
                            .push(Request::ShowMenu(menu, pointer.position));
                    } else {
                        self.choose_place(place);
                    }
                }
            }
            Hit::Message(id)
                if !secondary && (pointer.modifiers.control || pointer.modifiers.shift) =>
            {
                self.select_more(id, pointer.modifiers.shift);
            }
            Hit::Message(id) => {
                self.focus = Pane::List;
                let collapse = !secondary && self.is_chosen(id) && !self.also.is_empty();
                if !self.is_chosen(id) {
                    self.select(id);
                }
                if !secondary && pointer.count == 1 {
                    self.drag = Some(Drag {
                        from: pointer.position,
                        at: pointer.position,
                        moving: false,
                        over: None,
                        collapse: collapse.then_some(id),
                    });
                }
                if pointer.count >= 2 && !secondary {
                    self.continue_draft();
                }
                if secondary {
                    let menu = self.message_menu();
                    self.requests
                        .push(Request::ShowMenu(menu, pointer.position));
                }
            }
            Hit::Conversation(id) => {
                self.focus = Pane::List;
                self.toggle_conversation(id);
            }
            Hit::Reader(target) => {
                self.focus = Pane::Reader;
                self.reader.target = None;
                if matches!(target, Target::Card(_)) && !secondary {
                    let frame = self.reader_frame();
                    self.reader
                        .start_selection(self.ui.context(), frame, pointer);
                }
                self.click_reader(target, pointer);
            }
            Hit::Offline(account) => self.show_failure(account),
        }
        true
    }

    pub(crate) fn click_reader(&mut self, target: Target, pointer: &Pointer) {
        let secondary = pointer.button == PointerButton::Secondary;
        match target {
            Target::Card(id) => {
                self.reader.focus_entry(id);
                if secondary {
                    let menu = self.reader.text_menu();
                    self.requests
                        .push(Request::ShowMenu(menu, pointer.position));
                }
            }
            Target::Expand(id) => self.toggle_entry(id),
            Target::Quotes(id) => self.reader.toggle_quotes(id),
            Target::Respond(id, respond) => {
                self.reader.focus_entry(id);
                match respond {
                    Respond::Reply => self.respond(false),
                    Respond::ReplyAll => self.respond(true),
                    Respond::Forward => self.forward_selected(),
                }
            }
            Target::Link(url, ..) if secondary => {
                self.requests
                    .push(Request::ShowMenu(link_menu(&url), pointer.position));
            }
            Target::Link(url, claimed, _) => self.open_link(url, claimed),
            Target::Address(address) => {
                self.requests
                    .push(Request::ShowMenu(address_menu(&address), pointer.position));
            }
            Target::Attachment(id, index) if secondary => {
                self.reader.focus_entry(id);
                self.requests
                    .push(Request::ShowMenu(attachment_menu(index), pointer.position));
            }
            Target::Attachment(id, index) => {
                self.reader.focus_entry(id);
                self.open_attachment(index);
            }
            Target::LookUpKey(id) => self.look_up_key(id),
            Target::TrustIssuer(id) => self.confirm_trust_issuer(id),
            Target::Unseal(id) => self.unseal(id),
            Target::LoadRemote(id) => {
                let sender = self
                    .reader
                    .thread
                    .as_ref()
                    .and_then(|thread| thread.entry(id))
                    .and_then(|entry| entry.reading.as_ref())
                    .map(|reading| reading.content.envelope.sender().email)
                    .filter(|email| !email.is_empty());
                if let Some(email) = sender
                    && let Err(error) = self.store.trust_remote(&email)
                {
                    tracing::warn!("cannot remember {email}: {error}");
                }
                self.load_remote(id);
            }
        }
    }

    fn open_link(&mut self, url: String, claimed: Option<String>) {
        let Some(claimed) = claimed else {
            return self.requests.push(Request::OpenLink(url));
        };
        let signals = Rc::clone(&self.signals);
        self.ui.present_sheet(link_alert(url, &claimed, move |url| {
            signals.borrow_mut().push(Signal::OpenLink(url));
        }));
    }

    fn copy_selection(&mut self) -> bool {
        let Some(text) = self.reader.selected_text() else {
            return false;
        };
        self.requests
            .push(Request::Clipboard(ClipboardRequest::Copy(text)));
        true
    }

    fn mailbox(&self, id: MailboxId) -> Option<Mailbox> {
        self.accounts
            .iter()
            .flat_map(|(_, mailboxes)| mailboxes)
            .find(|mailbox| mailbox.id == id)
            .cloned()
    }

    pub fn current_account(&self) -> Option<AccountId> {
        match self.place {
            Place::Mailbox(id) => self.mailbox(id).map(|mailbox| mailbox.account),
            _ => None,
        }
        .or_else(|| self.accounts.first().map(|(account, _)| account.id))
    }

    fn mailbox_menu(&self, place: &Place) -> Option<Menu> {
        let Place::Mailbox(id) = place else {
            return None;
        };
        let mailbox = self.mailbox(*id)?;
        let own = mailbox.role.is_none();
        Some(Menu::new(vec![
            MenuItem::new(
                &tr!("New Mailbox…"),
                &format!("mailbox-new:{}", mailbox.account),
            ),
            MenuItem::new(&tr!("Rename Mailbox…"), &format!("mailbox-rename:{id}")).enabled(own),
            MenuItem::new(&tr!("Delete Mailbox…"), &format!("mailbox-delete:{id}")).enabled(own),
        ]))
    }

    fn ask_mailbox_name(
        &mut self,
        title: &str,
        name: &str,
        done: impl Fn(String) -> Signal + 'static,
    ) {
        let typed = Rc::new(RefCell::new(name.to_owned()));
        let field = {
            let typed = Rc::clone(&typed);
            TextField::new(&tr!("Name"), name, move |text| {
                *typed.borrow_mut() = text.to_owned();
            })
        };
        let signals = Rc::clone(&self.signals);
        let alert = Alert::new(title, "", move |choice| {
            let name = typed.borrow().trim().to_owned();
            if choice == "ok" && !name.is_empty() {
                signals.borrow_mut().push(done(name));
            }
        })
        .accessory(field)
        .primary(&tr!("OK"), "ok")
        .cancel(&tr!("Cancel"), "cancel");
        self.ui.present_sheet(alert);
    }

    fn mailbox_action(&mut self, verb: &str, id: i64) {
        match verb {
            "mailbox-new" => {
                self.ask_mailbox_name(&tr!("New Mailbox"), "", move |name| {
                    Signal::NewMailbox(id, name)
                });
            }
            "mailbox-rename" => {
                let Some(mailbox) = self.mailbox(id) else {
                    return;
                };
                let title = tr!("Rename “{name}”", name = mailbox.name.clone());
                self.ask_mailbox_name(&title, &mailbox.name, move |name| {
                    Signal::RenameMailbox(id, name)
                });
            }
            "mailbox-delete" => {
                let Some(mailbox) = self.mailbox(id) else {
                    return;
                };
                let signals = Rc::clone(&self.signals);
                let alert = Alert::new(
                    &tr!("Delete “{name}”?", name = mailbox.name),
                    &tr!("The mailbox and every message in it are deleted from the server."),
                    move |choice| {
                        if choice == "delete" {
                            signals.borrow_mut().push(Signal::DeleteMailbox(id));
                        }
                    },
                )
                .destructive(&tr!("Delete"), "delete")
                .cancel(&tr!("Cancel"), "cancel");
                self.ui.present_sheet(alert);
            }
            _ => {}
        }
    }

    fn change_mailboxes(&mut self, signal: Signal) {
        let (account, change) = match signal {
            Signal::NewMailbox(account, name) => {
                let prefix = self.namespace_prefix(account);
                (account, MailboxChange::Create(format!("{prefix}{name}")))
            }
            Signal::RenameMailbox(id, name) => {
                let Some(mailbox) = self.mailbox(id) else {
                    return;
                };
                let parent = mailbox
                    .delimiter
                    .and_then(|delimiter| {
                        mailbox
                            .path
                            .rsplit_once(delimiter)
                            .map(|(parent, _)| format!("{parent}{delimiter}"))
                    })
                    .unwrap_or_default();
                (
                    mailbox.account,
                    MailboxChange::Rename(mailbox.path, format!("{parent}{name}")),
                )
            }
            Signal::DeleteMailbox(id) => {
                let Some(mailbox) = self.mailbox(id) else {
                    return;
                };
                (mailbox.account, MailboxChange::Delete(mailbox.path))
            }
            _ => return,
        };
        self.send_command(account, Command::Mailbox(change));
    }

    fn namespace_prefix(&self, account: AccountId) -> String {
        let Some((_, mailboxes)) = self.accounts.iter().find(|(known, _)| known.id == account)
        else {
            return String::new();
        };
        let inbox = mailboxes
            .iter()
            .find(|mailbox| mailbox.role == Some(Role::Inbox));
        let Some(prefix) = inbox.and_then(|inbox| {
            inbox
                .delimiter
                .map(|delimiter| format!("{}{delimiter}", inbox.path))
        }) else {
            return String::new();
        };
        let others = mailboxes
            .iter()
            .filter(|mailbox| mailbox.role != Some(Role::Inbox))
            .collect::<Vec<_>>();
        match !others.is_empty()
            && others
                .iter()
                .all(|mailbox| mailbox.path.starts_with(&prefix))
        {
            true => prefix,
            false => String::new(),
        }
    }

    pub fn message_menu(&self) -> Menu {
        let summary = self.first_target();
        let read = summary.as_ref().is_some_and(|summary| summary.flags.seen);
        let flagged = summary
            .as_ref()
            .is_some_and(|summary| summary.flags.flagged);
        Menu::new(vec![
            MenuItem::new(&tr!("Reply"), "reply"),
            MenuItem::new(&tr!("Reply All"), "reply-all"),
            MenuItem::new(&tr!("Forward"), "forward"),
            MenuItem::separator(),
            MenuItem::new(
                &if read {
                    tr!("Mark as Unread")
                } else {
                    tr!("Mark as Read")
                },
                "toggle-read",
            ),
            MenuItem::new(
                &if flagged { tr!("Unflag") } else { tr!("Flag") },
                "toggle-flag",
            ),
            MenuItem::submenu(
                &tr!("Flag"),
                Menu::new(
                    Self::flag_colors()
                        .into_iter()
                        .map(|(name, action)| MenuItem::new(&name, &action))
                        .collect(),
                ),
            ),
            MenuItem::separator(),
            MenuItem::new(&tr!(context = "action", "Archive"), "archive")
                .enabled(self.can_move_to(Role::Archive)),
            MenuItem::new(&tr!("Move to Junk"), "junk").enabled(self.can_move_to(Role::Junk)),
            MenuItem::submenu(
                &tr!("Move To"),
                Menu::new(
                    self.move_targets()
                        .into_iter()
                        .map(|(name, id)| MenuItem::new(&name, &format!("move-to:{id}")))
                        .collect(),
                ),
            ),
            MenuItem::new(&tr!("Delete"), "delete"),
        ])
    }

    pub fn menu_action(&mut self, action: &str) -> bool {
        match action {
            "reply" => self.respond(false),
            "reply-all" => self.respond(true),
            "forward" => self.forward_selected(),
            "toggle-read" => self.toggle_read(),
            "toggle-flag" => self.toggle_flag(),
            "mark-all-read" => self.mark_all_read(),
            action if action.starts_with("flag-color:") => {
                if let Some(color) = action
                    .strip_prefix("flag-color:")
                    .and_then(|color| color.parse::<u8>().ok())
                    .filter(|color| usize::from(*color) < FLAG_COLORS.len())
                {
                    self.flag_with(color);
                }
            }
            "apply-rules" => self.apply_rules_to_chosen(),
            "bcc-field" => {
                self.settings.bcc_field = !self.settings.bcc_field;
                self.keep_settings();
            }
            "sort-ascending" | "sort-descending" => {
                self.settings.ascending = action == "sort-ascending";
                self.keep_settings();
            }
            action if action.starts_with("sort-") => {
                if let Some(sort) = SortBy::ALL
                    .into_iter()
                    .find(|sort| action.strip_prefix("sort-") == Some(sort.name()))
                {
                    self.settings.sort = sort;
                    self.keep_settings();
                }
            }
            "archive" => self.move_selected(Role::Archive),
            "junk" => self.move_selected(Role::Junk),
            "delete" => self.delete_selected(),
            "compose" => self.compose(Draft::default()),
            "get-mail" => self.get_mail(),
            "add-account" => self.present_setup(Mode::Kind),
            "new-mailbox" => {
                if let Some(account) = self.current_account() {
                    self.mailbox_action("mailbox-new", account);
                }
            }
            "find" => self.ui.set_focus(self.toolbar.search),
            "copy-selection" => {
                self.copy_selection();
            }
            "select-text" => self.reader.select_entry_text(),
            action if action.starts_with("copy:") => {
                let text = action["copy:".len()..].to_owned();
                self.requests
                    .push(Request::Clipboard(ClipboardRequest::Copy(text)));
            }
            action if action.starts_with("open-link:") => {
                let url = action["open-link:".len()..].to_owned();
                let claimed = self.reader.claimed_host(&url);
                self.open_link(url, claimed);
            }
            action if action.starts_with("compose-to:") => self.compose(Draft {
                to: parse_addresses(&action["compose-to:".len()..]),
                ..Draft::default()
            }),
            action if action.starts_with("move-to:") => {
                let target = action
                    .strip_prefix("move-to:")
                    .and_then(|id| id.parse::<MailboxId>().ok())
                    .and_then(|id| {
                        self.accounts
                            .iter()
                            .flat_map(|(_, mailboxes)| mailboxes)
                            .find(|mailbox| mailbox.id == id)
                            .cloned()
                    });
                if let Some(target) = target {
                    self.move_selected_to(&target);
                }
            }
            action if action.starts_with("mailbox-") => {
                if let Some((verb, id)) = action
                    .split_once(':')
                    .and_then(|(verb, id)| Some((verb, id.parse().ok()?)))
                {
                    self.mailbox_action(verb, id);
                }
            }
            action if action.starts_with("attachment-") => {
                let parsed = action
                    .split_once(':')
                    .and_then(|(verb, index)| Some((verb, index.parse::<usize>().ok()?)));
                match parsed {
                    Some(("attachment-open", index)) => self.open_attachment(index),
                    Some(("attachment-save", index)) => self.save_attachment(index),
                    Some(("attachment-save-as", index)) => {
                        if let Some((name, data)) = self.reader.attachment(index) {
                            self.requests.push(Request::SaveAttachment(name, data));
                        }
                    }
                    _ => {}
                }
            }
            action => {
                let handled = self.ui.menu_action(action);
                self.after_ui();
                return handled;
            }
        }
        self.after_ui();
        true
    }

    pub fn hit(&self, at: Point) -> Option<Hit> {
        self.hits
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(at))
            .map(|(_, hit)| hit.clone())
    }
}
