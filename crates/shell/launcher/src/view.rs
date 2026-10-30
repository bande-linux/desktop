use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use accesskit::{Action, ActionRequest, Node, Role};
use unbound_fs::{Entry, format_date, format_size};
use unbound_preview::Preview;
use unbound_search::{FileResults, Usage};
use unbound_spring::{Animation, Spring, State as SpringState};
use unbound_toolkit::Appearance;
use unbound_toolkit::ui::{
    Context, Event, Key, Menu, MenuItem, Size, TextField, Ui, Widget, WidgetId,
};
use unbound_toolkit::{
    Color, Image, Item, Palette, Paragraph, Point, Rect, Scene, Shape, TextAlign, TextStyle,
    TextSystem,
};

use crate::L10N;
use crate::model::{
    Action as LauncherAction, AppInfo, Heading, Hit, Kind, Model, PaneInfo, Target,
};

pub const CARD_WIDTH: f32 = 720.0;
pub const ICON_SIZE: u32 = 64;
const LIST_WIDTH: f32 = 300.0;
const CARD_RADIUS: f32 = 16.0;
const FIELD_ROW: f32 = 60.0;
const FIELD_SIZE: f32 = 24.0;
const GLYPH_SIZE: f32 = 20.0;
const PADDING: f32 = 8.0;
const ROW_HEIGHT: f32 = 32.0;
const ROW_RADIUS: f32 = 7.0;
const ROW_ICON: f32 = 22.0;
const TITLE_SIZE: f32 = 14.0;
const HEADING_HEIGHT: f32 = 26.0;
const HEADING_SIZE: f32 = 11.0;
const RESULTS_MIN_HEIGHT: f32 = 320.0;
const NO_RESULTS_HEIGHT: f32 = 96.0;
const PREVIEW_PADDING: f32 = 20.0;
const PREVIEW_VISUAL: f32 = 150.0;
const PREVIEW_TITLE_SIZE: f32 = 16.0;
const PREVIEW_TEXT_SIZE: f32 = 12.0;
const CODE_SIZE: f32 = 11.0;
const RESULT_SIZE: f32 = 34.0;
const HINT_SIZE: f32 = 11.0;
const INFO_LABEL_WIDTH: f32 = 72.0;
const INFO_LINE: f32 = 18.0;
const TOP_FRACTION: f32 = 0.22;
const APPEAR_RISE: f32 = 8.0;
const SETTLED: f64 = 0.002;
const SELECTED_LABEL: Color = Color::hex(0xFFFFFF);

pub trait Catalog {
    fn icon(&mut self, hit: &Hit) -> Option<Image>;
    fn kind(&mut self, path: &Path, folder: bool) -> String;
}

pub struct Loaded {
    pub entry: Entry,
    pub content: Preview<Image>,
}

struct State {
    model: Model,
    catalog: Box<dyn Catalog>,
    preview: Option<Loaded>,
    requested: Option<PathBuf>,
    hovered: Option<usize>,
    sent_generation: u64,
}

type Shared = Rc<RefCell<State>>;

fn contains(frame: Rect, point: Point) -> bool {
    point.x >= frame.x
        && point.y >= frame.y
        && point.x < frame.x + frame.width
        && point.y < frame.y + frame.height
}

