use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use bande_app::{Dropping, FileDrop, local_files};
use bande_l10n::{Text, text, tr, trn};

use crate::compose::{Draft, Original, Outgoing, keep_unsent, unsent_folder};
use crate::message::{Address, Attachment, parse_addresses};
use crate::store::Account;
use crate::ui::symbols::Symbol;
use crate::ui::view::{SHORTCUTS, TOOLBAR_HEIGHT};
use accesskit::{ActionRequest, Node, Role, TreeUpdate};
use bande_toolkit::ui::{
    Alert, BUTTON_HEIGHT, Button, CheckState, Checkbox, ClipboardRequest, Context, Cursor, Event,
    FreeLayout, Label, Menu, MenuItem, PopUpButton, Size, TEXT_FIELD_HEIGHT, TextField, TextView,
    Ui, Widget, WidgetId, access_rect, shortcut_pressed,
};
use bande_toolkit::{
    Appearance, HEADER_LEADING, HEADER_TRAILING, Header, Image, Item, Palette, Point, Radius, Rect,
    Scene, Shape, TextAlign, TextRole, TextStyle, TextSystem, Transfer, Window, WindowEvent,
};

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

pub const COMPOSE_SIZE: (u32, u32) = (720, 620);
const MARGIN: f32 = 16.0;
const LABEL_WIDTH: f32 = 64.0;
const FIELD_ROW: f32 = 36.0;
const FIELD_FONT: f32 = 13.0;
const SYMBOL_PIXELS: u32 = 32;
const BODY_GAP: f32 = 10.0;
const CHIP_HEIGHT: f32 = 26.0;
const CHIP_PADDING: f32 = 10.0;
const CHIP_GAP: f32 = 8.0;
const REMOVE_SIZE: f32 = 22.0;
const MORE_PADDING: f32 = 16.0;
const REMOVE_ACTION: &str = "remove-attachment:";
const AUTOSAVE_DELAY: Duration = Duration::from_secs(3);
const CHECK_WIDTH: f32 = 104.0;
const CHECK_GAP: f32 = 8.0;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Signal {
    Send,
    Attach,
    From(usize),
    Discard,
    Remove(u64),
    More,
    Dismissed,
}

struct Completion {
    field: WidgetId,
    typed: String,
    shown: String,
}

struct Chip {
    key: u64,
    remove: WidgetId,
    frame: Rect,
}

