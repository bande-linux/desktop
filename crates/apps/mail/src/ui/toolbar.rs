use std::cell::RefCell;
use std::rc::Rc;

use bande_l10n::{Text, text, tr};

use crate::imap::Role;
use crate::settings::SortBy;
use crate::store::Filter;
use crate::ui::symbols::Symbol;
use crate::ui::view::{MailView, Panes, TOOLBAR_HEIGHT};
use bande_toolkit::ui::{
    BUTTON_HEIGHT, Button, FreeLayout, Label, PopUpButton, SEGMENTED_HEIGHT, SearchField,
    SegmentedControl, TEXT_FIELD_HEIGHT, TextField, WidgetId,
};
use bande_toolkit::{Header, Image, Point, Rect};

const SYMBOL_PIXELS: u32 = 32;
const GROUP_GAP: f32 = 14.0;
const COLUMN_INSET: f32 = 12.0;
const LABELLED_COMPOSE: f32 = 132.0;
const FILTER_WIDTH: f32 = 192.0;
const SORT_WIDTH: f32 = 112.0;
const CONTROL_GAP: f32 = 6.0;
const STATUS_INSET: f32 = 18.0;
pub const LIST_HEADER_HEIGHT: f32 = 64.0;
pub const LIST_TITLE_HEIGHT: f32 = 28.0;
pub const SIDEBAR_FOOT_HEIGHT: f32 = 40.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    GetMail,
    Compose,
    Archive,
    Delete,
    Junk,
    Reply,
    ReplyAll,
    Forward,
    Flag,
    More,
    ToggleSidebar,
    Back,
    MarkRead,
    MoveMenu,
    Search(String),
    Filter(Filter),
    Sort(SortBy),
    Direction,
    NewMailbox(crate::store::AccountId, String),
    RenameMailbox(crate::store::MailboxId, String),
    DeleteMailbox(crate::store::MailboxId),
    TryAgain(crate::store::AccountId),
    ServerSettings,
    OpenMessageFile,
    OpenLink(String),
    TrustIssuer(crate::store::MessageId),
}

pub type Signals = Rc<RefCell<Vec<Signal>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Toolbar {
    pub sidebar: WidgetId,
    pub back: WidgetId,
    pub compose: WidgetId,
    pub labelled: bool,
    pub search: WidgetId,
    pub filter: WidgetId,
    pub sort: WidgetId,
    pub direction: WidgetId,
    pub ascending: bool,
    pub reply: WidgetId,
    pub reply_all: WidgetId,
    pub forward: WidgetId,
    pub archive: WidgetId,
    pub junk: WidgetId,
    pub delete: WidgetId,
    pub flag: WidgetId,
    pub more: WidgetId,
    pub status: WidgetId,
    pub get_mail: WidgetId,
    pub selection: [WidgetId; 5],
    pub reply_field: WidgetId,
}

impl Default for Toolbar {
    fn default() -> Self {
        Self {
            sidebar: WidgetId::next(),
            back: WidgetId::next(),
            compose: WidgetId::next(),
            labelled: true,
            search: WidgetId::next(),
            filter: WidgetId::next(),
            sort: WidgetId::next(),
            direction: WidgetId::next(),
            ascending: false,
            reply: WidgetId::next(),
            reply_all: WidgetId::next(),
            forward: WidgetId::next(),
            archive: WidgetId::next(),
            junk: WidgetId::next(),
            delete: WidgetId::next(),
            flag: WidgetId::next(),
            more: WidgetId::next(),
            status: WidgetId::next(),
            get_mail: WidgetId::next(),
            selection: [WidgetId::next(); 5],
            reply_field: WidgetId::next(),
        }
    }
}

impl Toolbar {
    fn widgets(&self) -> [WidgetId; 23] {
        let [archive, delete, read, move_to, flag] = self.selection;
        [
            self.sidebar,
            self.back,
            self.compose,
            self.search,
            self.filter,
            self.sort,
            self.direction,
            self.reply,
            self.reply_all,
            self.forward,
            self.archive,
            self.junk,
            self.delete,
            self.flag,
            self.more,
            self.status,
            self.get_mail,
            archive,
            delete,
            read,
            move_to,
            flag,
            self.reply_field,
        ]
    }

    fn actions(&self) -> [WidgetId; 8] {
        [
            self.reply,
            self.reply_all,
            self.forward,
            self.archive,
            self.junk,
            self.delete,
            self.flag,
            self.more,
        ]
    }
}