fn fitted(cx: &mut Context, text: &str, style: TextStyle, width: f32) -> Paragraph {
    let paragraph = cx.text.paragraph(text, style, None, TextAlign::Start);
    if paragraph.width() <= width {
        return paragraph;
    }
    let characters: Vec<char> = text.chars().collect();
    let (mut low, mut high) = (0, characters.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        let candidate: String = characters[..middle].iter().chain(['…'].iter()).collect();
        if cx
            .text
            .paragraph(&candidate, style, None, TextAlign::Start)
            .width()
            <= width
        {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let shortened: String = characters[..low].iter().chain(['…'].iter()).collect();
    cx.text.paragraph(&shortened, style, None, TextAlign::Start)
}

fn centered(paragraph: &Paragraph, frame: Rect, y: f32) -> Item {
    Item::Text {
        origin: Point::new(
            (frame.x + (frame.width - paragraph.width()) / 2.0).round(),
            y.round(),
        ),
        paragraph: paragraph.clone(),
    }
}

fn calculation_badge(cx: &mut Context, rect: Rect, selected: bool, items: &mut Vec<Item>) {
    let fill = if selected {
        SELECTED_LABEL.with_alpha(56)
    } else {
        cx.palette.accent_color
    };
    items.push(Item::Fill {
        shape: Shape::rounded(rect, (rect.width * 0.22).round()),
        paint: fill.into(),
    });
    let sign = cx.text.paragraph(
        "=",
        TextStyle::new((rect.height * 0.7).round(), SELECTED_LABEL).weight(TextStyle::SEMIBOLD),
        None,
        TextAlign::Start,
    );
    items.push(Item::Text {
        origin: Point::new(
            (rect.x + (rect.width - sign.width()) / 2.0).round(),
            (rect.y + (rect.height - sign.height()) / 2.0).round(),
        ),
        paragraph: sign,
    });
}

fn icon_items(
    cx: &mut Context,
    hit: &Hit,
    selected: bool,
    icon: Option<&Image>,
    rect: Rect,
    items: &mut Vec<Item>,
) {
    match (hit.kind, icon) {
        (Kind::Calculation, _) => calculation_badge(cx, rect, selected, items),
        (_, Some(image)) => items.push(Item::Image {
            rect,
            image: image.clone(),
        }),
        (_, None) => items.push(Item::Fill {
            shape: Shape::rounded(rect, (rect.width * 0.22).round()),
            paint: cx.palette.label_muted.with_alpha(90).into(),
        }),
    }
}

struct Row {
    id: WidgetId,
    state: Shared,
    position: usize,
    icon: Option<Image>,
    items: Vec<Item>,
    painted: Option<(bool, bool, Palette, Rect)>,
    frame: Rect,
}

impl Row {
    fn paint_into(&mut self, cx: &mut Context) {
        let state = self.state.borrow();
        let selected = state.model.selected == self.position;
        let hovered = state.hovered == Some(self.position);
        let key = (selected, hovered, cx.palette, self.frame);
        if self.painted == Some(key) {
            return;
        }
        self.painted = Some(key);
        self.items.clear();
        let Some(hit) = state.model.hits.get(self.position) else {
            return;
        };
        if selected {
            self.items.push(Item::Fill {
                shape: Shape::rounded(self.frame, ROW_RADIUS),
                paint: cx.palette.accent_color.into(),
            });
        } else if hovered {
            self.items.push(Item::Fill {
                shape: Shape::rounded(self.frame, ROW_RADIUS),
                paint: cx.palette.label_primary.with_alpha(16).into(),
            });
        }
        let icon = Rect::new(
            self.frame.x + 8.0,
            self.frame.y + ((self.frame.height - ROW_ICON) / 2.0).round(),
            ROW_ICON,
            ROW_ICON,
        );
        icon_items(cx, hit, selected, self.icon.as_ref(), icon, &mut self.items);
        let color = if selected {
            SELECTED_LABEL
        } else {
            cx.palette.label_primary
        };
        let x = icon.x + icon.width + 10.0;
        let title = fitted(
            cx,
            &hit.title,
            TextStyle::new(TITLE_SIZE, color),
            self.frame.x + self.frame.width - x - 8.0,
        );
        self.items.push(Item::Text {
            origin: Point::new(
                x,
                (self.frame.y + (self.frame.height - title.height()) / 2.0).round(),
            ),
            paragraph: title,
        });
    }
}

impl Widget for Row {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, ROW_HEIGHT)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        self.paint_into(cx);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        items.extend(self.items.iter().cloned());
    }

    fn access(&self, _cx: &Context) -> Node {
        let state = self.state.borrow();
        let mut node = Node::new(Role::ListBoxOption);
        if let Some(hit) = state.model.hits.get(self.position) {
            node.set_label(hit.title.clone());
            if let Some(detail) = &hit.detail {
                node.set_description(detail.clone());
            }
        }
        node.set_selected(state.model.selected == self.position);
        node.add_action(Action::Click);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        let mut state = self.state.borrow_mut();
        match event {
            Event::PointerEnter(_) | Event::PointerMove(_) => {
                let changed = state.hovered != Some(self.position);
                state.hovered = Some(self.position);
                changed
            }
            Event::PointerLeave => {
                let changed = state.hovered == Some(self.position);
                if changed {
                    state.hovered = None;
                }
                changed
            }
            Event::PointerDown(_) => {
                state.model.select(self.position);
                true
            }
            Event::PointerUp(pointer)
                if pointer.is_primary() && contains(self.frame, pointer.position) =>
            {
                state.model.select(self.position);
                if pointer.modifiers.logo {
                    state.model.reveal();
                } else {
                    state.model.activate();
                }
                true
            }
            _ => false,
        }
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action != Action::Click {
            return false;
        }
        let mut state = self.state.borrow_mut();
        state.model.select(self.position);
        state.model.activate();
        true
    }

    fn context_menu(&mut self, _cx: &mut Context, _point: Point) -> Option<Menu> {
        let state = self.state.borrow();
        let hit = state.model.hits.get(self.position)?;
        let mut items = Vec::new();
        match &hit.target {
            Target::Value(_) => {
                items.push(MenuItem::new(&L10N.tr("menu-copy"), COPY).shortcut("Super+C"));
            }
            Target::Path(_) => {
                items.push(MenuItem::new(&L10N.tr("menu-open"), OPEN).shortcut("Return"));
                items.push(
                    MenuItem::new(&L10N.tr("menu-show-in-files"), REVEAL).shortcut("Super+Return"),
                );
                items.push(MenuItem::separator());
                items.push(MenuItem::new(&L10N.tr("menu-copy-path"), COPY).shortcut("Super+C"));
            }
            Target::App(_) | Target::Settings(_) => {
                items.push(MenuItem::new(&L10N.tr("menu-open"), OPEN).shortcut("Return"));
            }
        }
        Some(Menu::new(items))
    }

    fn menu_action(&mut self, _cx: &mut Context, action: &str) -> bool {
        let mut state = self.state.borrow_mut();
        state.model.select(self.position);
        match action {
            OPEN => state.model.activate(),
            REVEAL => state.model.reveal(),
            COPY if !state.model.copy() => return false,
            COPY => {}
            _ => return false,
        }
        true
    }
}

