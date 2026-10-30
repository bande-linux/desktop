use accesskit::{Action, ActionRequest, Node, Role};
use unbound_toolkit::ui::{Context, Event, Menu, MenuItem, Size, Widget, WidgetId};
use unbound_toolkit::{
    Color, Image, Item, Palette, Paragraph, Point, Rect, Shape, TextAlign, TextStyle,
};

use crate::L10N;
use crate::model::{Hit, Kind, Model, Target};
use crate::view::Shared;

pub const ROW_HEIGHT: f32 = 32.0;
pub const CELL_HEIGHT: f32 = 96.0;
const ROW_RADIUS: f32 = 7.0;
const ROW_ICON: f32 = 22.0;
const TITLE_SIZE: f32 = 14.0;
const CELL_RADIUS: f32 = 10.0;
const CELL_ICON: f32 = 48.0;
const CELL_TITLE_SIZE: f32 = 11.5;
const CELL_TITLE_LINES: usize = 2;
pub const SELECTED_LABEL: Color = Color::hex(0xFFFFFF);

pub const OPEN: &str = "open";
pub const REVEAL: &str = "reveal";
pub const COPY: &str = "copy";

pub fn contains(frame: Rect, point: Point) -> bool {
    point.x >= frame.x
        && point.y >= frame.y
        && point.x < frame.x + frame.width
        && point.y < frame.y + frame.height
}

pub fn clipped(frame: Rect, viewport: Rect) -> Rect {
    let left = frame.x.max(viewport.x);
    let top = frame.y.max(viewport.y);
    let right = (frame.x + frame.width).min(viewport.x + viewport.width);
    let bottom = (frame.y + frame.height).min(viewport.y + viewport.height);
    if right <= left || bottom <= top {
        return Rect::default();
    }
    Rect::new(left, top, right - left, bottom - top)
}