const ACTIONS: [(Symbol, Text, Signal); 8] = [
    (Symbol::Reply, text!("Reply"), Signal::Reply),
    (Symbol::ReplyAll, text!("Reply All"), Signal::ReplyAll),
    (Symbol::Forward, text!("Forward"), Signal::Forward),
    (
        Symbol::Archive,
        text!(context = "action", "Archive"),
        Signal::Archive,
    ),
    (Symbol::Junk, text!("Move to Junk"), Signal::Junk),
    (Symbol::Trash, text!("Delete"), Signal::Delete),
    (Symbol::Flag, text!("Flag"), Signal::Flag),
    (Symbol::More, text!("More"), Signal::More),
];
const GROUP_ENDS: [usize; 2] = [3, 6];
const SELECTION: [(Text, Signal); 5] = [
    (text!(context = "action", "Archive"), Signal::Archive),
    (text!("Delete"), Signal::Delete),
    (text!("Mark as Read"), Signal::MarkRead),
    (text!("Move To…"), Signal::MoveMenu),
    (text!("Flag"), Signal::Flag),
];
const SELECTION_WIDTH: f32 = 128.0;
pub const SELECTION_TOP: f32 = 24.0;
pub const QUICK_REPLY_BAR: f32 = 52.0;
const REPLY_INSET: f32 = 24.0;

pub fn sort_title(sort: SortBy) -> String {
    match sort {
        SortBy::Date => tr!("Date"),
        SortBy::From => tr!("From"),
        SortBy::Subject => tr!("Subject"),
        SortBy::Size => tr!("Size"),
        SortBy::Unread => tr!("Unread"),
    }
}

fn centred(height: f32) -> f32 {
    ((TOOLBAR_HEIGHT - height) / 2.0).round()
}

fn sender(signals: &Signals) -> impl Fn(Signal) + 'static {
    let signals = Rc::clone(signals);
    move |signal| signals.borrow_mut().push(signal)
}

impl MailView {
    fn symbol_button(&self, symbol: Symbol, label: &str, signal: Signal) -> Button {
        let send = sender(&self.signals);
        let click = move || send(signal.clone());
        let image = Image::from_svg(symbol.svg(), SYMBOL_PIXELS, SYMBOL_PIXELS);
        Button::symbol(image, label, click).tooltip(label)
    }

    pub(crate) fn host_mut(&mut self) -> Option<&mut FreeLayout> {
        let host = self.host;
        self.ui.widget_as::<FreeLayout>(host)
    }

    fn compose_button(&self, labelled: bool) -> Button {
        let title = tr!("New Message");
        match labelled {
            true => {
                let send = sender(&self.signals);
                Button::new(&title, move || send(Signal::Compose))
            }
            false => self.symbol_button(Symbol::Compose, &title, Signal::Compose),
        }
    }

    fn direction_button(&self, ascending: bool) -> Button {
        let (symbol, label) = match ascending {
            true => (Symbol::SortAscending, tr!("Ascending")),
            false => (Symbol::SortDescending, tr!("Descending")),
        };
        self.symbol_button(symbol, &label, Signal::Direction)
    }