const OPEN: &str = "open";
const REVEAL: &str = "reveal";
const COPY: &str = "copy";

#[derive(Clone, Copy, PartialEq)]
struct PreviewKey {
    revision: u64,
    selected: usize,
    copied: bool,
    loaded: bool,
    palette: Palette,
    frame: Rect,
}

struct PreviewPane {
    id: WidgetId,
    state: Shared,
    items: Vec<Item>,
    painted: Option<PreviewKey>,
    frame: Rect,
}

impl PreviewPane {
    fn info(
        cx: &mut Context,
        frame: Rect,
        y: &mut f32,
        lines: &[(String, String)],
        items: &mut Vec<Item>,
    ) {
        let label_style = TextStyle::new(PREVIEW_TEXT_SIZE, cx.palette.label_muted);
        let value_style = TextStyle::new(PREVIEW_TEXT_SIZE, cx.palette.label_primary);
        let value_x = frame.x + PREVIEW_PADDING + INFO_LABEL_WIDTH;
        let value_width = frame.x + frame.width - PREVIEW_PADDING - value_x;
        for (label, value) in lines {
            let label = fitted(cx, label, label_style, INFO_LABEL_WIDTH - 8.0);
            items.push(Item::Text {
                origin: Point::new(value_x - 8.0 - label.width(), *y),
                paragraph: label,
            });
            let value = fitted(cx, value, value_style, value_width);
            items.push(Item::Text {
                origin: Point::new(value_x, *y),
                paragraph: value,
            });
            *y += INFO_LINE;
        }
    }

