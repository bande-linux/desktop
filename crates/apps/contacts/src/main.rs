use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use bande_address_book::store::{Account, AccountKind, BookInfo, Library, data_dir};
use bande_address_book::sync::apply;
use bande_address_book::vcard::{Contact, Label, Labelled, Photo, parse_cards};
use bande_app::{Cx, Dropping, FileDrop, Info, Keys, Kind, Menus, Program, View, WindowId};
use bande_app_menu::{Entry, edit_submenu};
use bande_contacts::offer::CONTACT_ARGUMENT;
use bande_contacts::setup::{Done, problem};
use bande_contacts::view::{
    ADD_ACCOUNT, ContactsView, DELETE_CONTACT, EDIT_CONTACT, EXPORT, FIND, IMPORT, MERGE,
    NEW_CONTACT, REDO, REFRESH, Request, UNDO,
};
use bande_contacts::widgets::{SIDEBAR_WIDTH, TOOLBAR_HEIGHT};
use bande_contacts::worker::{Job, Reply, Worker};
use bande_dav::offer::Offer;
use bande_dav::{Credentials, Error, FoundAddressBooks, new_uid};
use bande_inbox::Inbox;
use bande_l10n::{tr, trn};
use bande_toolkit::ui::{Event, Key};
use bande_toolkit::{Appearance, FrameRegions, Rect, Scene, Window, WindowEvent};
use zbus::blocking::Connection;

const APP_ID: &str = "org.bande.Contacts";
const CALENDAR_ID: &str = "org.bande.Calendar";
const WINDOW_SIZE: (u32, u32) = (980, 640);
const SYNC_EVERY: Duration = Duration::from_secs(15 * 60);
const SHOW_MAIN: &str = "show-main";

const ADD_FLAG: &str = "--add";
const MAX_PHOTO_FILE: u64 = 16 << 20;
const PHOTO_PIXELS: u32 = 512;

static INFO: Info = Info {
    id: APP_ID,
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || tr!("Contacts"),
    keys: Keys::Standard,
    settings: None,
};

fn is_vcard(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("vcf") || extension.eq_ignore_ascii_case("vcard")
        })
}

struct Pane(ContactsView, FileDrop);

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let full_screen = window.header_height() == 0;
        window.set_frame_regions(FrameRegions {
            sidebar: SIDEBAR_WIDTH,
            toolbar: match full_screen {
                true => TOOLBAR_HEIGHT,
                false => 0.0,
            },
        });
        self.0.set_full_screen(full_screen);
        let scene = View::scene(self.0.ui(), window);
        window.set_header(self.0.header());
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        if let Dropping::Dropped(paths) = self.1.event(window, &event, is_vcard) {
            self.0.dropped(paths);
            return true;
        }
        let editing = self.0.editing();
        let handled = match window.drive(self.0.ui(), event) {
            Ok(None) => true,
            Ok(Some(WindowEvent::Input(Event::Key { key, modifiers }))) if !editing => {
                let command = match key {
                    Key::Character('z' | 'Z') if modifiers.control && modifiers.shift => Some(REDO),
                    Key::Character('z') if modifiers.control => Some(UNDO),
                    Key::Delete | Key::Backspace if !modifiers.control => Some(DELETE_CONTACT),
                    _ => None,
                };
                if let Some(command) = command {
                    self.0.command(command);
                }
                command.is_some()
            }
            Ok(Some(_)) => false,
            Err(error) => {
                tracing::warn!("{error}");
                false
            }
        };
        self.0.pump() || handled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.0.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.0.ui().set_window_active(active);
    }

    fn deadline(&self) -> Option<Instant> {
        self.0.ui_ref().deadline()
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) -> bool {
        true
    }

    fn animating(&self) -> bool {
        self.0.ui_ref().is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.0.ui_ref().accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.0.ui_ref().text_input_area()
    }
}

#[derive(Debug)]
enum Chosen {
    Cards(
        String,
        Result<(Vec<bande_address_book::vcard::Card>, usize), String>,
    ),
    Photo(Result<Photo, String>),
    Saved(Result<(), String>),
}

fn read_cards(path: &Path) -> Chosen {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let read = bande_content_lines::read_file(path)
        .map(|text| {
            let parsed = parse_cards(&text);
            (parsed.cards, parsed.skipped)
        })
        .map_err(|error| error.to_string());
    Chosen::Cards(name, read)
}