pub struct ComposeView {
    ui: Ui,
    host: WidgetId,
    send: WidgetId,
    attach: WidgetId,
    sign: WidgetId,
    encrypt: WidgetId,
    to: WidgetId,
    cc: WidgetId,
    bcc: WidgetId,
    bcc_shown: bool,
    subject: WidgetId,
    from: Option<WidgetId>,
    body: WidgetId,
    signals: Rc<RefCell<Vec<Signal>>>,
    senders: Vec<Account>,
    sender: usize,
    draft: Draft,
    answering: Option<Original>,
    replaces: Option<Original>,
    initial: Draft,
    palette: Palette,
    size: (f32, f32),
    sent: Option<Outgoing>,
    requests: Vec<ComposeRequest>,
    drop: FileDrop,
    chips: Vec<Chip>,
    next_chip: u64,
    shown_chips: usize,
    more: WidgetId,
    more_frame: Rect,
    removed: Vec<(usize, Attachment)>,
    known: Vec<Address>,
    typed: Vec<(WidgetId, String)>,
    completion: Option<Completion>,
    access: WidgetId,
    token: u64,
    closed: bool,
    discarded: Option<Original>,
    unsent: Option<PathBuf>,
    recovered: bool,
    autosaved: Option<Draft>,
    autosave_at: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComposeRequest {
    ChooseFiles,
    Attach(Vec<PathBuf>),
    ShowMenu(Menu, Point),
    Clipboard(ClipboardRequest),
}

fn joined(addresses: &[Address]) -> String {
    addresses
        .iter()
        .map(Address::full)
        .collect::<Vec<_>>()
        .join(", ")
}

fn completes(candidate: &str, token: &str) -> Option<String> {
    let mut rest = candidate.char_indices();
    for typed in token.chars() {
        let (_, known) = rest.next()?;
        if !known.to_lowercase().eq(typed.to_lowercase()) {
            return None;
        }
    }
    let rest = rest.as_str();
    (!rest.is_empty()).then(|| rest.to_owned())
}

fn rows(from: bool, bcc: bool) -> Vec<Text> {
    let mut rows = vec![text!("To:"), text!("Cc:")];
    if bcc {
        rows.push(text!("Bcc:"));
    }
    rows.push(text!("Subject:"));
    if from {
        rows.push(text!("From:"));
    }
    rows
}

impl ComposeView {
    pub fn new(
        outgoing: Outgoing,
        senders: Vec<Account>,
        bcc_field: bool,
        known: Vec<Address>,
        palette: Palette,
        text: TextSystem,
    ) -> Self {
        let Outgoing {
            draft,
            answering,
            replaces,
            unsent,
        } = outgoing;
        let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
        let recovered = unsent.is_some();
        let unsent = unsent.or_else(|| {
            let started = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            unsent_folder().map(|folder| folder.join(format!("{started}-{token}.eml")))
        });
        let signals = Rc::new(RefCell::new(Vec::new()));
        let mut host = FreeLayout::new();
        let host_id = Widget::id(&host);
        let send_signal = Rc::clone(&signals);
        let click = move || send_signal.borrow_mut().push(Signal::Send);
        let send_button = Button::symbol(
            Image::from_svg(Symbol::Sent.svg(), SYMBOL_PIXELS, SYMBOL_PIXELS),
            &tr!("Send"),
            click,
        );
        let send = host.insert(send_button, Rect::default());
        let attach_signal = Rc::clone(&signals);
        let click = move || attach_signal.borrow_mut().push(Signal::Attach);
        let attach_button = Button::symbol(
            Image::from_svg(Symbol::Attachment.svg(), SYMBOL_PIXELS, SYMBOL_PIXELS),
            &tr!("Attach Files"),
            click,
        );
        let attach = host.insert(attach_button, Rect::default());
        let check = |label: &str, on: bool| {
            let state = match on {
                true => CheckState::On,
                false => CheckState::Off,
            };
            Checkbox::new(label, state, |_| {})
        };
        let sign = host.insert(check(&tr!("Sign"), draft.sign), Rect::default());
        let encrypt = host.insert(check(&tr!("Encrypt"), draft.encrypt), Rect::default());
        let field = |text: &str| TextField::new("", text, |_| {}).borderless(FIELD_FONT);
        let to = host.insert(field(&joined(&draft.to)), Rect::default());
        let cc = host.insert(field(&joined(&draft.cc)), Rect::default());
        let bcc_shown = bcc_field || !draft.bcc.is_empty();
        let bcc = match bcc_shown {
            true => host.insert(field(&joined(&draft.bcc)), Rect::default()),
            false => host.insert(Label::new(""), Rect::default()),
        };
        let subject = host.insert(field(&draft.subject), Rect::default());
        let sender = senders
            .iter()
            .position(|account| account.email.eq_ignore_ascii_case(&draft.from.email))
            .unwrap_or(0);
        let from = (senders.len() > 1).then(|| {
            let choices = senders
                .iter()
                .map(|account| Address::from(account).full())
                .collect::<Vec<_>>();
            let chosen = Rc::clone(&signals);
            host.insert(
                PopUpButton::new(&tr!("From:"), &choices, sender, move |index| {
                    chosen.borrow_mut().push(Signal::From(index));
                }),
                Rect::default(),
            )
        });
        let body = host.insert(
            TextView::new(&tr!("Write your message"), &draft.body, |_| {}).borderless(),
            Rect::default(),
        );
        let more = host.insert(Label::new(""), Rect::default());
        let mut view = Self {
            ui: Ui::new(Box::new(host), Context::new(text, palette)),
            host: host_id,
            send,
            attach,
            sign,
            encrypt,
            to,
            cc,
            bcc,
            bcc_shown,
            subject,
            from,
            body,
            signals,
            senders,
            sender,
            draft,
            answering,
            replaces,
            initial: Draft::default(),
            palette,
            size: (0.0, 0.0),
            sent: None,
            requests: Vec::new(),
            drop: FileDrop::default(),
            chips: Vec::new(),
            next_chip: 0,
            shown_chips: 0,
            more,
            more_frame: Rect::default(),
            removed: Vec::new(),
            known,
            typed: Vec::new(),
            completion: None,
            access: WidgetId::next(),
            token,
            closed: false,
            discarded: None,
            unsent,
            recovered,
            autosaved: None,
            autosave_at: None,
        };
        for index in 0..view.draft.attachments.len() {
            view.insert_chip(index);
        }
        view.typed = [view.to, view.cc, view.bcc]
            .into_iter()
            .map(|field| (field, view.field_text(field)))
            .collect();
        let first = match view.draft.to.is_empty() {
            true => view.to,
            false => view.body,
        };
        view.ui.set_focus(first);
        view.ui.set_sheet_top(TOOLBAR_HEIGHT);
        view.initial = view.compose_draft();
        view
    }