    fn visual(
        cx: &mut Context,
        hit: &Hit,
        icon: Option<&Image>,
        loaded: Option<&Loaded>,
        area: Rect,
        items: &mut Vec<Item>,
    ) {
        match loaded.map(|loaded| &loaded.content) {
            Some(Preview::Image(image)) => {
                let scale = (area.width / image.width() as f32)
                    .min(area.height / image.height() as f32)
                    .min(1.0);
                let (width, height) = (
                    (image.width() as f32 * scale).round(),
                    (image.height() as f32 * scale).round(),
                );
                let rect = Rect::new(
                    (area.x + (area.width - width) / 2.0).round(),
                    (area.y + (area.height - height) / 2.0).round(),
                    width,
                    height,
                );
                items.push(Item::Clip {
                    shape: Shape::rounded(rect, 6.0),
                    items: vec![Item::Image {
                        rect,
                        image: image.clone(),
                    }],
                });
            }
            Some(Preview::Text(text)) => {
                let shape = Shape::rounded(area, 8.0);
                let lines = (area.height / CODE_SIZE).ceil() as usize;
                let shown = text.lines().take(lines).collect::<Vec<_>>().join("\n");
                let paragraph = cx.text.paragraph(
                    &shown,
                    TextStyle::new(CODE_SIZE, cx.palette.label_primary).monospace(),
                    None,
                    TextAlign::Start,
                );
                items.push(Item::Fill {
                    shape,
                    paint: cx.palette.surface_background.into(),
                });
                items.push(Item::Clip {
                    shape,
                    items: vec![Item::Text {
                        origin: Point::new(area.x + 10.0, area.y + 8.0),
                        paragraph,
                    }],
                });
            }
            _ => {
                let size = ICON_SIZE as f32;
                let rect = Rect::new(
                    (area.x + (area.width - size) / 2.0).round(),
                    (area.y + (area.height - size) / 2.0).round(),
                    size,
                    size,
                );
                icon_items(cx, hit, false, icon, rect, items);
            }
        }
    }

    fn compose(&mut self, cx: &mut Context, key: PreviewKey) {
        self.items.clear();
        let frame = self.frame;
        let mut state = self.state.borrow_mut();
        let state = &mut *state;
        let Some(hit) = state.model.selection().cloned() else {
            return;
        };
        let inner = frame.width - 2.0 * PREVIEW_PADDING;
        let mut y = frame.y + PREVIEW_PADDING;
        let primary = cx.palette.label_primary;
        let muted = cx.palette.label_muted;
        let loaded = state.preview.as_ref().filter(
            |loaded| matches!(&hit.target, Target::Path(path) if *path == loaded.entry.path),
        );
        if let (Kind::Calculation, Some(expression)) = (hit.kind, &hit.detail) {
            let expression = fitted(cx, expression, TextStyle::new(15.0, muted), inner);
            let result = fitted(
                cx,
                &format!("= {}", hit.title),
                TextStyle::new(RESULT_SIZE, primary).weight(TextStyle::SEMIBOLD),
                inner,
            );
            let block = expression.height() + 6.0 + result.height();
            y = frame.y + ((frame.height - block) / 2.0 - PREVIEW_PADDING).max(PREVIEW_PADDING);
            self.items.push(centered(&expression, frame, y));
            y += expression.height() + 6.0;
            self.items.push(centered(&result, frame, y));
        } else {
            let icon = state.catalog.icon(&hit);
            let area = Rect::new(frame.x + PREVIEW_PADDING, y, inner, PREVIEW_VISUAL);
            Self::visual(cx, &hit, icon.as_ref(), loaded, area, &mut self.items);
            y += PREVIEW_VISUAL + 14.0;
            let title = fitted(
                cx,
                &hit.title,
                TextStyle::new(PREVIEW_TITLE_SIZE, primary).weight(TextStyle::SEMIBOLD),
                inner,
            );
            self.items.push(centered(&title, frame, y));
            y += title.height() + 2.0;
            let subtitle = match &hit.target {
                Target::Path(path) => Some(state.catalog.kind(path, hit.kind == Kind::Folder)),
                Target::Settings(_) => hit.detail.clone().or_else(|| Some(L10N.tr("settings"))),
                _ => hit.detail.clone(),
            };
            if let Some(subtitle) = subtitle.filter(|subtitle| *subtitle != hit.title) {
                let subtitle = fitted(
                    cx,
                    &subtitle,
                    TextStyle::new(PREVIEW_TEXT_SIZE, muted),
                    inner,
                );
                self.items.push(centered(&subtitle, frame, y));
                y += subtitle.height();
            }
            y += 14.0;
            if let Target::Path(_) = &hit.target {
                let mut lines = Vec::new();
                if let Some(loaded) = loaded {
                    match loaded.content {
                        Preview::Folder { items } => lines.push((
                            L10N.tr("info-items"),
                            L10N.tr_args("item-count", [("count", items.into())]),
                        )),
                        _ => lines.push((L10N.tr("info-size"), format_size(loaded.entry.size))),
                    }
                    if let Some(modified) = loaded.entry.modified {
                        lines.push((L10N.tr("info-modified"), format_date(modified)));
                    }
                }
                if let Some(parent) = &hit.detail {
                    lines.push((L10N.tr("info-where"), parent.clone()));
                }
                Self::info(cx, frame, &mut y, &lines, &mut self.items);
            }
        }
        let hint = if key.copied {
            L10N.tr("hint-copied")
        } else {
            match hit.kind {
                Kind::Calculation => L10N.tr("hint-calculation"),
                Kind::Application => L10N.tr("hint-application"),
                Kind::Setting => L10N.tr("hint-setting"),
                Kind::Folder | Kind::Document => L10N.tr("hint-file"),
            }
        };
        let color = if key.copied {
            cx.palette.accent_color
        } else {
            muted
        };
        let hint = fitted(cx, &hint, TextStyle::new(HINT_SIZE, color), inner);
        let bottom = frame.y + frame.height - PREVIEW_PADDING + 4.0 - hint.height();
        self.items.push(centered(&hint, frame, bottom.max(y)));
    }
}