    pub(crate) fn insert_toolbar(&mut self) {
        let labelled = self.compose_labelled();
        let ascending = self.settings.ascending;
        let compose = self.compose_button(labelled);
        let send = sender(&self.signals);
        let search = SearchField::new(&tr!("Search"), &self.search, move |text| {
            send(Signal::Search(text.to_owned()));
        });
        let send = sender(&self.signals);
        let filters = [tr!("All"), tr!("Unread"), tr!("Flagged")];
        let chosen = Filter::ALL.iter().position(|filter| *filter == self.filter);
        let filter = SegmentedControl::new(&tr!("Show"), &filters, chosen, move |index| {
            if let Some(filter) = Filter::ALL.get(index) {
                send(Signal::Filter(*filter));
            }
        })
        .fill_equally();
        let send = sender(&self.signals);
        let sorts = SortBy::ALL.map(sort_title);
        let chosen = SortBy::ALL
            .iter()
            .position(|sort| *sort == self.settings.sort)
            .unwrap_or_default();
        let sort = PopUpButton::new(&tr!("Sort by"), &sorts, chosen, move |index| {
            if let Some(sort) = SortBy::ALL.get(index) {
                send(Signal::Sort(*sort));
            }
        })
        .tooltip(&tr!("Sort by"));
        let direction = self.direction_button(ascending);
        let actions = ACTIONS
            .iter()
            .map(|(symbol, label, signal)| self.symbol_button(*symbol, &label.tr(), signal.clone()))
            .collect::<Vec<_>>();
        let status = Label::new("").secondary();
        let get_mail = self.symbol_button(Symbol::GetMail, &tr!("Get Mail"), Signal::GetMail);
        let sidebar =
            self.symbol_button(Symbol::Sidebar, &tr!("Show Sidebar"), Signal::ToggleSidebar);
        let back = self.symbol_button(Symbol::Back, &tr!("Back"), Signal::Back);
        let selection = SELECTION.map(|(title, signal)| {
            let send = sender(&self.signals);
            Button::new(&title.tr(), move || send(signal.clone()))
        });
        let reply_field = TextField::new(&tr!("Reply…"), "", |_| {});
        let Some(host) = self.host_mut() else {
            return;
        };
        let sidebar = host.insert_in_header(sidebar, Rect::default());
        let back = host.insert_in_header(back, Rect::default());
        let compose = host.insert_in_header(compose, Rect::default());
        let search = host.insert_in_header(search, Rect::default());
        let filter = host.insert(filter, Rect::default());
        let sort = host.insert(sort, Rect::default());
        let direction = host.insert(direction, Rect::default());
        let ids = actions
            .into_iter()
            .map(|button| host.insert_in_header(button, Rect::default()))
            .collect::<Vec<_>>();
        let status = host.insert(status, Rect::default());
        let get_mail = host.insert(get_mail, Rect::default());
        let selection = selection.map(|button| host.insert(button, Rect::default()));
        let reply_field = host.insert(reply_field, Rect::default());
        if let [reply, reply_all, forward, archive, junk, delete, flag, more] = ids[..] {
            self.toolbar = Toolbar {
                sidebar,
                back,
                compose,
                labelled,
                search,
                filter,
                sort,
                direction,
                ascending,
                reply,
                reply_all,
                forward,
                archive,
                junk,
                delete,
                flag,
                more,
                status,
                get_mail,
                selection,
                reply_field,
            };
        }
    }

    pub(crate) fn remove_toolbar(&mut self) {
        let widgets = self.toolbar.widgets();
        if let Some(host) = self.host_mut() {
            for id in widgets {
                host.remove(id);
            }
        }
    }

    fn compose_labelled(&self) -> bool {
        self.panes() == Panes::Three
            && self.sidebar_size() - self.header_insets.leading.max(COLUMN_INSET) - COLUMN_INSET
                >= LABELLED_COMPOSE
    }

    fn header_frames(&self) -> Vec<(WidgetId, Rect)> {
        let toolbar = self.toolbar;
        let button_y = centred(BUTTON_HEIGHT);
        let button = |x: f32| Rect::new(x, button_y, BUTTON_HEIGHT, BUTTON_HEIGHT);
        let hidden = Rect::default();
        let leading = self.header_insets.leading.max(COLUMN_INSET);
        let reading_alone = self.panes() == Panes::One && self.single_reader;
        let (sidebar, back, compose, after) = match (self.panes(), reading_alone) {
            (Panes::Three, _) if toolbar.labelled => {
                let width = self.sidebar_size() - leading - COLUMN_INSET;
                let compose = Rect::new(leading, button_y, width, BUTTON_HEIGHT);
                (hidden, hidden, compose, leading + width)
            }
            (Panes::Three, _) => {
                let x = self.sidebar_size() - COLUMN_INSET - BUTTON_HEIGHT;
                (hidden, hidden, button(x), x + BUTTON_HEIGHT)
            }
            (_, true) => (hidden, button(leading), hidden, leading + BUTTON_HEIGHT),
            _ => {
                let compose_x = leading + BUTTON_HEIGHT + CONTROL_GAP;
                (
                    button(leading),
                    hidden,
                    button(compose_x),
                    compose_x + BUTTON_HEIGHT,
                )
            }
        };
        let list = self.list_frame();
        let search_x = (list.x + COLUMN_INSET).max(after + COLUMN_INSET);
        let search_end = match self.panes() {
            Panes::One => self.size.0 - self.header_insets.trailing.max(COLUMN_INSET),
            Panes::Two | Panes::Three => list.x + list.width - COLUMN_INSET,
        };
        let search = match list.width > 0.0 {
            true => Rect::new(
                search_x,
                centred(TEXT_FIELD_HEIGHT),
                (search_end - search_x).max(0.0),
                TEXT_FIELD_HEIGHT,
            ),
            false => hidden,
        };
        let mut frames = vec![
            (toolbar.sidebar, sidebar),
            (toolbar.back, back),
            (toolbar.compose, compose),
            (toolbar.search, search),
        ];
        let reader = self.reader_frame();
        let mut x = (reader.x + COLUMN_INSET).max(after + GROUP_GAP);
        for (index, id) in toolbar.actions().into_iter().enumerate() {
            let frame = match reader.width > 0.0 {
                true => button(x),
                false => hidden,
            };
            frames.push((id, frame));
            x += BUTTON_HEIGHT;
            if GROUP_ENDS.contains(&(index + 1)) {
                x += GROUP_GAP;
            }
        }
        frames
    }

