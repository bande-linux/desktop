use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, Settings, View, WindowId};
use bande_app_menu::{Entry, edit_submenu};
use bande_inbox::Inbox;
use bande_l10n::{Text, text, tr};
use bande_mail_core::accounts::{self, Account, AccountId};
use bande_mail_core::net::Network;
use bande_mail_core::sheet::Mode;
use bande_notes::settings::{AccountsSettings, CHANGES, SETTINGS_SIZE};
use bande_notes::store::{Store, store_path};
use bande_notes::view::{NotesView, Request, WINDOW_SIZE};
use bande_notes::worker::{Command, Event, Worker};
use bande_toolkit::Appearance;
use bande_toolkit::ui::ListMarker;

static INFO: Info = Info {
    id: "org.bande.Notes",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || tr!("Notes"),
    keys: Keys::Standard,
    settings: Some(Settings {
        title: || tr!("Notes Settings"),
        size: SETTINGS_SIZE,
        view: settings_window,
    }),
};

const NEW_NOTE: &str = "--new-note";

fn settings_window(appearance: Appearance) -> Box<dyn View> {
    Box::new(AccountsSettings::new(appearance))
}

fn item(label: Text, action: &str) -> Entry {
    Entry::item(&label.tr(), action)
}

struct Notes {
    store: PathBuf,
    inbox: Inbox<Event>,
    workers: Vec<Worker>,
    accounts: Vec<Account>,
}

impl Notes {
    fn main_window(cx: &Cx<Self>) -> Option<WindowId> {
        cx.windows()
            .find(|window| cx.kind(*window) == Some(Kind::Main))
    }

    fn with_view(cx: &mut Cx<Self>, act: impl FnOnce(&mut NotesView)) {
        if let Some(window) = Self::main_window(cx)
            && let Some((shown, view)) = cx.window(window)
        {
            act(view);
            shown.request_redraw();
        }
    }

    fn open_window(&mut self, cx: &mut Cx<Self>) {
        if let Some(window) = Self::main_window(cx) {
            cx.raise(window, None);
            return;
        }
        let store = match Store::open(&self.store) {
            Ok(store) => store,
            Err(error) => return tracing::error!("cannot open the notes: {error}"),
        };
        let outbox = self.inbox.outbox();
        let accounts = self.accounts.clone();
        let opened = cx.open(Kind::Main, &tr!("Notes"), WINDOW_SIZE, move |appearance| {
            NotesView::new(appearance, store, accounts, outbox)
        });
        if let Err(error) = opened {
            tracing::warn!("cannot open a window: {error}");
        }
    }

    fn reconcile(&mut self, cx: &mut Cx<Self>, fresh: Option<(AccountId, String)>) {
        let found = accounts::open(&accounts::store_path())
            .and_then(|mail| Ok((accounts::list(&mail)?, accounts::notes(&mail)?)));
        let (all, used) = match found {
            Ok(found) => found,
            Err(error) => return tracing::warn!("cannot read the accounts: {error}"),
        };
        let used = all
            .into_iter()
            .filter(|account| used.contains(&account.id))
            .collect::<Vec<_>>();
        if let Err(error) = Store::open(&self.store).and_then(|store| {
            store.keep_accounts(&used.iter().map(|account| account.id).collect::<Vec<_>>())
        }) {
            tracing::warn!("cannot forget unused accounts: {error}");
        }
        let previous = std::mem::take(&mut self.accounts);
        self.workers.retain(|worker| {
            used.iter().any(|account| {
                account.id == worker.account && previous.iter().any(|known| known == account)
            })
        });
        for account in &used {
            let password = fresh
                .as_ref()
                .filter(|(id, _)| *id == account.id)
                .map(|(_, password)| password.clone());
            match self
                .workers
                .iter()
                .find(|worker| worker.account == account.id)
            {
                Some(worker) => {
                    if let Some(password) = password {
                        worker.send(Command::Password(password));
                    }
                }
                None => self.workers.push(Worker::spawn(
                    account.clone(),
                    password,
                    self.store.clone(),
                    Network::default(),
                    self.inbox.outbox(),
                )),
            }
        }
        self.accounts = used.clone();
        Self::with_view(cx, |view| view.set_accounts(used));
        cx.refresh_menus();
    }

    fn serve(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        let Some((_, view)) = cx.window(window) else {
            return;
        };
        for request in view.take_requests() {
            match request {
                Request::Sync(account) => {
                    if let Some(worker) =
                        self.workers.iter().find(|worker| worker.account == account)
                    {
                        worker.send(Command::Sync);
                    }
                }
                Request::Accounts(fresh) => self.reconcile(cx, fresh),
            }
        }
        cx.refresh_menus();
    }

    fn event(&mut self, cx: &mut Cx<Self>, event: Event) {
        match event {
            Event::Changed(_) => Self::with_view(cx, NotesView::reload),
            Event::Online(account) => Self::with_view(cx, |view| view.set_status(account, None)),
            Event::Offline(account, _) => {
                Self::with_view(cx, |view| view.set_status(account, Some(tr!("Offline"))));
            }
            Event::LoginFailed(account, _) => {
                Self::with_view(cx, |view| {
                    view.set_status(account, Some(tr!("Not signed in")));
                    view.login_failed(account);
                });
            }
            Event::NoFolder(account) => {
                let email = self
                    .accounts
                    .iter()
                    .find(|known| known.id == account)
                    .map(|known| known.email.clone())
                    .unwrap_or_default();
                if let Err(error) = accounts::open(&accounts::store_path())
                    .and_then(|mail| accounts::set_notes(&mail, account, false))
                {
                    tracing::warn!("cannot stop using {email} for Notes: {error}");
                }
                self.reconcile(cx, None);
                Self::with_view(cx, |view| {
                    view.tell(
                        &tr!("No Notes Folder"),
                        &tr!(
                            "The server of {email} has no Notes folder and does not let Notes make one, so the account is not used for notes.",
                            email = email.clone()
                        ),
                    );
                });
            }
            Event::AccountsChanged => self.reconcile(cx, None),
            Event::Found(found) => Self::with_view(cx, |view| view.servers_found(*found)),
            Event::Checked(outcome) => Self::with_view(cx, |view| view.setup_checked(*outcome)),
        }
    }