    pub fn title(&self) -> String {
        let subject = self
            .ui
            .widget_ref::<TextField>(self.subject)
            .map(TextField::text)
            .unwrap_or_default();
        match subject.trim() {
            "" => tr!("New Message"),
            subject => subject.to_owned(),
        }
    }

    pub fn set_bcc_field(&mut self, shown: bool) {
        if shown == self.bcc_shown || (!shown && !self.field_text(self.bcc).trim().is_empty()) {
            return;
        }
        let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) else {
            return;
        };
        let replaced = match shown {
            true => host.replace(
                self.bcc,
                TextField::new("", "", |_| {}).borderless(FIELD_FONT),
            ),
            false => host.replace(self.bcc, Label::new("")),
        };
        if let Some(bcc) = replaced {
            self.bcc = bcc;
            self.bcc_shown = shown;
        }
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.ui.set_appearance(appearance);
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
        self.ui.context().palette = palette;
    }

    fn frames(&self) -> Vec<(WidgetId, Rect)> {
        let (width, height) = self.size;
        let button = |slot: f32| {
            Rect::new(
                width - HEADER_TRAILING - slot * BUTTON_HEIGHT,
                ((TOOLBAR_HEIGHT - BUTTON_HEIGHT) / 2.0).round(),
                BUTTON_HEIGHT,
                BUTTON_HEIGHT,
            )
        };
        let check = |slot: f32| {
            let attach = button(2.0);
            Rect::new(
                attach.x - CHECK_GAP - slot * (CHECK_WIDTH + CHECK_GAP),
                attach.y,
                CHECK_WIDTH,
                BUTTON_HEIGHT,
            )
        };
        let mut frames = vec![
            (self.send, button(1.0)),
            (self.attach, button(2.0)),
            (self.encrypt, check(1.0)),
            (self.sign, check(2.0)),
        ];
        let field_x = MARGIN + LABEL_WIDTH;
        let field_width = (width - field_x - MARGIN).max(40.0);
        let mut y = TOOLBAR_HEIGHT;
        let mut fields = vec![self.to, self.cc];
        match self.bcc_shown {
            true => fields.push(self.bcc),
            false => frames.push((self.bcc, Rect::default())),
        }
        fields.push(self.subject);
        fields.extend(self.from);
        for id in fields {
            let height = self
                .ui
                .widget_ref::<TextField>(id)
                .map(|field| field.frame().height)
                .filter(|height| *height > 0.0)
                .unwrap_or(TEXT_FIELD_HEIGHT);
            frames.push((
                id,
                Rect::new(
                    field_x,
                    y + ((FIELD_ROW - height) / 2.0).round(),
                    field_width,
                    height,
                ),
            ));
            y += FIELD_ROW;
        }
        if !self.draft.attachments.is_empty() {
            y += FIELD_ROW;
        }
        for (index, chip) in self.chips.iter().enumerate() {
            let frame = match index < self.shown_chips {
                true => Rect::new(
                    chip.frame.x + chip.frame.width - REMOVE_SIZE - 2.0,
                    chip.frame.y + ((CHIP_HEIGHT - REMOVE_SIZE) / 2.0).round(),
                    REMOVE_SIZE,
                    REMOVE_SIZE,
                ),
                false => Rect::default(),
            };
            frames.push((chip.remove, frame));
        }
        frames.push((self.more, self.more_frame));
        frames.push((
            self.body,
            Rect::new(
                MARGIN - 8.0,
                y + BODY_GAP,
                width - 2.0 * MARGIN + 16.0,
                (height - y - BODY_GAP - MARGIN).max(40.0),
            ),
        ));
        frames
    }