    pub(crate) fn toolbar_frames(&self) -> Vec<(WidgetId, Rect)> {
        if self.welcoming() {
            return Vec::new();
        }
        let toolbar = self.toolbar;
        let mut frames = self.header_frames();
        let list = self.list_header_frame();
        let row = Rect::new(
            list.x,
            list.y + LIST_TITLE_HEIGHT,
            list.width,
            list.height - LIST_TITLE_HEIGHT,
        );
        let centred_in_row = |height: f32| row.y + ((row.height - height) / 2.0).round();
        let direction_x = row.x + row.width - COLUMN_INSET - BUTTON_HEIGHT;
        let sort_x = direction_x - CONTROL_GAP - SORT_WIDTH;
        let filter_width = FILTER_WIDTH
            .min(sort_x - CONTROL_GAP - row.x - COLUMN_INSET)
            .max(0.0);
        let shown = |frame: Rect| match list.width > 0.0 {
            true => frame,
            false => Rect::default(),
        };
        frames.extend([
            (
                toolbar.filter,
                shown(Rect::new(
                    row.x + COLUMN_INSET,
                    centred_in_row(SEGMENTED_HEIGHT),
                    filter_width,
                    SEGMENTED_HEIGHT,
                )),
            ),
            (
                toolbar.sort,
                shown(Rect::new(
                    sort_x,
                    centred_in_row(BUTTON_HEIGHT),
                    SORT_WIDTH,
                    BUTTON_HEIGHT,
                )),
            ),
            (
                toolbar.direction,
                shown(Rect::new(
                    direction_x,
                    centred_in_row(BUTTON_HEIGHT),
                    BUTTON_HEIGHT,
                    BUTTON_HEIGHT,
                )),
            ),
        ]);
        frames.extend(self.selection_frames());
        let reader = self.reader_frame();
        let reply = match self.reader.thread.is_some() && reader.width > 0.0 {
            true => Rect::new(
                reader.x + REPLY_INSET,
                reader.y + reader.height + ((QUICK_REPLY_BAR - TEXT_FIELD_HEIGHT) / 2.0).round(),
                (reader.width - 2.0 * REPLY_INSET).max(0.0),
                TEXT_FIELD_HEIGHT,
            ),
            false => Rect::default(),
        };
        frames.push((toolbar.reply_field, reply));
        let foot = self.sidebar_foot_frame();
        if foot.width <= 0.0 {
            frames.extend([
                (toolbar.status, Rect::default()),
                (toolbar.get_mail, Rect::default()),
            ]);
            return frames;
        }
        let get_mail_x = foot.x + foot.width - COLUMN_INSET - BUTTON_HEIGHT;
        let foot_y = foot.y + ((foot.height - BUTTON_HEIGHT) / 2.0).round();
        frames.extend([
            (
                toolbar.status,
                Rect::new(
                    foot.x + STATUS_INSET,
                    foot_y,
                    (get_mail_x - foot.x - STATUS_INSET - CONTROL_GAP).max(0.0),
                    BUTTON_HEIGHT,
                ),
            ),
            (
                toolbar.get_mail,
                Rect::new(get_mail_x, foot_y, BUTTON_HEIGHT, BUTTON_HEIGHT),
            ),
        ]);
        frames
    }