fn read_photo(path: &Path) -> Result<Photo, String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_PHOTO_FILE {
        return Err(tr!("The picture is too large."));
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| error.to_string())?;
    if u64::from(width) * u64::from(height) > 100_000_000 {
        return Err(tr!("The picture is too large."));
    }
    let picture = image::load_from_memory(&bytes)
        .map_err(|error| error.to_string())?
        .thumbnail(PHOTO_PIXELS, PHOTO_PIXELS)
        .into_rgb8();
    let mut jpeg = Vec::new();
    picture
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg),
            image::ImageFormat::Jpeg,
        )
        .map_err(|error| error.to_string())?;
    Ok(Photo {
        mime: "image/jpeg".to_owned(),
        bytes: jpeg,
    })
}

fn in_background(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
    {
        tracing::warn!("cannot start {name}: {error}");
    }
}

fn contact_from(text: &str) -> Contact {
    let (name, email) = match text.rsplit_once('<') {
        Some((name, email)) => (name.trim().trim_matches('"'), email.trim_end_matches('>')),
        None => ("", text),
    };
    let mut contact = Contact::default();
    let mut words = name.split_whitespace().collect::<Vec<_>>();
    if let Some(family) = words.pop() {
        contact.name.family = family.to_owned();
        contact.name.given = words.join(" ");
    }
    if !email.trim().is_empty() {
        contact.emails.push(Labelled {
            label: Label::Other,
            value: email.trim().to_owned(),
        });
    }
    contact
}

struct Contacts {
    main: Option<WindowId>,
    parked: Option<Pane>,
    worker: Worker,
    replies: Inbox<Reply>,
    chosen: Inbox<Chosen>,
    online: Inbox<bool>,
    syncing: bool,
    again: bool,
    next_sync: Instant,
}

impl Contacts {
    fn view<'a>(&'a mut self, cx: &'a mut Cx<Self>) -> Option<&'a mut ContactsView> {
        match self.main {
            Some(main) => cx.window(main).map(|(_, pane)| &mut pane.0),
            None => self.parked.as_mut().map(|pane| &mut pane.0),
        }
    }

    fn show(&mut self, cx: &mut Cx<Self>) {
        if let Some(main) = self.main {
            return cx.raise(main, None);
        }
        let Some(mut pane) = self.parked.take() else {
            return;
        };
        pane.0.calendar_installed = cx.installed(CALENDAR_ID);
        let opened = cx.open(Kind::Main, &tr!("Contacts"), WINDOW_SIZE, |appearance| {
            pane.0.set_appearance(appearance);
            pane
        });
        match opened {
            Ok(window) => self.main = Some(window),
            Err(error) => {
                tracing::warn!("cannot open a window: {error}");
                self.parked = Some(parked());
            }
        }
    }

    fn sync(&mut self, cx: &mut Cx<Self>) {
        self.next_sync = Instant::now() + SYNC_EVERY;
        if self.syncing {
            self.again = true;
            return;
        }
        let Some(view) = self.view(cx) else {
            return;
        };
        let library = &view.library;
        let books: Vec<_> = library
            .books
            .iter()
            .filter(|book| book.info.url.is_some())
            .filter_map(|book| {
                let account = library.account(&book.info.account)?;
                let AccountKind::CardDav { user, .. } = &account.kind else {
                    return None;
                };
                Some((account.id.clone(), user.clone(), book.for_sync()))
            })
            .collect();
        view.status.failed.clear();
        self.worker.send(Job::Adopt);
        if books.is_empty() {
            return;
        }
        self.syncing = true;
        self.worker.send(Job::Sync(books));
    }

    fn add_account(
        &mut self,
        cx: &mut Cx<Self>,
        address: String,
        server: String,
        credentials: Credentials,
        found: FoundAddressBooks,
    ) {
        let id = new_uid();
        let account = Account {
            id: id.clone(),
            name: address,
            kind: AccountKind::CardDav {
                server,
                user: credentials.user.clone(),
                home: found.home.clone(),
            },
        };
        let books = found
            .books
            .iter()
            .map(|book| BookInfo::remote(&id, book))
            .collect();
        if let Some(view) = self.view(cx) {
            view.library.add_account(account, books);
            view.refresh();
        }
        self.worker.send(Job::SetPassword {
            account: id,
            user: credentials.user,
            password: credentials.password,
        });
    }

