use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use accesskit::{Action, ActionRequest, Node, Role};
use unbound_fs::Entry;
use unbound_preview::Preview;
use unbound_search::{FileResults, Usage};
use unbound_spring::{Animation, Spring, State as SpringState};
use unbound_toolkit::Appearance;
use unbound_toolkit::ui::{
    Context, Event, Key, Menu, Modifiers, ScrollState, Size, TextField, Ui, Widget, WidgetId,
};
use unbound_toolkit::{
    Color, Image, Item, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

use crate::L10N;
use crate::emoji::EMOJI;
use crate::model::{
    Action as LauncherAction, AppInfo, Clip, Heading, Hit, Kind, Mode, Model, PaneInfo, Target,
};
use crate::preview::{PREVIEW_TITLE_SIZE, PreviewPane};
use crate::rows::{
    CELL_HEIGHT, Choice, Form, GLYPH_HEIGHT, ROW_HEIGHT, apply_menu_action, centered, contains,
    fitted, hit_menu,
};
use crate::tones::{TONES_WIDTH, Tones};

pub const CARD_WIDTH: f32 = 720.0;
pub const ICON_SIZE: u32 = 64;
const LIST_WIDTH: f32 = 300.0;
const CARD_RADIUS: f32 = 16.0;
const FIELD_ROW: f32 = 60.0;
const FIELD_SIZE: f32 = 24.0;
const GLYPH_SIZE: f32 = 20.0;
const PADDING: f32 = 8.0;
const GRID_PADDING: f32 = 12.0;
const HEADING_HEIGHT: f32 = 26.0;
const HEADING_SIZE: f32 = 11.0;
const RESULTS_MIN_HEIGHT: f32 = 320.0;
const RESULTS_MAX_HEIGHT: f32 = 460.0;
const BOTTOM_MARGIN: f32 = 24.0;
const NO_RESULTS_HEIGHT: f32 = 96.0;
const FOOTER_HEIGHT: f32 = 40.0;
const FOOTER_SIZE: f32 = 12.0;
const TOP_FRACTION: f32 = 0.22;
const APPEAR_RISE: f32 = 8.0;
const SETTLED: f64 = 0.002;

pub trait Catalog {
    fn icon(&mut self, hit: &Hit) -> Option<Image>;
    fn kind(&mut self, path: &Path, folder: bool) -> String;
}

pub struct Loaded {
    pub entry: Entry,
    pub content: Preview<Image>,
}

pub struct State {
    pub model: Model,
    pub catalog: Box<dyn Catalog>,
    pub preview: Option<Loaded>,
    pub clip_picture: Option<(u64, Preview<Image>)>,
    requested: Option<PathBuf>,
    requested_clip: Option<u64>,
    pub hovered: Option<usize>,
    sent_generation: u64,
    pub viewport: Rect,
    pub selected_frame: Rect,
}

pub type Shared = Rc<RefCell<State>>;

struct Card {
    id: WidgetId,
    state: Shared,
    revision: Option<u64>,
    children: Vec<Box<dyn Widget>>,
    labels: Vec<Item>,
    footer: Vec<Item>,
    empty: bool,
    scroll: ScrollState,
    followed: Option<usize>,
    max_height: f32,
    frame: Rect,
}

fn heading_text(heading: Heading) -> String {
    L10N.tr(match heading {
        Heading::TopHit => "top-hit",
        Heading::Applications => "applications",
        Heading::Settings => "settings",
        Heading::Folders => "folders",
        Heading::Documents => "documents",
        Heading::RecentApps => "recent-apps",
        Heading::RecentDocuments => "recent-documents",
        Heading::AllApps => "all-apps",
        Heading::FrequentlyUsed => "frequently-used",
        Heading::EmojiGroup(group) => match EMOJI.groups.get(group).copied() {
            Some("smileys-emotion") => "emoji-smileys-emotion",
            Some("people-body") => "emoji-people-body",
            Some("animals-nature") => "emoji-animals-nature",
            Some("food-drink") => "emoji-food-drink",
            Some("travel-places") => "emoji-travel-places",
            Some("activities") => "emoji-activities",
            Some("objects") => "emoji-objects",
            Some("symbols") => "emoji-symbols",
            Some("flags") => "emoji-flags",
            _ => "emoji-characters",
        },
    })
}

fn placeholder(mode: Mode) -> String {
    L10N.tr(match mode {
        Mode::Search => "search",
        Mode::Emoji => "search-emoji",
        Mode::Clipboard => "search-clipboard",
    })
}

const FIELD: usize = 0;
const PREVIEW: usize = 1;
const TONES: usize = 2;
const FIRST_ROW: usize = 3;

struct Slot {
    heading: Option<Heading>,
    rect: Rect,
}

impl Card {
    fn sync_rows(&mut self) {
        let mut state = self.state.borrow_mut();
        if self.revision == Some(state.model.revision) {
            return;
        }
        self.revision = Some(state.model.revision);
        self.scroll = ScrollState::default();
        self.followed = None;
        state.hovered = None;
        let form = match (state.model.mode, state.model.columns) {
            (Mode::Emoji, _) => Form::Glyph,
            (_, 0) => Form::Row,
            _ => Form::Cell,
        };
        let hits = state.model.hits.clone();
        let icons: Vec<Option<Image>> = hits.iter().map(|hit| state.catalog.icon(hit)).collect();
        drop(state);
        self.children.truncate(FIRST_ROW);
        for (position, icon) in icons.into_iter().enumerate() {
            self.children.push(Box::new(Choice::new(
                self.state.clone(),
                position,
                form,
                icon,
            )));
        }
    }

    fn shows_results(&self) -> bool {
        let state = self.state.borrow();
        !state.model.hits.is_empty()
            || state.model.mode == Mode::Clipboard
            || (!state.model.query.trim().is_empty() && state.model.files_complete)
    }

    fn is_grid(&self) -> bool {
        self.state.borrow().model.columns > 0
    }

    fn has_footer(&self) -> bool {
        self.state.borrow().model.mode == Mode::Emoji
    }

    fn footer_items(&self, cx: &mut Context, footer: Rect) -> Vec<Item> {
        let state = self.state.borrow();
        let muted = TextStyle::new(FOOTER_SIZE, cx.palette.ink2);
        let hint = cx.text.paragraph(
            &L10N.tr(if state.model.inserts {
                "hint-emoji-insert"
            } else {
                "hint-emoji-copy"
            }),
            muted,
            None,
            TextAlign::Start,
        );
        let hint_x = footer.x + footer.width - 20.0 - TONES_WIDTH - 16.0 - hint.width();
        let middle = |paragraph: &unbound_toolkit::Paragraph| {
            (footer.y + (footer.height - paragraph.height()) / 2.0).round()
        };
        let mut items = Vec::new();
        if let Some(hit) = state.model.selection() {
            let name = fitted(
                cx,
                &hit.title,
                TextStyle::new(FOOTER_SIZE, cx.palette.ink).weight(TextStyle::MEDIUM),
                hint_x - footer.x - 40.0,
            );
            items.push(Item::Text {
                origin: Point::new(footer.x + 20.0, middle(&name)),
                paragraph: name,
            });
        }
        items.push(Item::Text {
            origin: Point::new(hint_x.round(), middle(&hint)),
            paragraph: hint,
        });
        items
    }

    fn slots(&self, width: f32) -> (Vec<Slot>, f32) {
        let state = self.state.borrow();
        let model = &state.model;
        let columns = model.columns;
        let cell_height = if model.mode == Mode::Emoji {
            GLYPH_HEIGHT
        } else {
            CELL_HEIGHT
        };
        let mut slots = Vec::with_capacity(model.hits.len());
        let mut headings = model.headings.iter().peekable();
        let mut y = PADDING;
        let mut section_start = 0;
        let cell_width = if columns > 0 {
            ((width - 2.0 * GRID_PADDING) / columns as f32).floor()
        } else {
            0.0
        };
        for position in 0..model.hits.len() {
            let heading = headings
                .next_if(|(start, _)| *start == position)
                .map(|(_, heading)| *heading);
            if columns == 0 {
                if heading.is_some() {
                    y += HEADING_HEIGHT;
                }
                slots.push(Slot {
                    heading,
                    rect: Rect::new(PADDING, y, LIST_WIDTH - 2.0 * PADDING, ROW_HEIGHT),
                });
                y += ROW_HEIGHT;
                continue;
            }
            if heading.is_some() {
                if position > 0 {
                    y += cell_height;
                }
                y += HEADING_HEIGHT;
                section_start = position;
            }
            let offset = position - section_start;
            if offset > 0 && offset % columns == 0 {
                y += cell_height;
            }
            slots.push(Slot {
                heading,
                rect: Rect::new(
                    GRID_PADDING + (offset % columns) as f32 * cell_width,
                    y,
                    cell_width,
                    cell_height,
                ),
            });
        }
        let content = match slots.last() {
            Some(last) => last.rect.y + last.rect.height + PADDING,
            None => 0.0,
        };
        (slots, content)
    }

    fn list_height(&self, content: f32) -> f32 {
        if self.state.borrow().model.hits.is_empty() {
            return NO_RESULTS_HEIGHT;
        }
        let floor = if self.is_grid() {
            0.0
        } else {
            RESULTS_MIN_HEIGHT
        };
        content.max(floor).min(self.max_height)
    }

    fn follow_selection(&mut self, cx: &Context, slots: &[Slot]) {
        let state = self.state.borrow();
        let selected = state.model.selected;
        let columns = state.model.columns.max(1);
        drop(state);
        if self.followed == Some(selected) {
            return;
        }
        self.followed = Some(selected);
        let Some(slot) = slots.get(selected) else {
            return;
        };
        let section_start = slots[..=selected]
            .iter()
            .rposition(|slot| slot.heading.is_some())
            .filter(|start| selected - start < columns);
        let top = match section_start {
            Some(start) => slots[start].rect.y - HEADING_HEIGHT,
            None => slot.rect.y,
        };
        let top = if top <= PADDING + HEADING_HEIGHT {
            0.0
        } else {
            top
        };
        self.scroll
            .follow(cx, top, slot.rect.y + slot.rect.height + PADDING);
    }

    fn viewport(&self) -> Rect {
        self.state.borrow().viewport
    }
}

impl Widget for Card {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.sync_rows();
        self.max_height = (available.height * (1.0 - TOP_FRACTION) - FIELD_ROW - BOTTOM_MARGIN)
            .clamp(NO_RESULTS_HEIGHT, RESULTS_MAX_HEIGHT);
        self.children[FIELD].measure(cx, Size::new(CARD_WIDTH, FIELD_ROW));
        let footer = if self.has_footer() {
            FOOTER_HEIGHT
        } else {
            0.0
        };
        self.max_height -= footer;
        let results = if self.shows_results() {
            let (_, content) = self.slots(CARD_WIDTH);
            self.list_height(content) + 1.0 + footer
        } else {
            0.0
        };
        Size::new(CARD_WIDTH, FIELD_ROW + results)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let field_x = frame.x + 20.0 + GLYPH_SIZE + 12.0;
        let field_size = self.children[FIELD].measure(
            cx,
            Size::new(frame.x + frame.width - 20.0 - field_x, FIELD_ROW),
        );
        self.children[FIELD].place(
            cx,
            Rect::new(
                field_x,
                frame.y + ((FIELD_ROW - field_size.height) / 2.0).round(),
                frame.x + frame.width - 20.0 - field_x,
                field_size.height,
            ),
        );
        self.labels.clear();
        self.empty = false;
        let top = frame.y + FIELD_ROW + 1.0;
        if !self.shows_results() {
            self.state.borrow_mut().viewport = Rect::default();
            self.children[PREVIEW].place(cx, Rect::default());
            self.children[TONES].place(cx, Rect::default());
            return;
        }
        let footer = if self.has_footer() {
            Rect::new(
                frame.x,
                frame.y + frame.height - FOOTER_HEIGHT,
                frame.width,
                FOOTER_HEIGHT,
            )
        } else {
            Rect::default()
        };
        let tones = if self.has_footer() {
            let size = self.children[TONES].measure(cx, Size::new(frame.width, FOOTER_HEIGHT));
            Rect::new(
                footer.x + footer.width - 20.0 - size.width,
                footer.y + ((footer.height - size.height) / 2.0).round(),
                size.width,
                size.height,
            )
        } else {
            Rect::default()
        };
        self.children[TONES].place(cx, tones);
        let height = frame.y + frame.height - top - footer.height;
        let grid = self.is_grid();
        self.empty = self.state.borrow().model.hits.is_empty();
        if self.empty {
            let empty_history = {
                let model = &self.state.borrow().model;
                model.mode == Mode::Clipboard && model.query.trim().is_empty()
            };
            let message = cx.text.paragraph(
                &L10N.tr(if empty_history {
                    "clipboard-empty"
                } else {
                    "no-results"
                }),
                TextStyle::new(PREVIEW_TITLE_SIZE, cx.palette.ink2),
                None,
                TextAlign::Start,
            );
            let area = Rect::new(frame.x, top, frame.width, height);
            self.labels.push(centered(
                &message,
                area,
                top + (height - message.height()) / 2.0,
            ));
        }
        let viewport = if grid || self.empty {
            Rect::new(frame.x, top, frame.width, height)
        } else {
            Rect::new(frame.x, top, LIST_WIDTH, height)
        };
        let (slots, content) = self.slots(frame.width);
        self.scroll.set_extent(content, height);
        self.follow_selection(cx, &slots);
        let offset = self.scroll.offset(cx).round();
        {
            let mut state = self.state.borrow_mut();
            state.viewport = viewport;
            state.selected_frame = Rect::default();
        }
        let heading_style =
            TextStyle::new(HEADING_SIZE, cx.palette.ink2).weight(TextStyle::SEMIBOLD);
        let heading_x = if grid {
            GRID_PADDING + 8.0
        } else {
            PADDING + 8.0
        };
        for (slot, child) in slots.iter().zip(self.children[FIRST_ROW..].iter_mut()) {
            let rect = Rect::new(
                frame.x + slot.rect.x,
                top + slot.rect.y - offset,
                slot.rect.width,
                slot.rect.height,
            );
            if let Some(heading) = slot.heading {
                let label_top = rect.y - HEADING_HEIGHT;
                if label_top + HEADING_HEIGHT > viewport.y && label_top < viewport.y + height {
                    let label = cx.text.paragraph(
                        &heading_text(heading),
                        heading_style,
                        None,
                        TextAlign::Start,
                    );
                    self.labels.push(Item::Text {
                        origin: Point::new(
                            frame.x + heading_x,
                            (label_top + HEADING_HEIGHT - label.height() - 3.0).round(),
                        ),
                        paragraph: label,
                    });
                }
            }
            child.place(cx, rect);
        }
        let preview = if grid || self.empty {
            Rect::default()
        } else {
            Rect::new(
                frame.x + LIST_WIDTH + 1.0,
                top,
                frame.width - LIST_WIDTH - 1.0,
                height,
            )
        };
        self.children[PREVIEW].place(cx, preview);
        self.footer = if footer.height > 0.0 {
            self.footer_items(cx, footer)
        } else {
            Vec::new()
        };
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, CARD_RADIUS);
        items.push(Item::Shadow {
            shape,
            offset: Point::new(0.0, 16.0),
            blur_radius: 48.0,
            color: Color::hex(0x000000).with_alpha(80),
        });
        items.push(Item::Fill {
            shape,
            paint: cx.palette.overlay.into(),
        });
        items.push(Item::Stroke {
            shape: Shape::rounded(self.frame.inset(0.5), CARD_RADIUS - 0.5),
            width: 1.0,
            paint: cx.palette.hairline.into(),
        });
        let glyph = Rect::new(
            self.frame.x + 20.0,
            self.frame.y + ((FIELD_ROW - GLYPH_SIZE) / 2.0).round(),
            GLYPH_SIZE,
            GLYPH_SIZE,
        );
        let lens = Rect::new(glyph.x + 1.0, glyph.y + 1.0, 14.0, 14.0);
        items.push(Item::Stroke {
            shape: Shape::rounded(lens, 7.0),
            width: 2.0,
            paint: cx.palette.ink2.into(),
        });
        items.push(Item::Line {
            from: Point::new(glyph.x + 13.0, glyph.y + 13.0),
            to: Point::new(glyph.x + 19.0, glyph.y + 19.0),
            width: 2.2,
            paint: cx.palette.ink2.into(),
        });
        for child in &self.children[..FIRST_ROW] {
            child.paint(cx, items);
        }
        if self.frame.height <= FIELD_ROW {
            return;
        }
        let top = self.frame.y + FIELD_ROW;
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(self.frame.x, top, self.frame.width, 1.0)),
            paint: cx.palette.hairline.into(),
        });
        if !self.empty && !self.is_grid() {
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    self.frame.x + LIST_WIDTH,
                    top + 1.0,
                    1.0,
                    self.frame.y + self.frame.height - top - 1.0,
                )),
                paint: cx.palette.hairline.into(),
            });
        }
        let viewport = self.viewport();
        let mut list = self.labels.clone();
        for child in &self.children[FIRST_ROW..] {
            child.paint(cx, &mut list);
        }
        items.push(Item::Clip {
            shape: Shape::rect(viewport),
            items: list,
        });
        self.scroll.paint_scrollbar(cx, viewport, items);
        if !self.footer.is_empty() {
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    self.frame.x,
                    viewport.y + viewport.height,
                    self.frame.width,
                    1.0,
                )),
                paint: cx.palette.hairline.into(),
            });
            items.extend(self.footer.iter().cloned());
        }
    }

    fn access(&self, cx: &Context) -> Node {
        let mut node = Node::new(Role::Dialog);
        node.set_label(L10N.tr("launcher"));
        self.scroll.describe(cx, &mut node);
        node
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::Scroll(scroll) => self.scroll.scroll(cx, scroll),
            _ => self.scroll.pointer(cx, self.viewport(), event),
        }
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        self.scroll.perform(cx, request)
    }

    fn hit_children(&self, point: Point) -> bool {
        !self.scroll.claims(self.viewport(), point)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

struct Backdrop {
    id: WidgetId,
    state: Shared,
    children: Vec<Box<dyn Widget>>,
    frame: Rect,
}

impl Widget for Backdrop {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let card = &mut self.children[0];
        let size = card.measure(cx, Size::new(frame.width, frame.height));
        card.place(
            cx,
            Rect::new(
                frame.x + ((frame.width - size.width) / 2.0).round(),
                frame.y + (frame.height * TOP_FRACTION).round(),
                size.width,
                size.height,
            ),
        );
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::Window)
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        if let Event::PointerDown(pointer) = event
            && pointer.is_primary()
            && !contains(self.children[0].frame(), pointer.position)
        {
            self.state
                .borrow_mut()
                .model
                .actions
                .push(LauncherAction::Close);
            return true;
        }
        false
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

fn opens_menu(key: Key, modifiers: Modifiers) -> bool {
    key == Key::Menu
        || (key == Key::Function(10)
            && modifiers
                == Modifiers {
                    shift: true,
                    ..Modifiers::default()
                })
}

pub struct Launcher {
    state: Shared,
    pub ui: Ui,
    menu_request: Option<(Menu, Point)>,
    field: WidgetId,
    appear: Option<(Instant, Animation)>,
    reduce_motion: bool,
}

impl Launcher {
    pub fn new(
        apps: Vec<AppInfo>,
        panes: Vec<PaneInfo>,
        catalog: Box<dyn Catalog>,
        home: PathBuf,
        usage: Usage,
        appearance: Appearance,
        text: TextSystem,
    ) -> Self {
        let state: Shared = Rc::new(RefCell::new(State {
            model: Model::new(apps, panes, home, usage, 0),
            catalog,
            preview: None,
            clip_picture: None,
            requested: None,
            requested_clip: None,
            hovered: None,
            sent_generation: 0,
            viewport: Rect::default(),
            selected_frame: Rect::default(),
        }));
        let changed = state.clone();
        let field = TextField::new(&L10N.tr("search"), "", move |query| {
            changed.borrow_mut().model.set_query(query);
        })
        .borderless(FIELD_SIZE);
        let field_id = field.id();
        let preview = PreviewPane::new(state.clone());
        let card = Card {
            id: WidgetId::next(),
            state: state.clone(),
            revision: None,
            children: vec![
                Box::new(field),
                Box::new(preview),
                Box::new(Tones::new(state.clone())),
            ],
            labels: Vec::new(),
            footer: Vec::new(),
            empty: false,
            scroll: ScrollState::default(),
            followed: None,
            max_height: RESULTS_MAX_HEIGHT,
            frame: Rect::default(),
        };
        let backdrop = Backdrop {
            id: WidgetId::next(),
            state: state.clone(),
            children: vec![Box::new(card)],
            frame: Rect::default(),
        };
        let mut ui = Ui::new(Box::new(backdrop), Context::new(text, appearance.palette()));
        ui.set_appearance(appearance);
        ui.perform(&ActionRequest {
            action: Action::Focus,
            target_tree: accesskit::TreeId::ROOT,
            target_node: field_id.into(),
            data: None,
        });
        Self {
            state,
            ui,
            menu_request: None,
            field: field_id,
            appear: None,
            reduce_motion: appearance.reduce_motion,
        }
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
        self.reduce_motion = appearance.reduce_motion;
    }

    pub fn query(&self) -> String {
        self.state.borrow().model.query.clone()
    }

    pub fn hits(&self) -> Vec<Hit> {
        self.state.borrow().model.hits.clone()
    }

    pub fn headings(&self) -> Vec<Heading> {
        let state = self.state.borrow();
        state
            .model
            .headings
            .iter()
            .map(|(_, heading)| *heading)
            .collect()
    }

    pub fn selection(&self) -> Option<Hit> {
        self.state.borrow().model.selection().cloned()
    }

    pub fn set_now(&mut self, seconds: u64) {
        self.state.borrow_mut().model.set_now(seconds);
    }

    pub fn file_boosts(&self) -> HashMap<PathBuf, u32> {
        self.state.borrow().model.file_boosts()
    }

    pub fn record_use(&mut self, target: &Target) -> io::Result<()> {
        self.state.borrow_mut().model.record_use(target)
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        if let Event::Key { key, modifiers } = event {
            let mut state = self.state.borrow_mut();
            let selected_frame = state.selected_frame;
            let model = &mut state.model;
            let handled = match key {
                Key::Down | Key::Up if modifiers.logo => {
                    model.select_next_group(*key == Key::Down);
                    true
                }
                Key::Down | Key::Up => {
                    model.select_vertically(*key == Key::Down);
                    true
                }
                Key::Left if model.columns > 0 && *modifiers == Modifiers::default() => {
                    model.select_previous();
                    true
                }
                Key::Right if model.columns > 0 && *modifiers == Modifiers::default() => {
                    model.select_next();
                    true
                }
                _ if opens_menu(*key, *modifiers) => {
                    let Some(menu) = model.selection().map(|hit| hit_menu(hit, model)) else {
                        return false;
                    };
                    let frame = selected_frame;
                    self.menu_request = Some((menu, Point::new(frame.x, frame.y + frame.height)));
                    return true;
                }
                Key::Tab => {
                    model.select_next_group(!modifiers.shift);
                    true
                }
                Key::Enter if modifiers.logo => {
                    model.reveal();
                    true
                }
                Key::Enter => {
                    model.activate();
                    true
                }
                Key::Character('c') if modifiers.logo => model.copy(),
                Key::Delete if model.mode == Mode::Clipboard && model.query.is_empty() => {
                    model.remove_clip();
                    true
                }
                Key::Backspace if model.mode == Mode::Clipboard && modifiers.logo => {
                    model.remove_clip();
                    true
                }
                Key::Escape if model.query.is_empty() => {
                    model.actions.push(LauncherAction::Close);
                    true
                }
                Key::Escape => {
                    drop(state);
                    self.clear();
                    return true;
                }
                _ => false,
            };
            if handled {
                return true;
            }
        }
        self.ui.handle(event)
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        self.ui.perform(request)
    }

    pub fn take_menu_request(&mut self) -> Option<(Menu, Point)> {
        self.ui
            .take_menu_request()
            .or_else(|| self.menu_request.take())
    }

    pub fn menu_action(&mut self, action: &str) -> bool {
        self.ui.menu_action(action) || apply_menu_action(&mut self.state.borrow_mut().model, action)
    }

    pub fn set_recently_used(&mut self, xbel: Option<&str>) {
        self.state.borrow_mut().model.set_recently_used(xbel);
    }

    pub fn appear(&mut self, now: Instant) {
        let spring = if self.reduce_motion {
            Spring::FADE
        } else {
            Spring::QUICK
        };
        let start = SpringState {
            position: 0.0,
            velocity: 0.0,
        };
        self.appear = Some((now, Animation::new(spring, start, 1.0)));
    }

    fn progress(&self, now: Instant) -> f32 {
        self.appear.map_or(1.0, |(start, mut animation)| {
            animation.advance(now.saturating_duration_since(start).as_secs_f64());
            if animation.is_at_rest(SETTLED) {
                1.0
            } else {
                animation.state().position.clamp(0.0, 1.0) as f32
            }
        })
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        self.progress(now) < 1.0 || self.ui.is_animating()
    }

    pub fn clear(&mut self) {
        self.ui.perform(&ActionRequest {
            action: Action::SetValue,
            target_tree: accesskit::TreeId::ROOT,
            target_node: self.field.into(),
            data: Some(accesskit::ActionData::Value("".into())),
        });
        let mut state = self.state.borrow_mut();
        state.model.set_query("");
        state.preview = None;
        state.requested = None;
        state.clip_picture = None;
        state.requested_clip = None;
    }

    pub fn take_actions(&mut self) -> Vec<LauncherAction> {
        std::mem::take(&mut self.state.borrow_mut().model.actions)
    }

    pub fn open(&mut self, mode: Mode, inserts: bool) {
        self.clear();
        self.state.borrow_mut().model.set_mode(mode, inserts);
        if let Some(field) = self.ui.widget_as::<TextField>(self.field) {
            field.set_placeholder(&placeholder(mode));
        }
    }

    pub fn set_clips(&mut self, clips: Vec<Clip>) {
        self.state.borrow_mut().model.set_clips(clips);
    }

    pub fn clip_picture_request(&mut self) -> Option<(u64, String)> {
        let mut state = self.state.borrow_mut();
        let Some(Target::Clip(id)) = state.model.selection().map(|hit| hit.target.clone()) else {
            return None;
        };
        let mime = state
            .model
            .clip(id)
            .filter(|clip| clip.kind() == Kind::CopiedImage)?
            .picture()?
            .to_owned();
        if state.requested_clip == Some(id) {
            return None;
        }
        state.requested_clip = Some(id);
        Some((id, mime))
    }

    pub fn set_clip_picture(&mut self, id: u64, picture: Preview<Image>) {
        let mut state = self.state.borrow_mut();
        if state.requested_clip == Some(id) {
            state.clip_picture = Some((id, picture));
        }
    }

    pub fn mode(&self) -> Mode {
        self.state.borrow().model.mode
    }

    pub fn set_skin_tone(&mut self, tone: Option<usize>) {
        let mut state = self.state.borrow_mut();
        state.model.set_skin_tone(tone);
        state.model.actions.clear();
    }

    pub fn file_query(&mut self) -> Option<(u64, String)> {
        let mut state = self.state.borrow_mut();
        if state.sent_generation == state.model.generation {
            return None;
        }
        state.sent_generation = state.model.generation;
        if state.model.mode != Mode::Search {
            return None;
        }
        Some((state.model.generation, state.model.query.clone()))
    }

    pub fn set_files(&mut self, results: FileResults) {
        self.state.borrow_mut().model.set_files(results);
    }

    pub fn preview_request(&mut self) -> Option<PathBuf> {
        let mut state = self.state.borrow_mut();
        let path = match state.model.selection().map(|hit| &hit.target) {
            Some(Target::Path(path)) => path.clone(),
            _ => return None,
        };
        if state.requested.as_ref() == Some(&path) {
            return None;
        }
        state.requested = Some(path.clone());
        Some(path)
    }

    pub fn set_preview(&mut self, loaded: Loaded) {
        let mut state = self.state.borrow_mut();
        if state.requested.as_ref() == Some(&loaded.entry.path) {
            state.preview = Some(loaded);
        }
    }

    pub fn scene(&mut self, width: u32, height: u32, now: Instant) -> Scene {
        let progress = self.progress(now);
        let mut scene = self.ui.scene(width, height);
        scene.background = Color::TRANSPARENT;
        if progress < 1.0 {
            let items = std::mem::take(&mut scene.items);
            scene.items = vec![Item::Opacity {
                alpha: progress,
                items: if self.reduce_motion {
                    items
                } else {
                    vec![Item::Translate {
                        offset: Point::new(0.0, (APPEAR_RISE * (1.0 - progress)).round()),
                        items,
                    }]
                },
            }];
        } else {
            self.appear = None;
        }
        scene
    }
}