    fn format_menu(view: Option<&NotesView>) -> Entry {
        let editing = view.is_some_and(NotesView::has_note);
        let (attributes, paragraph) = view.map(NotesView::attributes).unwrap_or_default();
        let level = bande_rich_text::heading_level(&attributes).unwrap_or(0);
        let styles = [
            (text!("Title"), 1),
            (text!("Heading"), 2),
            (text!("Subheading"), 3),
            (text!("Body"), 0),
        ]
        .into_iter()
        .map(|(label, at)| {
            item(label, &format!("style:heading:{at}"))
                .radio(level == at)
                .enabled(editing)
        });
        let lists = [
            (text!("Bulleted List"), "bullet", Some(ListMarker::Bullet)),
            (text!("Numbered List"), "number", Some(ListMarker::Number)),
            (text!("No List"), "none", None),
        ]
        .into_iter()
        .map(|(label, key, marker)| {
            item(label, &format!("style:list:{key}"))
                .radio(paragraph.list == marker)
                .enabled(editing)
        });
        let mut entries = styles.collect::<Vec<_>>();
        entries.push(Entry::separator());
        entries.extend([
            item(text!("Bold"), "style:bold")
                .shortcut("Ctrl+B")
                .check(attributes.bold)
                .enabled(editing),
            item(text!("Italic"), "style:italic")
                .shortcut("Ctrl+I")
                .check(attributes.italic)
                .enabled(editing),
            item(text!("Underline"), "style:underline")
                .shortcut("Ctrl+U")
                .check(attributes.underline)
                .enabled(editing),
            item(text!("Strikethrough"), "style:strikethrough")
                .shortcut("Shift+Ctrl+X")
                .check(attributes.strikethrough)
                .enabled(editing),
            Entry::separator(),
        ]);
        entries.extend(lists);
        Entry::submenu(&tr!("Format"), entries)
    }
}

impl Program for Notes {
    type View = NotesView;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let store = store_path()?;
        let inbox = Inbox::new()?;
        if CHANGES.set(inbox.outbox()).is_err() {
            tracing::warn!("the settings already report their changes");
        }
        let mut notes = Self {
            store,
            inbox,
            workers: Vec::new(),
            accounts: Vec::new(),
        };
        notes.reconcile(cx, None);
        notes.open(cx, arguments, Path::new(""));
        Ok(notes)
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, _directory: &Path) {
        if arguments.iter().any(|argument| argument == NEW_NOTE) {
            self.command(cx, None, "new");
        }
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let view = Self::main_window(cx).and_then(|window| cx.view(window));
        let selected = view.is_some_and(NotesView::has_note);
        let current = view.map(NotesView::current_folder);
        let targets = view
            .map(NotesView::folders)
            .unwrap_or_default()
            .into_iter()
            .map(|(id, title)| {
                Entry::item(&title, &format!("move:{id}")).enabled(Some(id) != current && selected)
            })
            .collect();
        let file = Entry::submenu(
            &tr!("File"),
            vec![
                item(text!("New Note"), "new").shortcut("Ctrl+N"),
                Entry::separator(),
                Entry::submenu(&tr!("Move To"), targets).enabled(selected),
                item(text!("Delete Note"), "delete").enabled(selected),
                item(text!("Undo Delete"), "undo-delete")
                    .enabled(view.is_some_and(NotesView::can_undo)),
                Entry::separator(),
                item(text!("Add Account…"), "add-account"),
            ],
        );
        let find = vec![item(text!("Find"), "find").shortcut("Ctrl+F")];
        Menus {
            menus: vec![file, edit_submenu(find), Self::format_menu(view)],
            window: vec![item(text!("Notes"), "window")],
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: Option<WindowId>, command: &str) {
        if Self::main_window(cx).is_none() {
            self.open_window(cx);
        }
        match command {
            "window" => self.open_window(cx),
            "new" => Self::with_view(cx, NotesView::new_note),
            "delete" => Self::with_view(cx, NotesView::delete_note),
            "undo-delete" => Self::with_view(cx, NotesView::undo_delete),
            "find" => Self::with_view(cx, NotesView::focus_search),
            "add-account" => Self::with_view(cx, |view| view.present_setup(Mode::Kind)),
            command => {
                if let Some(style) = command.strip_prefix("style:") {
                    Self::with_view(cx, |view| view.style(style));
                } else if let Some(target) = command
                    .strip_prefix("move:")
                    .and_then(|target| target.parse().ok())
                {
                    Self::with_view(cx, |view| view.move_note(target));
                }
            }
        }
        if let Some(window) = Self::main_window(cx) {
            self.serve(cx, window);
        }
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        self.open_window(cx);
    }

    fn changed(&mut self, cx: &mut Cx<Self>, window: WindowId) {
        self.serve(cx, window);
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, _window: WindowId, mut view: NotesView) {
        view.flush();
    }

    fn should_quit(&mut self, cx: &mut Cx<Self>) -> bool {
        Self::with_view(cx, NotesView::flush);
        true
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.inbox.fd()]
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, _index: usize) {
        for event in self.inbox.drain() {
            self.event(cx, event);
        }
        if let Some(window) = Self::main_window(cx) {
            self.serve(cx, window);
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<Notes>(&INFO)
}