    fn done(&mut self, cx: &mut Cx<Self>, done: Done) {
        match done {
            Done::Added {
                address,
                server,
                credentials,
                found,
                also_calendar,
            } => {
                if also_calendar {
                    self.worker.send(Job::Offer {
                        offer: Offer {
                            id: new_uid(),
                            address: address.clone(),
                            server: server.clone(),
                            user: credentials.user.clone(),
                        },
                        password: credentials.password.clone(),
                    });
                }
                self.add_account(cx, address, server, credentials, found);
            }
            Done::Updated {
                account,
                server,
                credentials,
                found,
            } => {
                let books = found
                    .books
                    .iter()
                    .map(|book| BookInfo::remote(&account, book))
                    .collect();
                let kind = AccountKind::CardDav {
                    server,
                    user: credentials.user.clone(),
                    home: found.home.clone(),
                };
                if let Some(view) = self.view(cx) {
                    view.library.update_account(&account, kind, books);
                    view.refresh();
                }
                self.worker.send(Job::SetPassword {
                    account,
                    user: credentials.user,
                    password: credentials.password,
                });
            }
            Done::Password {
                account,
                credentials,
            } => self.worker.send(Job::SetPassword {
                account,
                user: credentials.user,
                password: credentials.password,
            }),
        }
        self.sync(cx);
    }

    fn reply(&mut self, cx: &mut Cx<Self>, reply: Reply) {
        match reply {
            Reply::Synced(outcome) => {
                let Some(view) = self.view(cx) else {
                    return;
                };
                let conflicts = apply(&mut view.library, outcome);
                view.refresh();
                if !conflicts.is_empty() {
                    view.alert(
                        &trn!(
                            "A contact changed on another device too",
                            "Contacts changed on another device too",
                            count = conflicts.len()
                        ),
                        &tr!(
                            "{names}: Contacts kept the version on the server and your change as a second contact, so you can merge them.",
                            names = conflicts.join(", ")
                        ),
                    );
                }
            }
            Reply::Failed { account, error } => {
                if let Some(view) = self.view(cx) {
                    if error == Error::Unauthorized {
                        view.ask_password(&account);
                    }
                    view.status.failed.push((account, problem(&error)));
                    view.refresh();
                }
            }
            Reply::Finished => {
                self.syncing = false;
                if std::mem::take(&mut self.again) {
                    self.sync(cx);
                }
            }
            Reply::Found { setup, server } => {
                if let Some(view) = self.view(cx) {
                    view.setup_found(setup, server);
                }
            }
            Reply::Signed { setup, result } => {
                let done = self
                    .view(cx)
                    .and_then(|view| view.setup_signed(setup, result));
                if let Some(done) = done {
                    self.done(cx, done);
                }
            }
            Reply::Adopted {
                offer,
                credentials,
                result,
            } => match result {
                Ok(found) => {
                    self.add_account(cx, offer.address, offer.server, credentials, found);
                    self.sync(cx);
                }
                Err(error) => {
                    if let Some(view) = self.view(cx) {
                        view.alert(
                            &tr!(
                                "{address} could not be used for Contacts",
                                address = offer.address
                            ),
                            &problem(&error),
                        );
                    }
                }
            },
            Reply::OfferFailed(error) => {
                if let Some(view) = self.view(cx) {
                    view.alert(&tr!("The account could not be offered to Calendar"), &error);
                }
            }
        }
    }

    fn serve(&mut self, cx: &mut Cx<Self>) {
        let session = cx.session().cloned();
        let token = self.main.and_then(|main| {
            cx.window(main).and_then(|(window, _)| {
                window
                    .activation_token(APP_ID)
                    .inspect_err(|error| tracing::warn!("cannot get an activation token: {error}"))
                    .ok()
            })
        });
        let Some(view) = self.view(cx) else {
            return;
        };
        let done = view.take_done();
        let requests = view.take_requests();
        bande_contacts::offer::offer_to_search(&view.library);
        if let Some(done) = done {
            self.done(cx, done);
        }
        for request in requests {
            match request {
                Request::Run(job) => self.worker.send(job),
                Request::Sync => self.sync(cx),
                Request::Open(url) => {
                    if let Err(error) = bande_xdg::open(&url, token.as_deref()) {
                        tracing::warn!("cannot open {url}: {error}");
                    }
                }
                Request::ChooseImport => self.choose_import(session.clone()),
                Request::ChooseExport(text) => self.choose_export(session.clone(), text),
                Request::ChoosePhoto => self.choose_photo(session.clone()),
                Request::Read(paths) => self.read(paths),
            }
        }
        if let Some(main) = self.main
            && let Some((window, _)) = cx.window(main)
        {
            window.request_redraw();
        }
        cx.refresh_menus();
    }