    pub fn header(&self) -> Header {
        Header {
            title_x: HEADER_LEADING,
            items: self
                .frames()
                .into_iter()
                .take(4)
                .map(|(_, frame)| frame)
                .collect(),
            tabs: Vec::new(),
        }
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        self.place_chips();
        let mut placed = Vec::new();
        for _ in 0..3 {
            let frames = self.frames();
            if frames == placed {
                break;
            }
            let host = self.host;
            if let Some(host) = self.ui.widget_as::<FreeLayout>(host) {
                for (id, frame) in &frames {
                    host.set_frame(*id, *frame);
                }
            }
            self.ui.layout(Size::new(self.size.0, self.size.1));
            placed = frames;
        }
        let (controls, sheet) = self.ui.paint_layers(width, height);
        let palette = self.palette;
        let mut items = Vec::new();
        let mut y = TOOLBAR_HEIGHT;
        for label in rows(self.from.is_some(), self.bcc_shown) {
            let paragraph = self.ui.context().text.paragraph(
                &label.tr(),
                TextStyle::role(TextRole::Body, palette.ink3),
                Some(LABEL_WIDTH - 8.0),
                TextAlign::End,
            );
            items.push(Item::Text {
                origin: Point::new(
                    MARGIN - 8.0,
                    y + ((FIELD_ROW - paragraph.height()) / 2.0).round(),
                ),
                paragraph,
            });
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    MARGIN,
                    y + FIELD_ROW - 1.0,
                    self.size.0 - 2.0 * MARGIN,
                    1.0,
                )),
                paint: palette.hairline.into(),
            });
            y += FIELD_ROW;
        }
        self.paint_attachments(&mut items);
        items.extend(controls);
        items.extend(sheet);
        Scene {
            width,
            height,
            background: palette.sheet,
            items,
        }
    }

    fn chip_label(attachment: &Attachment) -> String {
        format!(
            "{}  {}",
            attachment.name,
            bande_fs::format_size(attachment.data.len() as u64)
        )
    }

    fn chip_paragraph(&mut self, text: &str) -> bande_toolkit::Paragraph {
        let ink = self.palette.ink;
        self.ui.context().text.paragraph(
            text,
            TextStyle::role(TextRole::Callout, ink),
            None,
            TextAlign::Start,
        )
    }

    fn place_chips(&mut self) {
        let rows = rows(self.from.is_some(), self.bcc_shown).len() as f32;
        let chip_y = TOOLBAR_HEIGHT + rows * FIELD_ROW + ((FIELD_ROW - CHIP_HEIGHT) / 2.0).round();
        let right = self.size.0 - MARGIN;
        let count = self.chips.len();
        let more_width = |hidden: usize, view: &mut Self| {
            let title = trn!("{count} more", "{count} more", count = hidden);
            view.chip_paragraph(&title).width() + 2.0 * MORE_PADDING
        };
        let mut x = MARGIN + LABEL_WIDTH;
        self.shown_chips = count;
        for index in 0..count {
            let Some(label) = self.draft.attachments.get(index).map(Self::chip_label) else {
                break;
            };
            let width = self.chip_paragraph(&label).width() + CHIP_PADDING + REMOVE_SIZE + 4.0;
            let reserve = match count - index - 1 {
                0 => 0.0,
                hidden => CHIP_GAP + more_width(hidden, self),
            };
            if x + width + reserve > right {
                self.shown_chips = index;
                break;
            }
            self.chips[index].frame = Rect::new(x, chip_y, width, CHIP_HEIGHT);
            x += width + CHIP_GAP;
        }
        let hidden = count - self.shown_chips;
        self.more_frame = match hidden {
            0 => Rect::default(),
            _ => Rect::new(x, chip_y, more_width(hidden, self), CHIP_HEIGHT),
        };
        let title = trn!("{count} more", "{count} more", count = hidden);
        let is_button = self.ui.widget_as::<Button>(self.more).is_some();
        if (hidden > 0) != is_button {
            let signals = Rc::clone(&self.signals);
            let replaced =
                self.ui
                    .widget_as::<FreeLayout>(self.host)
                    .and_then(|host| match hidden {
                        0 => host.replace(self.more, Label::new("")),
                        _ => host.replace(
                            self.more,
                            Button::new(&title, move || signals.borrow_mut().push(Signal::More)),
                        ),
                    });
            if let Some(more) = replaced {
                self.more = more;
            }
        }
        if let Some(more) = self.ui.widget_as::<Button>(self.more) {
            more.set_title(&title);
        }
        for (index, chip) in self.chips.iter().enumerate() {
            if let Some(remove) = self.ui.widget_as::<Button>(chip.remove) {
                remove.set_enabled(index < self.shown_chips);
            }
        }
    }

    fn paint_attachments(&mut self, items: &mut Vec<Item>) {
        let well = self.palette.well;
        for index in 0..self.shown_chips {
            let (Some(chip), Some(attachment)) =
                (self.chips.get(index), self.draft.attachments.get(index))
            else {
                break;
            };
            let frame = chip.frame;
            let label = Self::chip_label(attachment);
            let paragraph = self.chip_paragraph(&label);
            items.push(Item::Fill {
                shape: Shape::rounded(frame, Radius::S),
                paint: well.into(),
            });
            items.push(Item::Text {
                origin: Point::new(
                    frame.x + CHIP_PADDING,
                    frame.y + ((CHIP_HEIGHT - paragraph.height()) / 2.0).round(),
                ),
                paragraph,
            });
        }
    }

    fn insert_chip(&mut self, index: usize) {
        let Some(name) = self
            .draft
            .attachments
            .get(index)
            .map(|attachment| attachment.name.clone())
        else {
            return;
        };
        let key = self.next_chip;
        self.next_chip += 1;
        let signals = Rc::clone(&self.signals);
        let click = move || signals.borrow_mut().push(Signal::Remove(key));
        let label = tr!("Remove “{name}”", name = name);
        let button = Button::symbol(
            Image::from_svg(Symbol::Remove.svg(), SYMBOL_PIXELS, SYMBOL_PIXELS),
            &label,
            click,
        );
        let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) else {
            return;
        };
        let remove = host.insert(button, Rect::default());
        self.chips.insert(
            index.min(self.chips.len()),
            Chip {
                key,
                remove,
                frame: Rect::default(),
            },
        );
    }

    pub fn token(&self) -> u64 {
        self.token
    }

    pub fn add_attachments(&mut self, attachments: Vec<Attachment>) {
        for attachment in attachments {
            self.draft.attachments.push(attachment);
            self.insert_chip(self.draft.attachments.len() - 1);
        }
        self.schedule_autosave();
    }

    pub fn attachments(&self) -> &[Attachment] {
        &self.draft.attachments
    }

    fn field_text(&self, id: WidgetId) -> String {
        self.ui
            .widget_ref::<TextField>(id)
            .map(TextField::text)
            .unwrap_or_default()
    }

    fn checked(&self, id: WidgetId) -> bool {
        self.ui
            .widget_ref::<Checkbox>(id)
            .is_some_and(|check| check.state() == CheckState::On)
    }

    fn compose_draft(&self) -> Draft {
        Draft {
            sign: self.checked(self.sign),
            encrypt: self.checked(self.encrypt),
            from: self
                .senders
                .get(self.sender)
                .map(Address::from)
                .unwrap_or_else(|| self.draft.from.clone()),
            to: parse_addresses(&self.field_text(self.to)),
            cc: parse_addresses(&self.field_text(self.cc)),
            bcc: match self.bcc_shown {
                true => parse_addresses(&self.field_text(self.bcc)),
                false => self.draft.bcc.clone(),
            },
            subject: self.field_text(self.subject),
            body: self
                .ui
                .widget_ref::<TextView>(self.body)
                .map(TextView::text)
                .unwrap_or_default(),
            ..self.draft.clone()
        }
    }

    fn try_send(&mut self) {
        let draft = self.compose_draft();
        let typed = self.field_text(self.to);
        if draft.to.is_empty() && draft.cc.is_empty() {
            let message = match typed.trim().is_empty() {
                true => tr!("Add at least one recipient."),
                false => tr!("“{text}” is not a valid address.", text = typed),
            };
            let signals = Rc::clone(&self.signals);
            self.ui.present_sheet(
                Alert::new(&tr!("This message can’t be sent"), &message, move |_| {
                    signals.borrow_mut().push(Signal::Dismissed);
                })
                .primary(&tr!("OK"), "ok"),
            );
            return;
        }
        self.sent = Some(self.outgoing(draft));
        self.closed = true;
    }

    pub fn show_error(&mut self, error: &str) {
        let signals = Rc::clone(&self.signals);
        self.ui.present_sheet(
            Alert::new(&tr!("The message was not sent"), error, move |_| {
                signals.borrow_mut().push(Signal::Dismissed);
            })
            .primary(&tr!("OK"), "ok"),
        );
    }

    fn change_sender(&mut self, index: usize) {
        let signature = |index: usize| {
            self.senders
                .get(index)
                .map(|account| account.signature.trim_end().to_owned())
                .unwrap_or_default()
        };
        let (old, new) = (signature(self.sender), signature(index));
        let opened = self
            .senders
            .iter()
            .position(|account| account.email.eq_ignore_ascii_case(&self.draft.from.email))
            .map(signature)
            .filter(|opened| !opened.is_empty())
            .map(|opened| format!("\n\n{opened}"))
            .unwrap_or_default();
        let quoted = self
            .draft
            .body
            .strip_prefix(opened.as_str())
            .unwrap_or(&self.draft.body)
            .to_owned();
        self.sender = index;
        let Some(body) = self.ui.widget_as::<TextView>(self.body) else {
            return;
        };
        let text = body.text();
        let swapped = match (old.is_empty(), new.is_empty()) {
            (false, false) => match text.find(&old) {
                Some(at) => format!("{}{new}{}", &text[..at], &text[at + old.len()..]),
                None => return,
            },
            (false, true) => {
                let spaced = format!("\n\n{old}");
                let gone = if text.contains(&spaced) { spaced } else { old };
                match text.find(&gone) {
                    Some(at) => format!("{}{}", &text[..at], &text[at + gone.len()..]),
                    None => return,
                }
            }
            (true, true) => return,
            (true, false) => {
                let at = text.rfind(&quoted).unwrap_or(text.len());
                format!("{}\n\n{new}{}", &text[..at], &text[at..])
            }
        };
        body.set_text(&swapped);
    }

    fn after_ui(&mut self) {
        let signals = std::mem::take(&mut *self.signals.borrow_mut());
        for signal in signals {
            match signal {
                Signal::Send => self.try_send(),
                Signal::Attach => self.requests.push(ComposeRequest::ChooseFiles),
                Signal::From(index) => self.change_sender(index),
                Signal::Discard => self.discard(),
                Signal::Remove(key) => self.remove_attachment(key),
                Signal::More => self.show_hidden_attachments(),
                Signal::Dismissed => {}
            }
        }
        self.complete_address();
        if let Some(request) = self.ui.take_clipboard_request() {
            self.requests.push(ComposeRequest::Clipboard(request));
        }
        if let Some((menu, at)) = self.ui.take_menu_request() {
            self.requests.push(ComposeRequest::ShowMenu(menu, at));
        }
        self.schedule_autosave();
    }

    fn complete_address(&mut self) {
        let Some(field) = self.ui.focus().filter(|focused| {
            [self.to, self.cc].contains(focused) || (self.bcc_shown && *focused == self.bcc)
        }) else {
            return;
        };
        let text = self.field_text(field);
        let completion = self
            .completion
            .take()
            .filter(|completion| completion.field == field);
        if completion
            .as_ref()
            .is_some_and(|completion| completion.shown == text)
        {
            self.completion = completion;
            return;
        }
        let Some(at) = self.typed.iter().position(|(id, _)| *id == field) else {
            self.typed.push((field, text));
            return;
        };
        let before = std::mem::replace(&mut self.typed[at].1, text.clone());
        if let Some(completion) = completion
            && let Some(separator) = text.strip_prefix(completion.typed.as_str())
            && [",", ";"].contains(&separator)
        {
            self.set_field(field, &format!("{}, ", completion.shown), None);
            return;
        }
        let selection = self
            .ui
            .widget_ref::<TextField>(field)
            .map(TextField::selection)
            .unwrap_or_default();
        if text.len() <= before.len()
            || !text.starts_with(before.as_str())
            || selection != (text.len()..text.len())
        {
            return;
        }
        let token = text
            .rsplit([',', ';'])
            .next()
            .unwrap_or_default()
            .trim_start();
        if token.chars().count() < 2 {
            return;
        }
        let Some(rest) = self.known.iter().find_map(|address| {
            [address.full(), address.email.clone()]
                .into_iter()
                .find_map(|candidate| completes(&candidate, token))
        }) else {
            return;
        };
        let shown = format!("{text}{rest}");
        self.set_field(field, &shown, Some(text.len()..shown.len()));
        self.completion = Some(Completion {
            field,
            typed: text,
            shown,
        });
    }

    fn set_field(&mut self, field: WidgetId, text: &str, selected: Option<std::ops::Range<usize>>) {
        let Some(widget) = self.ui.widget_as::<TextField>(field) else {
            return;
        };
        widget.replace_text(text);
        widget.select(selected.unwrap_or(text.len()..text.len()));
        if let Some((_, typed)) = self.typed.iter_mut().find(|(id, _)| *id == field) {
            text.clone_into(typed);
        }
    }

    fn schedule_autosave(&mut self) {
        if self.autosave_at.is_none() && !self.closed {
            self.autosave_at = Some(Instant::now() + AUTOSAVE_DELAY);
        }
    }

    pub fn autosave(&mut self, now: Instant) {
        if self.autosave_at.is_none_or(|at| at > now) || self.closed {
            return;
        }
        self.autosave_at = None;
        if self.unsent.is_none() {
            return;
        }
        let draft = self.compose_draft();
        let unchanged = !self.recovered && self.autosaved.is_none() && draft == self.initial;
        if unchanged || self.autosaved.as_ref() == Some(&draft) {
            return;
        }
        match keep_unsent(&self.outgoing(draft.clone())) {
            Ok(()) => self.autosaved = Some(draft),
            Err(error) => tracing::warn!("cannot keep the unsent message: {error}"),
        }
    }

    pub fn discard_unsent(&self) {
        if !self.closed {
            crate::compose::forget_unsent(&self.outgoing(Draft::default()));
        }
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        if SHORTCUTS
            .iter()
            .any(|(action, shortcut)| *action == "send" && shortcut_pressed(event, shortcut))
            && !self.ui.has_sheet()
        {
            self.try_send();
            return true;
        }
        if shortcut_pressed(event, "Ctrl+Z")
            && !self.ui.has_sheet()
            && self.ui.text_input_area().is_none()
            && self.restore_attachment()
        {
            return true;
        }
        let handled = self.ui.handle(event);
        self.after_ui();
        handled
    }

    pub fn menu_action(&mut self, action: &str) -> bool {
        match action {
            "send" => self.try_send(),
            "discard-draft" => self.confirm_discard(),
            removal if removal.starts_with(REMOVE_ACTION) => {
                if let Ok(key) = removal[REMOVE_ACTION.len()..].parse() {
                    self.remove_attachment(key);
                }
            }
            _ => {
                let handled = self.ui.menu_action(action);
                self.after_ui();
                return handled;
            }
        }
        true
    }

    fn confirm_discard(&mut self) {
        if self.ui.has_sheet() {
            return;
        }
        let message = match self.replaces {
            Some(_) => tr!("The draft is moved to the Trash."),
            None => tr!("The message is not kept."),
        };
        let signals = Rc::clone(&self.signals);
        self.ui.present_sheet(
            Alert::new(&tr!("Discard this message?"), &message, move |choice| {
                if choice == "discard" {
                    signals.borrow_mut().push(Signal::Discard);
                }
            })
            .destructive(&tr!("Discard"), "discard")
            .cancel(&tr!("Cancel"), "cancel"),
        );
    }

    fn discard(&mut self) {
        self.discard_unsent();
        self.discarded = self.replaces.take();
        self.closed = true;
    }

    pub fn take_discarded(&mut self) -> Option<Original> {
        self.discarded.take()
    }

    pub fn dragged(&mut self, window: &mut Window, event: &WindowEvent) -> bool {
        match self.drop.event(window, event, Path::is_file) {
            Dropping::Over => true,
            Dropping::Dropped(paths) => {
                self.requests.push(ComposeRequest::Attach(paths));
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

    pub fn pasted(&mut self, text: &str) -> bool {
        let handled = self.ui.pasted(text);
        self.after_ui();
        handled
    }

    pub fn pasted_transfer(&mut self, transfer: &Transfer) -> bool {
        let paths = local_files(&transfer.uris, Path::is_file);
        if paths.is_empty() {
            return self.pasted(&transfer.text);
        }
        self.requests.push(ComposeRequest::Attach(paths));
        true
    }

    fn remove_attachment(&mut self, key: u64) {
        let Some(index) = self.chips.iter().position(|chip| chip.key == key) else {
            return;
        };
        if index >= self.draft.attachments.len() {
            return;
        }
        let chip = self.chips.remove(index);
        let attachment = self.draft.attachments.remove(index);
        let focused = self.ui.focus() == Some(chip.remove);
        if let Some(host) = self.ui.widget_as::<FreeLayout>(self.host) {
            host.remove(chip.remove);
        }
        self.removed.push((index, attachment));
        if focused {
            let next = self
                .chips
                .get(index)
                .or_else(|| {
                    index
                        .checked_sub(1)
                        .and_then(|before| self.chips.get(before))
                })
                .map_or(self.attach, |chip| chip.remove);
            self.ui.set_focus(next);
        }
        self.schedule_autosave();
    }

    fn restore_attachment(&mut self) -> bool {
        let Some((index, attachment)) = self.removed.pop() else {
            return false;
        };
        let index = index.min(self.draft.attachments.len());
        self.draft.attachments.insert(index, attachment);
        self.insert_chip(index);
        self.schedule_autosave();
        true
    }

    fn show_hidden_attachments(&mut self) {
        let items = self
            .chips
            .iter()
            .zip(&self.draft.attachments)
            .skip(self.shown_chips)
            .map(|(chip, attachment)| {
                MenuItem::new(
                    &tr!("Remove “{name}”", name = attachment.name.as_str()),
                    &format!("{REMOVE_ACTION}{}", chip.key),
                )
            })
            .collect();
        let at = Point::new(
            self.more_frame.x,
            self.more_frame.y + self.more_frame.height,
        );
        self.requests
            .push(ComposeRequest::ShowMenu(Menu::new(items), at));
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        let handled = self.ui.perform(request);
        self.after_ui();
        handled
    }

    pub fn menu_closed(&mut self) -> bool {
        self.ui.menu_closed()
    }

    fn outgoing(&self, draft: Draft) -> Outgoing {
        Outgoing {
            draft,
            answering: self.answering.clone(),
            replaces: self.replaces.clone(),
            unsent: self.unsent.clone(),
        }
    }

    pub fn take_sent(&mut self) -> Option<Outgoing> {
        self.sent.take()
    }

    pub fn unsent(&self) -> Option<Outgoing> {
        let draft = self.compose_draft();
        (self.sent.is_none() && !self.closed && (self.recovered || draft != self.initial))
            .then(|| self.outgoing(draft))
    }

    pub fn take_requests(&mut self) -> Vec<ComposeRequest> {
        std::mem::take(&mut self.requests)
    }

    pub fn finished(&self) -> bool {
        self.closed
    }

    pub fn cursor(&self) -> Cursor {
        self.ui.cursor()
    }

    pub fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    pub fn deadline(&self) -> Option<Instant> {
        [self.ui.deadline(), self.autosave_at]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn accessibility(&self) -> TreeUpdate {
        let mut tree = self.ui.accessibility();
        let mut chips = Vec::new();
        for (index, (chip, attachment)) in self
            .chips
            .iter()
            .zip(&self.draft.attachments)
            .take(self.shown_chips)
            .enumerate()
        {
            let mut node = Node::new(Role::Label);
            node.set_label(attachment.name.as_str());
            node.set_description(bande_fs::format_size(attachment.data.len() as u64));
            node.set_bounds(access_rect(chip.frame));
            let id = self.access.part(index as u64);
            chips.push(id);
            tree.nodes.push((id, node));
        }
        if let Some((_, root)) = tree.nodes.first_mut() {
            for chip in chips {
                root.push_child(chip);
            }
        }
        tree
    }

    pub fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}