pub fn fitted(cx: &mut Context, text: &str, style: TextStyle, width: f32) -> Paragraph {
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

fn highlighted(
    cx: &mut Context,
    text: &str,
    matched: &[usize],
    style: TextStyle,
    width: f32,
) -> Option<Vec<(f32, Paragraph)>> {
    if matched.is_empty() {
        return None;
    }
    let mut runs: Vec<(bool, String)> = Vec::new();
    for (index, character) in text.chars().enumerate() {
        let strong = matched.contains(&index);
        match runs.last_mut() {
            Some((last, run)) if *last == strong => run.push(character),
            _ => runs.push((strong, character.to_string())),
        }
    }
    let mut x = 0.0;
    let mut placed = Vec::with_capacity(runs.len());
    for (strong, run) in runs {
        let style = if strong {
            style.weight(TextStyle::SEMIBOLD)
        } else {
            style
        };
        let paragraph = cx.text.paragraph(&run, style, None, TextAlign::Start);
        let advance = paragraph.width();
        placed.push((x, paragraph));
        x += advance;
    }
    (x <= width).then_some(placed)
}

pub fn centered(paragraph: &Paragraph, frame: Rect, y: f32) -> Item {
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

pub fn icon_items(
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

pub fn hit_menu(hit: &Hit) -> Menu {
    let items = match &hit.target {
        Target::Value(_) => {
            vec![MenuItem::new(&L10N.tr("menu-copy"), COPY).shortcut("Super+C")]
        }
        Target::Path(_) => vec![
            MenuItem::new(&L10N.tr("menu-open"), OPEN).shortcut("Return"),
            MenuItem::new(&L10N.tr("menu-show-in-files"), REVEAL).shortcut("Super+Return"),
            MenuItem::separator(),
            MenuItem::new(&L10N.tr("menu-copy-path"), COPY).shortcut("Super+C"),
        ],
        Target::App(_) | Target::Settings(_) => {
            vec![MenuItem::new(&L10N.tr("menu-open"), OPEN).shortcut("Return")]
        }
    };
    Menu::new(items)
}

pub fn apply_menu_action(model: &mut Model, action: &str) -> bool {
    match action {
        OPEN => model.activate(),
        REVEAL => model.reveal(),
        COPY => return model.copy(),
        _ => return false,
    }
    true
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Row,
    Cell,
}

pub struct Choice {
    id: WidgetId,
    state: Shared,
    position: usize,
    shape: Form,
    icon: Option<Image>,
    items: Vec<Item>,
    painted: Option<(bool, bool, Palette, Rect)>,
    rect: Rect,
    visible: Rect,
}

impl Choice {
    pub fn new(state: Shared, position: usize, shape: Form, icon: Option<Image>) -> Self {
        Self {
            id: WidgetId::next(),
            state,
            position,
            shape,
            icon,
            items: Vec::new(),
            painted: None,
            rect: Rect::default(),
            visible: Rect::default(),
        }
    }

    fn compose(&mut self, cx: &mut Context, hit: &Hit, selected: bool, hovered: bool) {
        self.items.clear();
        let radius = match self.shape {
            Form::Row => ROW_RADIUS,
            Form::Cell => CELL_RADIUS,
        };
        let background = match self.shape {
            Form::Row => self.rect,
            Form::Cell => self.rect.inset(2.0),
        };
        if selected {
            self.items.push(Item::Fill {
                shape: Shape::rounded(background, radius),
                paint: cx.palette.accent_color.into(),
            });
        } else if hovered {
            self.items.push(Item::Fill {
                shape: Shape::rounded(background, radius),
                paint: cx.palette.label_primary.with_alpha(16).into(),
            });
        }
        let color = if selected {
            SELECTED_LABEL
        } else {
            cx.palette.label_primary
        };
        match self.shape {
            Form::Row => self.compose_row(cx, hit, selected, color),
            Form::Cell => self.compose_cell(cx, hit, selected, color),
        }
    }

    fn compose_row(&mut self, cx: &mut Context, hit: &Hit, selected: bool, color: Color) {
        let icon = Rect::new(
            self.rect.x + 8.0,
            self.rect.y + ((self.rect.height - ROW_ICON) / 2.0).round(),
            ROW_ICON,
            ROW_ICON,
        );
        icon_items(cx, hit, selected, self.icon.as_ref(), icon, &mut self.items);
        let x = icon.x + icon.width + 10.0;
        let width = self.rect.x + self.rect.width - x - 8.0;
        let style = TextStyle::new(TITLE_SIZE, color);
        let runs = highlighted(cx, &hit.title, &hit.matched, style, width)
            .unwrap_or_else(|| vec![(0.0, fitted(cx, &hit.title, style, width))]);
        for (offset, paragraph) in runs {
            self.items.push(Item::Text {
                origin: Point::new(
                    x + offset,
                    (self.rect.y + (self.rect.height - paragraph.height()) / 2.0).round(),
                ),
                paragraph,
            });
        }
    }

    fn compose_cell(&mut self, cx: &mut Context, hit: &Hit, selected: bool, color: Color) {
        let icon = Rect::new(
            (self.rect.x + (self.rect.width - CELL_ICON) / 2.0).round(),
            self.rect.y + 10.0,
            CELL_ICON,
            CELL_ICON,
        );
        icon_items(cx, hit, selected, self.icon.as_ref(), icon, &mut self.items);
        let width = self.rect.width - 10.0;
        let style = TextStyle::new(CELL_TITLE_SIZE, color);
        let y = icon.y + CELL_ICON + 6.0;
        let wrapped = cx
            .text
            .paragraph(&hit.title, style, Some(width), TextAlign::Center);
        if wrapped.line_count() > CELL_TITLE_LINES {
            let title = fitted(cx, &hit.title, style, width);
            self.items.push(centered(&title, self.rect, y));
        } else {
            self.items.push(Item::Text {
                origin: Point::new((self.rect.x + (self.rect.width - width) / 2.0).round(), y),
                paragraph: wrapped,
            });
        }
    }
}

impl Widget for Choice {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        let height = match self.shape {
            Form::Row => ROW_HEIGHT,
            Form::Cell => CELL_HEIGHT,
        };
        Size::new(available.width, height)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.rect = frame;
        let mut state = self.state.borrow_mut();
        self.visible = clipped(frame, state.viewport);
        let selected = state.model.selected == self.position;
        if selected {
            state.selected_frame = self.visible;
        }
        if self.visible == Rect::default() {
            self.painted = None;
            self.items.clear();
            return;
        }
        let hovered = state.hovered == Some(self.position);
        let key = (selected, hovered, cx.palette, self.rect);
        if self.painted == Some(key) {
            return;
        }
        self.painted = Some(key);
        let Some(hit) = state.model.hits.get(self.position).cloned() else {
            self.items.clear();
            return;
        };
        drop(state);
        self.compose(cx, &hit, selected, hovered);
    }

    fn frame(&self) -> Rect {
        self.visible
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
                if pointer.is_primary() && contains(self.visible, pointer.position) =>
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
        state.model.hits.get(self.position).map(hit_menu)
    }

    fn menu_action(&mut self, _cx: &mut Context, action: &str) -> bool {
        let mut state = self.state.borrow_mut();
        state.model.select(self.position);
        apply_menu_action(&mut state.model, action)
    }
}