impl Widget for PreviewPane {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let key = {
            let state = self.state.borrow();
            PreviewKey {
                revision: state.model.revision,
                selected: state.model.selected,
                copied: state.model.copied,
                loaded: state.preview.is_some(),
                palette: cx.palette,
                frame,
            }
        };
        if self.painted != Some(key) {
            self.painted = Some(key);
            self.compose(cx, key);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        items.extend(self.items.iter().cloned());
    }

    fn access(&self, _cx: &Context) -> Node {
        let state = self.state.borrow();
        let mut node = Node::new(Role::Group);
        node.set_label(L10N.tr("preview"));
        if let Some(hit) = state.model.selection() {
            node.set_description(hit.title.clone());
        }
        node
    }
}

struct Card {
    id: WidgetId,
    state: Shared,
    revision: Option<u64>,
    children: Vec<Box<dyn Widget>>,
    labels: Vec<Item>,
    empty: bool,
    frame: Rect,
}

fn heading_text(heading: Heading) -> String {
    L10N.tr(match heading {
        Heading::TopHit => "top-hit",
        Heading::Applications => "applications",
        Heading::Settings => "settings",
        Heading::Folders => "folders",
        Heading::Documents => "documents",
    })
}

const FIELD: usize = 0;
const PREVIEW: usize = 1;
const FIRST_ROW: usize = 2;

impl Card {
    fn sync_rows(&mut self) {
        let mut state = self.state.borrow_mut();
        if self.revision == Some(state.model.revision) {
            return;
        }
        self.revision = Some(state.model.revision);
        state.hovered = None;
        let hits = state.model.hits.clone();
        let icons: Vec<Option<Image>> = hits.iter().map(|hit| state.catalog.icon(hit)).collect();
        drop(state);
        self.children.truncate(FIRST_ROW);
        for (position, icon) in icons.into_iter().enumerate() {
            self.children.push(Box::new(Row {
                id: WidgetId::next(),
                state: self.state.clone(),
                position,
                icon,
                items: Vec::new(),
                painted: None,
                frame: Rect::default(),
            }));
        }
    }

    fn shows_results(&self) -> bool {
        let state = self.state.borrow();
        !state.model.hits.is_empty()
            || (!state.model.query.trim().is_empty() && state.model.files_complete)
    }

    fn list_height(&self) -> f32 {
        let state = self.state.borrow();
        let headings = state.model.headings.len() as f32;
        let rows = state.model.hits.len() as f32;
        if state.model.hits.is_empty() {
            return NO_RESULTS_HEIGHT;
        }
        (2.0 * PADDING + headings * HEADING_HEIGHT + rows * ROW_HEIGHT).max(RESULTS_MIN_HEIGHT)
    }
}