    fn read(&self, paths: Vec<PathBuf>) {
        let outbox = self.chosen.outbox();
        in_background("read-contacts", move || {
            for path in paths {
                outbox.send(read_cards(&path));
            }
        });
    }

    fn choose_import(&self, session: Option<Connection>) {
        let Some(session) = session else {
            return tracing::warn!("no session bus to ask for a file");
        };
        let outbox = self.chosen.outbox();
        in_background("choose-contacts", move || {
            let filter = (
                tr!("Contacts"),
                vec![(1, "text/vcard".to_owned()), (1, "text/x-vcard".to_owned())],
            );
            match bande_app::choose_files(&session, &tr!("Import Contacts"), Some(filter)) {
                Ok(paths) => {
                    for path in paths {
                        outbox.send(read_cards(&path));
                    }
                }
                Err(error) => tracing::warn!("cannot choose contacts: {error}"),
            }
        });
    }

    fn choose_export(&self, session: Option<Connection>, text: String) {
        let Some(session) = session else {
            return tracing::warn!("no session bus to ask for a file");
        };
        let outbox = self.chosen.outbox();
        in_background("export-contacts", move || {
            let name = format!("{}.vcf", tr!("Contacts"));
            match bande_app::choose_save_file(&session, &tr!("Export Contacts"), &name) {
                Ok(Some(path)) => {
                    let written = std::fs::write(&path, text).map_err(|error| error.to_string());
                    outbox.send(Chosen::Saved(written));
                }
                Ok(None) => {}
                Err(error) => tracing::warn!("cannot choose where to export: {error}"),
            }
        });
    }

    fn choose_photo(&self, session: Option<Connection>) {
        let Some(session) = session else {
            return tracing::warn!("no session bus to ask for a file");
        };
        let outbox = self.chosen.outbox();
        in_background("choose-photo", move || {
            let filter = (
                tr!("Pictures"),
                vec![(1, "image/jpeg".to_owned()), (1, "image/png".to_owned())],
            );
            match bande_app::choose_file(&session, &tr!("Choose a Photo"), Some(filter)) {
                Ok(Some(path)) => {
                    outbox.send(Chosen::Photo(read_photo(&path)));
                }
                Ok(None) => {}
                Err(error) => tracing::warn!("cannot choose a photo: {error}"),
            }
        });
    }

    fn chosen(&mut self, cx: &mut Cx<Self>, chosen: Chosen) {
        self.show(cx);
        let Some(view) = self.view(cx) else {
            return;
        };
        match chosen {
            Chosen::Cards(name, Ok((cards, skipped))) => view.offer_import(&name, cards, skipped),
            Chosen::Cards(name, Err(error)) => {
                view.alert(&tr!("“{name}” could not be opened", name = name), &error)
            }
            Chosen::Photo(Ok(photo)) => view.set_photo(photo),
            Chosen::Photo(Err(error)) => view.alert(&tr!("The photo could not be used"), &error),
            Chosen::Saved(Ok(())) => {}
            Chosen::Saved(Err(error)) => {
                view.alert(&tr!("The contacts could not be exported"), &error)
            }
        }
    }

    fn arguments(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        self.show(cx);
        let mut paths = Vec::new();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            if argument == ADD_FLAG {
                if let Some(address) = arguments.next()
                    && let Some(view) = self.view(cx)
                {
                    view.new_contact(contact_from(&address));
                }
            } else if let Some(uid) = argument.strip_prefix(CONTACT_ARGUMENT) {
                if let Some(view) = self.view(cx) {
                    view.select_uid(uid);
                }
            } else {
                paths.push(
                    bande_xdg::path_from_file_uri(&argument)
                        .unwrap_or_else(|| directory.join(&argument)),
                );
            }
        }
        if !paths.is_empty() {
            self.read(paths);
        }
        self.serve(cx);
    }
}

fn parked() -> Pane {
    Pane(
        ContactsView::new(Library::load(data_dir()), Appearance::default()),
        FileDrop::default(),
    )
}

