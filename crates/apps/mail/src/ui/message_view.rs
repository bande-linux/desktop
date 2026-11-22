use std::cell::RefCell;
use std::io::Read;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeUpdate};
use bande_datetime::DateFormats;
use bande_inbox::{Inbox, Outbox};
use bande_l10n::tr;
use bande_toolkit::ui::{
    ClipboardRequest, Context, Cursor, Event, FreeLayout, Key, Modifiers, Pointer, PointerButton,
    Size, Ui, WidgetId, access_rect, opens_menu,
};
use bande_toolkit::{
    Appearance, Header, Item, Palette, Point, Rect, Scene, TextAlign, TextRole, TextStyle,
    TextSystem,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::crypto::{Issuer, Keys};
use crate::imap::Flags;
use crate::message::Content;
use crate::net::Network;
use crate::picture::Picture;
use crate::settings::MailSettings;
use crate::store::{MessageId, Store, Summary};
use crate::ui::reader::{Clock, Entry, Reading, Target, Thread};
use crate::ui::reader_pane::{
    Look, ReaderPane, address_menu, attachment_menu, fetch_remote, link_alert, link_menu,
    paint_link_target,
};
use crate::ui::symbols::Symbols;
use crate::ui::view::security::{
    issuer_not_trusted, key_not_found, keys_of, look_up_key, open_in_background,
    remember_certificates, trust_issuer_alert,
};
use crate::ui::view::{Request, TOOLBAR_HEIGHT};

const MAX_MESSAGE_BYTES: u64 = 64 << 20;
pub const MESSAGE_SIZE: (u32, u32) = (760, 760);
const MESSAGE: MessageId = 0;
const TITLE_INSET: f32 = 16.0;
const TARGETS: u64 = 1 << 20;
const PART_MASK: u64 = (1 << 32) - 1;

enum Arrived {
    Read(Result<(Vec<u8>, Box<Content>), String>),
    Opened(Box<Content>),
    Pictures(Vec<(String, Picture)>),
    LookedUp(Result<(), String>),
}

enum Answer {
    OpenLink(String),
    TrustIssuer,
}

enum Shown {
    Opening,
    Failed(String),
    Open(Vec<u8>),
}

pub struct MessageView {
    path: PathBuf,
    store: Store,
    settings: MailSettings,
    network: Network,
    arrived: Inbox<Arrived>,
    shown: Shown,
    reader: ReaderPane,
    ui: Ui,
    palette: Palette,
    symbols: Symbols,
    zone: TimeZone,
    formats: DateFormats,
    size: (f32, f32),
    hits: Vec<(Rect, Target)>,
    hover: Option<Target>,
    answers: Rc<RefCell<Vec<Answer>>>,
    approved: Vec<String>,
    requests: Vec<Request>,
    access: WidgetId,
}

fn read_message(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut raw = Vec::new();
    file.take(MAX_MESSAGE_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|error| error.to_string())?;
    if raw.len() as u64 > MAX_MESSAGE_BYTES {
        return Err(tr!(
            "The message is larger than {limit}.",
            limit = bande_fs::format_size(MAX_MESSAGE_BYTES)
        ));
    }
    Ok(raw)
}

fn read_in_background(path: PathBuf, keys: Keys, outbox: Outbox<Arrived>) -> Shown {
    let spawned = std::thread::Builder::new()
        .name("open-message".to_owned())
        .spawn(move || {
            let read = read_message(&path).map(|raw| {
                let content = Box::new(Content::open(&raw, Some(&keys)));
                (raw, content)
            });
            outbox.send(Arrived::Read(read));
        });
    match spawned {
        Ok(_) => Shown::Opening,
        Err(error) => {
            tracing::warn!("cannot open a message file: {error}");
            Shown::Failed(error.to_string())
        }
    }
}

fn summary_of(content: &Content, size: usize) -> Summary {
    let envelope = &content.envelope;
    Summary {
        id: MESSAGE,
        mailbox: 0,
        account: 0,
        uid: 0,
        subject: envelope.subject.clone(),
        sender: envelope.sender(),
        date: envelope.date.unwrap_or_default(),
        preview: String::new(),
        flags: Flags {
            seen: true,
            ..Flags::default()
        },
        attachments: !content.attachments.is_empty(),
        downloaded: true,
        thread: None,
        size: i64::try_from(size).unwrap_or(i64::MAX),
    }
}

impl MessageView {
    pub fn new(
        path: PathBuf,
        store_path: &Path,
        settings: MailSettings,
        palette: Palette,
        text: TextSystem,
    ) -> crate::error::Result<Self> {
        let store = Store::open(store_path)?;
        let arrived = Inbox::new()?;
        let shown = read_in_background(path.clone(), keys_of(&store), arrived.outbox());
        let host = FreeLayout::new();
        Ok(Self {
            path,
            store,
            settings,
            network: Network::default(),
            arrived,
            shown,
            reader: ReaderPane::default(),
            ui: Ui::new(Box::new(host), Context::new(text, palette)),
            palette,
            symbols: Symbols::default(),
            zone: TimeZone::system(),
            formats: DateFormats::from_env(),
            size: (0.0, 0.0),
            hits: Vec::new(),
            hover: None,
            answers: Rc::default(),
            approved: Vec::new(),
            requests: Vec::new(),
            access: WidgetId::next(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn title(&self) -> String {
        let subject = self
            .reader
            .focused()
            .map(|(_, reading)| reading.content.envelope.subject.clone())
            .filter(|subject| !subject.is_empty());
        subject.unwrap_or_else(|| {
            self.path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| tr!("Message"))
        })
    }

    pub fn header(&self) -> Header {
        Header {
            title_x: TITLE_INSET,
            items: Vec::new(),
            tabs: Vec::new(),
        }
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.arrived.fd()
    }

    pub fn printable(&self) -> Option<(String, String)> {
        self.reader.printable(&self.zone, &self.formats)
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.ui.set_appearance(appearance);
        self.reader.layout = None;
    }

    pub fn set_window_active(&mut self, active: bool) -> bool {
        self.ui.set_window_active(active)
    }

    pub fn cursor(&self) -> Cursor {
        match &self.hover {
            Some(Target::Card(_)) => Cursor::Text,
            Some(_) => Cursor::Pointer,
            None => self.ui.cursor(),
        }
    }

    pub fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.ui.deadline()
    }

    pub fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }

    pub fn finish(&mut self) {
        self.reader.remove_unedited_attachments();
    }

    fn frame(&self) -> Rect {
        Rect::new(
            0.0,
            TOOLBAR_HEIGHT,
            self.size.0,
            (self.size.1 - TOOLBAR_HEIGHT).max(0.0),
        )
    }

    fn show(&mut self, raw: Vec<u8>, content: Content) {
        remember_certificates(&self.store, &content);
        let summary = summary_of(&content, raw.len());
        let reading = Reading::new(content, true, &self.settings);
        let sources = reading.remote_sources();
        let asked =
            !sources.is_empty() && sources.iter().all(|source| self.approved.contains(source));
        self.reader.thread = Some(Thread {
            entries: vec![Entry {
                summary,
                reading: Some(reading),
                quotes: false,
            }],
            focus: MESSAGE,
            reveal: self.reader.thread.is_none(),
        });
        self.reader.layout = None;
        self.shown = Shown::Open(raw);
        if asked {
            self.load_remote();
        }
    }

    pub fn arrived(&mut self) -> bool {
        for arrived in self.arrived.drain() {
            match arrived {
                Arrived::Read(Ok((raw, content))) => self.show(raw, *content),
                Arrived::Read(Err(error)) => {
                    tracing::warn!("cannot open {}: {error}", self.path.display());
                    self.shown = Shown::Failed(error);
                }
                Arrived::Opened(content) => {
                    if let Shown::Open(raw) = &self.shown {
                        let raw = raw.clone();
                        self.show(raw, *content);
                    }
                }
                Arrived::Pictures(pictures) => self.reader.add_pictures(MESSAGE, pictures),
                Arrived::LookedUp(Ok(())) => self.unseal(),
                Arrived::LookedUp(Err(error)) => self.ui.present_sheet(key_not_found(&error)),
            }
        }
        true
    }

    fn unseal(&mut self) {
        let Shown::Open(raw) = &self.shown else {
            return;
        };
        let outbox = self.arrived.outbox();
        open_in_background(raw.clone(), keys_of(&self.store), move |content| {
            outbox.send(Arrived::Opened(Box::new(content)));
        });
    }

    fn load_remote(&mut self) {
        let Some(sources) = self.reader.start_remote(MESSAGE) else {
            return;
        };
        self.approved.extend(sources.iter().cloned());
        let outbox = self.arrived.outbox();
        fetch_remote(sources, self.network.clone(), move |pictures| {
            outbox.send(Arrived::Pictures(pictures));
        });
    }

    fn content(&self) -> Option<&Content> {
        self.reader.focused().map(|(_, reading)| &reading.content)
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        self.ui.layout(Size::new(self.size.0, self.size.1));
        let (controls, sheet) = self.ui.paint_layers(width, height);
        let palette = self.palette;
        let frame = self.frame();
        let mut items = Vec::new();
        let status = match &self.shown {
            Shown::Opening => Some(tr!("Opening the message…")),
            Shown::Failed(error) => Some(tr!(
                "The message couldn’t be opened: {error}",
                error = error.clone()
            )),
            Shown::Open(_) => None,
        };
        self.hits.clear();
        match status {
            Some(status) => {
                let paragraph = self.ui.context().text.paragraph(
                    &status,
                    TextStyle::role(TextRole::Heading, palette.ink3),
                    Some(frame.width),
                    TextAlign::Center,
                );
                items.push(Item::Text {
                    origin: Point::new(
                        frame.x,
                        frame.y + (frame.height / 2.0 - paragraph.height()).round(),
                    ),
                    paragraph,
                });
            }
            None => {
                let (zone, formats) = (self.zone.clone(), self.formats.clone());
                let look = Look {
                    palette: &palette,
                    symbols: &mut self.symbols,
                    clock: Clock {
                        zone: &zone,
                        formats: &formats,
                        now: Timestamp::now(),
                    },
                    focused: true,
                    sheet: self.ui.has_sheet(),
                };
                items.extend(self.reader.paint(self.ui.context(), look, frame));
                self.hits = self.reader.visible_targets(self.ui.context(), frame);
                if let Some(Target::Link(url, ..)) = &self.hover {
                    let url = url.clone();
                    paint_link_target(self.ui.context(), &palette, &mut items, frame, &url);
                }
            }
        }
        items.extend(controls);
        items.extend(sheet);
        Scene {
            width,
            height,
            background: palette.sheet,
            items,
        }
    }

    fn hit(&self, at: Point) -> Option<Target> {
        self.hits
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(at))
            .map(|(_, target)| target.clone())
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        let handled = self.route(event);
        self.after_ui();
        handled
    }

    fn route(&mut self, event: &Event) -> bool {
        if self.ui.has_sheet() {
            return self.ui.handle(event);
        }
        let frame = self.frame();
        match event {
            Event::Key { key, modifiers } => self.key(*key, *modifiers),
            Event::PointerDown(pointer) => {
                let Some(target) = self.hit(pointer.position) else {
                    return false;
                };
                self.reader.target = None;
                if matches!(target, Target::Card(_)) && pointer.button != PointerButton::Secondary {
                    self.reader
                        .start_selection(self.ui.context(), frame, pointer);
                }
                self.click(target, pointer);
                true
            }
            Event::PointerMove(pointer) if self.reader.selecting => {
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
            Event::PointerMove(pointer) | Event::PointerEnter(pointer) => {
                let hover = self.hit(pointer.position);
                let changed = hover != self.hover;
                self.hover = hover;
                self.ui.handle(event) | changed
            }
            Event::PointerLeave => self.ui.handle(event) | self.hover.take().is_some(),
            Event::Scroll(scroll) => {
                let cx = self.ui.context();
                self.reader.scroll.scroll(cx, scroll)
            }
            _ => self.ui.handle(event),
        }
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        if modifiers.logo || modifiers.alt {
            return false;
        }
        let frame = self.frame();
        if modifiers.control {
            return match key {
                Key::Character('c' | 'C') if !modifiers.shift => self.copy_selection(),
                Key::Character('a' | 'A') if !modifiers.shift => {
                    self.reader.select_entry_text();
                    true
                }
                _ => false,
            };
        }
        match key {
            Key::Tab => self.reader.step_target(self.ui.context(), !modifiers.shift),
            Key::Escape if self.reader.target.is_some() => {
                self.reader.target = None;
                true
            }
            _ if opens_menu(key, modifiers) => self.press(true) || self.show_text_menu(),
            Key::Enter | Key::Space if self.reader.target.is_some() => self.press(false),
            Key::Space => self
                .reader
                .scroll_key(self.ui.context(), frame, Key::PageDown),
            Key::Up | Key::Down | Key::Home | Key::End | Key::PageUp | Key::PageDown => {
                self.reader.scroll_key(self.ui.context(), frame, key)
            }
            _ => false,
        }
    }

    fn press(&mut self, secondary: bool) -> bool {
        let frame = self.frame();
        let Some((target, pointer)) =
            self.reader
                .pressed_target(self.ui.context(), frame, secondary)
        else {
            return false;
        };
        self.click(target, &pointer);
        true
    }

    fn show_text_menu(&mut self) -> bool {
        let frame = self.frame();
        self.requests.push(Request::ShowMenu(
            self.reader.text_menu(),
            Point::new(frame.x + frame.width / 2.0, frame.y + frame.height / 3.0),
        ));
        true
    }

    fn copy_selection(&mut self) -> bool {
        let Some(text) = self.reader.selected_text() else {
            return false;
        };
        self.requests
            .push(Request::Clipboard(ClipboardRequest::Copy(text)));
        true
    }

    fn click(&mut self, target: Target, pointer: &Pointer) {
        let secondary = pointer.button == PointerButton::Secondary;
        let menu = match target {
            Target::Card(_) if secondary => Some(self.reader.text_menu()),
            Target::Quotes(id) => {
                self.reader.toggle_quotes(id);
                None
            }
            Target::Link(url, ..) if secondary => Some(link_menu(&url)),
            Target::Link(url, claimed, _) => {
                self.open_link(url, claimed);
                None
            }
            Target::Address(address) => Some(address_menu(&address)),
            Target::Attachment(_, index) if secondary => Some(attachment_menu(index)),
            Target::Attachment(_, index) => {
                self.open_attachment(index);
                None
            }
            Target::LoadRemote(_) => {
                let sender = self
                    .content()
                    .map(|content| content.envelope.sender().email)
                    .filter(|email| !email.is_empty());
                if let Some(email) = sender
                    && let Err(error) = self.store.trust_remote(&email)
                {
                    tracing::warn!("cannot remember {email}: {error}");
                }
                self.load_remote();
                None
            }
            Target::LookUpKey(_) => {
                let email = self
                    .content()
                    .map(|content| content.envelope.sender().email)
                    .filter(|email| !email.is_empty());
                if let Some(email) = email {
                    let outbox = self.arrived.outbox();
                    look_up_key(email, self.network.clone(), move |found| {
                        outbox.send(Arrived::LookedUp(found));
                    });
                }
                None
            }
            Target::TrustIssuer(_) => {
                self.confirm_trust_issuer();
                None
            }
            Target::Unseal(_) => {
                self.unseal();
                None
            }
            Target::Card(_) | Target::Expand(_) | Target::Respond(..) => None,
        };
        if let Some(menu) = menu {
            self.requests
                .push(Request::ShowMenu(menu, pointer.position));
        }
    }

    fn confirm_trust_issuer(&mut self) {
        let Some(issuer) = self.issuer() else {
            return;
        };
        let answers = Rc::clone(&self.answers);
        self.ui.present_sheet(trust_issuer_alert(&issuer, move || {
            answers.borrow_mut().push(Answer::TrustIssuer);
        }));
    }

    fn issuer(&self) -> Option<Issuer> {
        self.content()?.security.signed.as_ref()?.issuer.clone()
    }

    fn trust_issuer(&mut self) {
        let Some(issuer) = self.issuer() else {
            return;
        };
        match self.store.trust_certificate(&issuer.der) {
            Ok(()) => self.unseal(),
            Err(error) => self
                .ui
                .present_sheet(issuer_not_trusted(&error.to_string())),
        }
    }

    fn open_link(&mut self, url: String, claimed: Option<String>) {
        let Some(claimed) = claimed else {
            return self.requests.push(Request::OpenLink(url));
        };
        let answers = Rc::clone(&self.answers);
        self.ui.present_sheet(link_alert(url, &claimed, move |url| {
            answers.borrow_mut().push(Answer::OpenLink(url));
        }));
    }

    fn open_attachment(&mut self, index: usize) {
        let Some(folder) = bande_xdg::cache_home().map(|cache| {
            let name = self
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            cache.join("bande-mail/opened").join(name)
        }) else {
            return;
        };
        if let Some(path) = self.reader.open_attachment(folder, index) {
            self.requests.push(Request::OpenAttachment(path));
        }
    }

    fn after_ui(&mut self) {
        let answers = std::mem::take(&mut *self.answers.borrow_mut());
        for answer in answers {
            match answer {
                Answer::OpenLink(url) => self.requests.push(Request::OpenLink(url)),
                Answer::TrustIssuer => self.trust_issuer(),
            }
        }
        if let Some(request) = self.ui.take_clipboard_request() {
            self.requests.push(Request::Clipboard(request));
        }
        if let Some((menu, at)) = self.ui.take_menu_request() {
            self.requests.push(Request::ShowMenu(menu, at));
        }
    }

    pub fn menu_action(&mut self, action: &str) -> bool {
        match action {
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
            action if action.starts_with("attachment-") => {
                let parsed = action
                    .split_once(':')
                    .and_then(|(verb, index)| Some((verb, index.parse::<usize>().ok()?)));
                match parsed {
                    Some(("attachment-open", index)) => self.open_attachment(index),
                    Some(("attachment-save", index)) => {
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

    pub fn menu_closed(&mut self) -> bool {
        self.ui.menu_closed()
    }

    fn node(&self, part: u64) -> NodeId {
        self.access.part(part)
    }

    fn part_of(&self, node: NodeId) -> Option<u64> {
        let owner = NodeId::from(self.access).0;
        (node.0 >> 32 == owner)
            .then(|| (node.0 & PART_MASK).checked_sub(1))
            .flatten()
    }

    pub fn accessibility(&self) -> TreeUpdate {
        let mut tree = self.ui.accessibility();
        let mut children = Vec::new();
        for (index, _, mut article, text) in self.reader.articles(&self.zone, &self.formats) {
            let text_id = self.node(2 * index as u64 + 2);
            tree.nodes.push((text_id, text));
            article.set_children(vec![text_id]);
            let article_id = self.node(2 * index as u64 + 1);
            tree.nodes.push((article_id, article));
            children.push(article_id);
        }
        for (index, (rect, target)) in self.hits.iter().enumerate() {
            let Some(mut node) = self.reader.target_node(target) else {
                continue;
            };
            node.set_bounds(access_rect(*rect));
            let id = self.node(TARGETS + index as u64);
            tree.nodes.push((id, node));
            children.push(id);
        }
        let mut document = Node::new(Role::Document);
        match &self.shown {
            Shown::Open(_) => document.set_label(tr!("Message")),
            Shown::Opening => document.set_label(tr!("Opening the message…")),
            Shown::Failed(error) => document.set_label(tr!(
                "The message couldn’t be opened: {error}",
                error = error.clone()
            )),
        }
        document.set_bounds(access_rect(self.frame()));
        document.add_action(Action::Focus);
        document.set_children(children);
        let document_id = self.node(0);
        tree.nodes.push((document_id, document));
        if let Some((_, root)) = tree.nodes.first_mut() {
            root.push_child(document_id);
        }
        if self.ui.focus().is_none() && !self.ui.has_sheet() {
            tree.focus = self
                .reader
                .target
                .as_ref()
                .and_then(|(_, target)| self.hits.iter().position(|(_, hit)| hit == target))
                .map_or(document_id, |index| self.node(TARGETS + index as u64));
        }
        tree
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        let handled = match self.part_of(request.target_node) {
            Some(part) if part >= TARGETS => {
                let hit = usize::try_from(part - TARGETS)
                    .ok()
                    .and_then(|index| self.hits.get(index).cloned());
                match (hit, request.action) {
                    (Some((rect, target)), Action::Click) => {
                        let centre =
                            Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
                        self.click(target, &Pointer::at(centre));
                        true
                    }
                    (Some(_), Action::Focus) => true,
                    _ => false,
                }
            }
            Some(_) => request.action == Action::Focus,
            None => self.ui.perform(request),
        };
        self.after_ui();
        handled
    }
}