impl Widget for Card {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        self.sync_rows();
        self.children[FIELD].measure(cx, Size::new(CARD_WIDTH, FIELD_ROW));
        let results = if self.shows_results() {
            self.list_height() + 1.0
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
            self.children[PREVIEW].place(cx, Rect::default());
            return;
        }
        let height = frame.y + frame.height - top;
        let headings = self.state.borrow().model.headings.clone();
        self.empty = headings.is_empty();
        if self.empty {
            let message = cx.text.paragraph(
                &L10N.tr("no-results"),
                TextStyle::new(PREVIEW_TITLE_SIZE, cx.palette.label_muted),
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
        let heading_style =
            TextStyle::new(HEADING_SIZE, cx.palette.label_muted).weight(TextStyle::SEMIBOLD);
        let mut y = top + PADDING;
        let mut next_heading = headings.iter().peekable();
        for (position, child) in self.children[FIRST_ROW..].iter_mut().enumerate() {
            if let Some((_, heading)) = next_heading.next_if(|(start, _)| *start == position) {
                let label = cx.text.paragraph(
                    &heading_text(*heading),
                    heading_style,
                    None,
                    TextAlign::Start,
                );
                self.labels.push(Item::Text {
                    origin: Point::new(
                        frame.x + PADDING + 8.0,
                        (y + HEADING_HEIGHT - label.height() - 3.0).round(),
                    ),
                    paragraph: label,
                });
                y += HEADING_HEIGHT;
            }
            child.place(
                cx,
                Rect::new(frame.x + PADDING, y, LIST_WIDTH - 2.0 * PADDING, ROW_HEIGHT),
            );
            y += ROW_HEIGHT;
        }
        self.children[PREVIEW].place(
            cx,
            Rect::new(
                frame.x + LIST_WIDTH + 1.0,
                top,
                frame.width - LIST_WIDTH - 1.0,
                height,
            ),
        );
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
            paint: cx.palette.surface_elevated.into(),
        });
        items.push(Item::Stroke {
            shape: Shape::rounded(self.frame.inset(0.5), CARD_RADIUS - 0.5),
            width: 1.0,
            paint: cx.palette.separator_color.into(),
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
            paint: cx.palette.label_muted.into(),
        });
        items.push(Item::Line {
            from: Point::new(glyph.x + 13.0, glyph.y + 13.0),
            to: Point::new(glyph.x + 19.0, glyph.y + 19.0),
            width: 2.2,
            paint: cx.palette.label_muted.into(),
        });
        if self.frame.height > FIELD_ROW {
            let top = self.frame.y + FIELD_ROW;
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(self.frame.x, top, self.frame.width, 1.0)),
                paint: cx.palette.separator_color.into(),
            });
            if !self.empty {
                items.push(Item::Fill {
                    shape: Shape::rect(Rect::new(
                        self.frame.x + LIST_WIDTH,
                        top + 1.0,
                        1.0,
                        self.frame.y + self.frame.height - top - 1.0,
                    )),
                    paint: cx.palette.separator_color.into(),
                });
            }
        }
        items.extend(self.labels.iter().cloned());
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Dialog);
        node.set_label(L10N.tr("launcher"));
        node
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

pub struct Launcher {
    state: Shared,
    pub ui: Ui,
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
            requested: None,
            hovered: None,
            sent_generation: 0,
        }));
        let changed = state.clone();
        let field = TextField::new(&L10N.tr("search"), "", move |query| {
            changed.borrow_mut().model.set_query(query);
        })
        .borderless(FIELD_SIZE);
        let field_id = field.id();
        let preview = PreviewPane {
            id: WidgetId::next(),
            state: state.clone(),
            items: Vec::new(),
            painted: None,
            frame: Rect::default(),
        };
        let card = Card {
            id: WidgetId::next(),
            state: state.clone(),
            revision: None,
            children: vec![Box::new(field), Box::new(preview)],
            labels: Vec::new(),
            empty: false,
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
            let model = &mut state.model;
            let handled = match key {
                Key::Down | Key::Up if modifiers.logo => {
                    model.select_next_group(*key == Key::Down);
                    true
                }
                Key::Down => {
                    model.select_next();
                    true
                }
                Key::Up => {
                    model.select_previous();
                    true
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
            if animation.is_settled(SETTLED, SETTLED) {
                1.0
            } else {
                animation.state().position.clamp(0.0, 1.0) as f32
            }
        })
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
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
    }

    pub fn take_actions(&mut self) -> Vec<LauncherAction> {
        std::mem::take(&mut self.state.borrow_mut().model.actions)
    }

    pub fn file_query(&mut self) -> Option<(u64, String)> {
        let mut state = self.state.borrow_mut();
        if state.sent_generation == state.model.generation {
            return None;
        }
        state.sent_generation = state.model.generation;
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