fn follow_network(outbox: &bande_inbox::Outbox<bool>) -> Result<(), bande_network::Error> {
    bande_network::follow_online(&Connection::system()?, |online| outbox.send(online))
}

fn follow_online(outbox: bande_inbox::Outbox<bool>) {
    in_background("online", move || {
        if let Err(error) = follow_network(&outbox) {
            tracing::debug!("cannot follow the network: {error}");
        }
    });
}

impl Program for Contacts {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let replies = Inbox::new()?;
        let online = Inbox::new()?;
        follow_online(online.outbox());
        let mut contacts = Self {
            main: None,
            parked: Some(parked()),
            worker: Worker::start(replies.outbox())?,
            replies,
            chosen: Inbox::new()?,
            online,
            syncing: false,
            again: false,
            next_sync: Instant::now() + SYNC_EVERY,
        };
        if !arguments.is_empty() {
            let directory = std::env::current_dir()?;
            contacts.arguments(cx, arguments, &directory);
        }
        contacts.sync(cx);
        Ok(contacts)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let view = match self.main {
            Some(main) => cx.view(main).map(|pane| &pane.0),
            None => self.parked.as_ref().map(|pane| &pane.0),
        };
        let item = |title: String, command: &str| {
            Entry::item(&title, command).enabled(view.is_some_and(|view| view.can(command)))
        };
        let file = vec![
            item(tr!("New Contact"), NEW_CONTACT).shortcut("Ctrl+N"),
            item(tr!("Import…"), IMPORT)
                .shortcut("Ctrl+O")
                .without_window(),
            item(tr!("Export…"), EXPORT).shortcut("Shift+Ctrl+E"),
            Entry::separator(),
            item(tr!("Add Account…"), ADD_ACCOUNT),
        ];
        let edit = edit_submenu(vec![
            item(tr!("Edit Contact"), EDIT_CONTACT).shortcut("Ctrl+E"),
            item(tr!("Delete Contact"), DELETE_CONTACT),
            item(tr!("Merge Duplicates…"), MERGE),
            Entry::separator(),
            item(tr!("Find"), FIND).shortcut("Ctrl+F"),
        ]);
        let view_menu = vec![
            item(tr!("Refresh"), REFRESH)
                .shortcut("Ctrl+R")
                .without_window(),
        ];
        Menus {
            menus: vec![
                Entry::submenu(&tr!("File"), file),
                edit,
                Entry::submenu(&tr!("View"), view_menu),
            ],
            window: vec![Entry::item(&tr!("Contacts"), SHOW_MAIN)],
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: Option<WindowId>, command: &str) {
        match command {
            SHOW_MAIN => self.show(cx),
            REFRESH => self.sync(cx),
            command => {
                if command != IMPORT {
                    self.show(cx);
                }
                if let Some(view) = self.view(cx) {
                    view.command(command);
                }
            }
        }
        self.serve(cx);
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        self.arguments(cx, arguments, directory);
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        self.show(cx);
    }

    fn window_closed(&mut self, _cx: &mut Cx<Self>, window: WindowId, view: Pane) {
        if self.main == Some(window) {
            self.main = None;
            self.parked = Some(view);
        }
    }

    fn changed(&mut self, cx: &mut Cx<Self>, _window: WindowId) {
        self.serve(cx);
    }

    fn installed_changed(&mut self, cx: &mut Cx<Self>) {
        let installed = cx.installed(CALENDAR_ID);
        if let Some(view) = self.view(cx) {
            view.calendar_installed = installed;
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.replies.fd(), self.chosen.fd(), self.online.fd()]
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, index: usize) {
        match index {
            0 => {
                for reply in self.replies.drain() {
                    self.reply(cx, reply);
                }
            }
            1 => {
                for chosen in self.chosen.drain() {
                    self.chosen(cx, chosen);
                }
            }
            _ => {
                let changes = self.online.drain();
                if let Some(&online) = changes.last() {
                    if let Some(view) = self.view(cx) {
                        view.status.offline = !online;
                        view.refresh();
                    }
                    if online {
                        self.sync(cx);
                    }
                }
            }
        }
        self.serve(cx);
    }

    fn deadline(&self) -> Option<Instant> {
        Some(self.next_sync)
    }

    fn tick(&mut self, cx: &mut Cx<Self>, now: Instant) {
        if now >= self.next_sync {
            self.sync(cx);
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<Contacts>(&INFO)
}