    fn selection_frames(&self) -> Vec<(WidgetId, Rect)> {
        let reader = self.reader_frame();
        let several = self.reader.thread.is_none() && !self.also.is_empty();
        let per_row = ((reader.width - 2.0 * COLUMN_INSET + CONTROL_GAP)
            / (SELECTION_WIDTH + CONTROL_GAP))
            .floor()
            .max(1.0) as usize;
        let top = reader.y + (reader.height / 2.0).round() + SELECTION_TOP;
        self.toolbar
            .selection
            .iter()
            .enumerate()
            .map(|(index, id)| {
                if !several || reader.width <= 0.0 {
                    return (*id, Rect::default());
                }
                let (row, column) = (index / per_row, index % per_row);
                let in_row = (SELECTION.len() - row * per_row).min(per_row) as f32;
                let row_width = in_row * SELECTION_WIDTH + (in_row - 1.0) * CONTROL_GAP;
                let x = reader.x
                    + ((reader.width - row_width) / 2.0).round()
                    + column as f32 * (SELECTION_WIDTH + CONTROL_GAP);
                let y = top + row as f32 * (BUTTON_HEIGHT + CONTROL_GAP);
                (*id, Rect::new(x, y, SELECTION_WIDTH, BUTTON_HEIGHT))
            })
            .collect()
    }

    pub(crate) fn layout_toolbar(&mut self) {
        if self.welcoming() {
            return;
        }
        let labelled = self.compose_labelled();
        if labelled != self.toolbar.labelled {
            let compose = self.compose_button(labelled);
            let old = self.toolbar.compose;
            if let Some(id) = self.host_mut().and_then(|host| host.replace(old, compose)) {
                self.toolbar.compose = id;
                self.toolbar.labelled = labelled;
            }
        }
        if self.settings.ascending != self.toolbar.ascending {
            let direction = self.direction_button(self.settings.ascending);
            let old = self.toolbar.direction;
            if let Some(id) = self
                .host_mut()
                .and_then(|host| host.replace(old, direction))
            {
                self.toolbar.direction = id;
                self.toolbar.ascending = self.settings.ascending;
            }
        }
        let frames = self.toolbar_frames();
        if let Some(host) = self.host_mut() {
            for (id, frame) in frames {
                host.set_frame(id, frame);
            }
        }
    }

    pub(crate) fn toolbar_header(&self) -> Header {
        Header {
            title_x: self.size.0,
            items: match self.welcoming() {
                true => Vec::new(),
                false => self
                    .header_frames()
                    .into_iter()
                    .map(|(_, frame)| frame)
                    .filter(|frame| frame.width > 0.0)
                    .collect(),
            },
            tabs: Vec::new(),
        }
    }

    pub(crate) fn over_control(&self, at: Point) -> bool {
        self.toolbar_frames()
            .iter()
            .any(|(_, frame)| frame.contains(at))
    }

    pub(crate) fn toolbar_order(&self, id: WidgetId) -> Option<usize> {
        self.toolbar.widgets().iter().position(|known| *known == id)
    }

    pub(crate) fn refresh_toolbar(&mut self) {
        if self.welcoming() {
            return;
        }
        let selected = self.selected.is_some();
        let has_account = self.has_accounts();
        let reading = self
            .reader
            .thread
            .as_ref()
            .and_then(crate::ui::reader::Thread::focused)
            .is_some_and(|(_, reading)| reading.downloaded);
        let states = [
            (self.toolbar.get_mail, has_account),
            (self.toolbar.compose, has_account),
            (self.toolbar.archive, self.can_move_to(Role::Archive)),
            (self.toolbar.delete, selected),
            (self.toolbar.junk, self.can_move_to(Role::Junk)),
            (self.toolbar.reply, reading),
            (self.toolbar.reply_all, reading),
            (self.toolbar.forward, reading),
            (self.toolbar.flag, selected),
            (self.toolbar.more, selected),
            (self.toolbar.sidebar, self.panes() != Panes::Three),
            (
                self.toolbar.back,
                self.panes() == Panes::One && self.single_reader,
            ),
        ];
        for (id, enabled) in states {
            if let Some(button) = self.ui.widget_as::<Button>(id) {
                button.set_enabled(enabled);
            }
        }
        let placeholder = self
            .reader
            .focused()
            .map(|(summary, _)| match summary.sender.shown() {
                "" => tr!("Reply…"),
                name => tr!("Reply to {name}…", name = name.to_owned()),
            });
        if let Some(placeholder) = placeholder
            && let Some(field) = self.ui.widget_as::<TextField>(self.toolbar.reply_field)
        {
            field.set_placeholder(&placeholder);
        }
        let status = self.sync_line();
        if let Some(label) = self.ui.widget_as::<Label>(self.toolbar.status) {
            label.set_text(&status);
        }
    }
}
