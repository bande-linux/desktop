use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, Role, TreeId};
use unbound_toolkit::ui::{
    Animated, Context, Event, Key, Menu, MenuItem, Parts, ShortcutText, Size, Spring, TextField,
    Ui, Widget, WidgetId,
};
use unbound_toolkit::{
    Appearance, Color, Item, Paragraph, Point, Radius, Rect, Scene, Shape, TextAlign, TextStyle,
    TextSystem,
};

use crate::L10N;

pub const NAMESPACE: &str = "menu-search";
const PATH_SEPARATOR: &str = " › ";
const CARD_WIDTH: f32 = 560.0;
const FIELD_ROW: f32 = 52.0;
const FIELD_SIZE: f32 = 20.0;
const ROW_HEIGHT: f32 = 32.0;
const VISIBLE_ROWS: usize = 10;
const PADDING: f32 = 8.0;
const TEXT_INSET: f32 = 12.0;
const TEXT_SIZE: f32 = 13.0;
const SHORTCUT_GAP: f32 = 24.0;
const TOP_FRACTION: f32 = 0.22;
const APPEAR_RISE: f32 = 8.0;
const SHADOW_BLUR: f32 = 24.0;

fn leaf(title: &str) -> &str {
    title.rsplit(PATH_SEPARATOR).next().unwrap_or(title)
}

fn rank(title: &str, query: &str) -> u8 {
    let leaf = leaf(title).to_lowercase();
    if leaf.starts_with(query) {
        0
    } else if leaf
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| word.starts_with(query))
    {
        1
    } else {
        2
    }
}

pub fn search(menu: &Menu, query: &str) -> Vec<MenuItem> {
    let query = query.trim().to_lowercase();
    let mut found = menu.matching(&query);
    found.sort_by_key(|item| rank(&item.title, &query));
    found
}

struct Model {
    menu: Menu,
    query: String,
    found: Vec<MenuItem>,
    selected: usize,
    first: usize,
    chosen: Option<String>,
    closing: bool,
}

impl Model {
    fn set_query(&mut self, query: &str) {
        query.clone_into(&mut self.query);
        self.found = search(&self.menu, query);
        self.selected = 0;
        self.first = 0;
    }

    fn select(&mut self, index: usize) {
        let Some(last) = self.found.len().checked_sub(1) else {
            return;
        };
        self.selected = index.min(last);
        if self.selected < self.first {
            self.first = self.selected;
        } else if self.selected >= self.first + VISIBLE_ROWS {
            self.first = self.selected + 1 - VISIBLE_ROWS;
        }
    }

    fn choose(&mut self, index: usize) {
        if let Some(item) = self.found.get(index) {
            self.chosen = Some(item.action.clone());
        }
    }
}

type Shared = Rc<RefCell<Model>>;

struct Row {
    index: usize,
    frame: Rect,
    path: Option<Paragraph>,
    title: Paragraph,
    shortcut: Option<(ShortcutText, Color)>,
    selected: bool,
}

struct Card {
    id: WidgetId,
    model: Shared,
    children: Vec<Box<dyn Widget>>,
    rows: Vec<Row>,
    empty: Option<Paragraph>,
    appear: Animated,
    frame: Rect,
}

fn fitted(cx: &mut Context, text: &str, style: TextStyle, width: f32) -> Paragraph {
    let mut characters: Vec<char> = text.chars().collect();
    let mut shown = text.to_owned();
    loop {
        let paragraph = cx.text.paragraph(&shown, style, None, TextAlign::Start);
        if paragraph.width() <= width || characters.len() <= 1 {
            return paragraph;
        }
        characters.pop();
        shown = format!("{}…", characters.iter().collect::<String>().trim_end());
    }
}

impl Card {
    fn list_top(&self) -> f32 {
        self.frame.y + FIELD_ROW + PADDING
    }

    fn compose_rows(&mut self, cx: &mut Context) {
        let model = self.model.borrow();
        let width = self.frame.width - 2.0 * PADDING;
        let mut rows = Vec::new();
        for (offset, item) in model
            .found
            .iter()
            .enumerate()
            .skip(model.first)
            .take(VISIBLE_ROWS)
            .map(|(index, item)| (index - model.first, (index, item)))
        {
            let (index, item) = item;
            let selected = index == model.selected;
            let (ink, muted) = if selected {
                (cx.palette.on_accent, cx.palette.on_accent)
            } else {
                (cx.palette.ink, cx.palette.ink2)
            };
            let shortcut = item.shortcut.as_deref().map(|shortcut| {
                (
                    ShortcutText::new(&mut cx.text, shortcut, TextStyle::new(TEXT_SIZE, muted)),
                    muted,
                )
            });
            let room = width
                - 2.0 * TEXT_INSET
                - shortcut
                    .as_ref()
                    .map_or(0.0, |(shortcut, _)| shortcut.width() + SHORTCUT_GAP);
            let title = leaf(&item.title);
            let prefix = item
                .title
                .strip_suffix(title)
                .filter(|path| !path.is_empty());
            let title_style = TextStyle::new(TEXT_SIZE, ink).weight(TextStyle::MEDIUM);
            let title = fitted(cx, title, title_style, room);
            let path = prefix.map(|path| {
                fitted(
                    cx,
                    path,
                    TextStyle::new(TEXT_SIZE, muted),
                    (room - title.width()).max(0.0),
                )
            });
            rows.push(Row {
                index,
                frame: Rect::new(
                    self.frame.x + PADDING,
                    self.list_top() + offset as f32 * ROW_HEIGHT,
                    width,
                    ROW_HEIGHT,
                ),
                path,
                title,
                shortcut,
                selected,
            });
        }
        self.empty = model.found.is_empty().then(|| {
            cx.text.paragraph(
                &L10N.tr("no-menu-results"),
                TextStyle::new(TEXT_SIZE, cx.palette.ink2),
                None,
                TextAlign::Start,
            )
        });
        drop(model);
        self.rows = rows;
    }

    fn row_at(&self, point: Point) -> Option<usize> {
        self.rows
            .iter()
            .find(|row| row.frame.contains(point))
            .map(|row| row.index)
    }

    fn paint_row(&self, cx: &Context, row: &Row, items: &mut Vec<Item>) {
        if row.selected {
            items.push(Item::Fill {
                shape: Shape::rounded(row.frame, Radius::S),
                paint: cx.palette.accent.into(),
            });
        }
        let baseline = |paragraph: &Paragraph| {
            (row.frame.y + (row.frame.height - paragraph.height()) / 2.0).round()
        };
        let mut x = row.frame.x + TEXT_INSET;
        if let Some(path) = &row.path {
            items.push(Item::Text {
                origin: Point::new(x, baseline(path)),
                paragraph: path.clone(),
            });
            x += path.width();
        }
        items.push(Item::Text {
            origin: Point::new(x.round(), baseline(&row.title)),
            paragraph: row.title.clone(),
        });
        if let Some((shortcut, color)) = &row.shortcut {
            shortcut.paint(
                Point::new(
                    (row.frame.x + row.frame.width - TEXT_INSET - shortcut.width()).round(),
                    (row.frame.y + (row.frame.height - shortcut.height()) / 2.0).round(),
                ),
                *color,
                items,
            );
        }
    }
}

impl Widget for Card {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, _available: Size) -> Size {
        let model = self.model.borrow();
        let rows = model.found.len().clamp(1, VISIBLE_ROWS) as f32;
        Size::new(CARD_WIDTH, FIELD_ROW + 2.0 * PADDING + rows * ROW_HEIGHT)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let field = &mut self.children[0];
        let width = frame.width - 2.0 * (PADDING + TEXT_INSET);
        let height = field.measure(cx, Size::new(width, FIELD_ROW)).height;
        field.place(
            cx,
            Rect::new(
                frame.x + PADDING + TEXT_INSET,
                frame.y + ((FIELD_ROW - height) / 2.0).round(),
                width,
                height,
            ),
        );
        self.compose_rows(cx);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shown = self.appear.value(cx).clamp(0.0, 1.0);
        let shape = Shape::rounded(self.frame, Radius::XL);
        let mut card = vec![
            Item::Shadow {
                shape,
                offset: Point::new(0.0, 8.0),
                blur_radius: SHADOW_BLUR,
                color: Color::hex(0x000000).with_alpha(60),
            },
            Item::Fill {
                shape,
                paint: cx.palette.overlay.into(),
            },
            Item::Stroke {
                shape: Shape::rounded(self.frame.inset(0.5), Radius::XL - 0.5),
                width: 1.0,
                paint: cx.palette.edge.into(),
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(
                    self.frame.x,
                    self.frame.y + FIELD_ROW,
                    self.frame.width,
                    1.0,
                )),
                paint: cx.palette.hairline.into(),
            },
        ];
        for child in &self.children {
            child.paint(cx, &mut card);
        }
        for row in &self.rows {
            self.paint_row(cx, row, &mut card);
        }
        if let Some(empty) = &self.empty {
            card.push(Item::Text {
                origin: Point::new(
                    (self.frame.x + (self.frame.width - empty.width()) / 2.0).round(),
                    (self.list_top() + (ROW_HEIGHT - empty.height()) / 2.0).round(),
                ),
                paragraph: empty.clone(),
            });
        }
        let rise = if cx.reduce_motion {
            0.0
        } else {
            (APPEAR_RISE * (1.0 - shown)).round()
        };
        items.push(Item::Opacity {
            alpha: shown,
            items: vec![Item::Translate {
                offset: Point::new(0.0, rise),
                items: card,
            }],
        });
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Dialog);
        node.set_label(L10N.tr("search-menus"));
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerMove(pointer) => {
                let Some(index) = self.row_at(pointer.position) else {
                    return false;
                };
                let mut model = self.model.borrow_mut();
                if model.selected == index {
                    return false;
                }
                model.select(index);
                true
            }
            Event::PointerDown(pointer) if pointer.is_primary() => {
                if let Some(index) = self.row_at(pointer.position) {
                    self.model.borrow_mut().choose(index);
                }
                true
            }
            _ => false,
        }
    }

    fn parts(&self, _cx: &Context) -> Parts {
        let nodes: Vec<(NodeId, Node)> = self
            .rows
            .iter()
            .map(|row| {
                let mut node = Node::new(Role::ListBoxOption);
                let model = self.model.borrow();
                if let Some(item) = model.found.get(row.index) {
                    node.set_label(item.title.clone());
                    if let Some(shortcut) = &item.shortcut {
                        node.set_description(shortcut.clone());
                    }
                }
                node.set_selected(row.selected);
                node.add_action(Action::Click);
                node.set_bounds(accesskit::Rect {
                    x0: f64::from(row.frame.x),
                    y0: f64::from(row.frame.y),
                    x1: f64::from(row.frame.x + row.frame.width),
                    y1: f64::from(row.frame.y + row.frame.height),
                });
                (self.id.part(row.index as u64), node)
            })
            .collect();
        Parts {
            children: nodes.iter().map(|(id, _)| *id).collect(),
            nodes,
            focus: None,
        }
    }

    fn perform_part(&mut self, _cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        if request.action != Action::Click {
            return false;
        }
        self.model.borrow_mut().choose(part as usize);
        true
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
    model: Shared,
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
            && !self.children[0].frame().contains(pointer.position)
        {
            self.model.borrow_mut().closing = true;
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

pub struct MenuSearch {
    model: Shared,
    ui: Ui,
    card: WidgetId,
    field: WidgetId,
}

impl MenuSearch {
    pub fn new(appearance: Appearance, text: TextSystem) -> Self {
        let model: Shared = Rc::new(RefCell::new(Model {
            menu: Menu::default(),
            query: String::new(),
            found: Vec::new(),
            selected: 0,
            first: 0,
            chosen: None,
            closing: false,
        }));
        let changed = model.clone();
        let field = TextField::new(&L10N.tr("search-menus"), "", move |query| {
            changed.borrow_mut().set_query(query);
        })
        .borderless(FIELD_SIZE);
        let field_id = field.id();
        let card = Card {
            id: WidgetId::next(),
            model: model.clone(),
            children: vec![Box::new(field)],
            rows: Vec::new(),
            empty: None,
            appear: Animated::new(0.0),
            frame: Rect::default(),
        };
        let card_id = card.id;
        let backdrop = Backdrop {
            id: WidgetId::next(),
            model: model.clone(),
            children: vec![Box::new(card)],
            frame: Rect::default(),
        };
        let mut ui = Ui::new(Box::new(backdrop), Context::new(text, appearance.palette()));
        ui.set_appearance(appearance);
        Self {
            model,
            ui,
            card: card_id,
            field: field_id,
        }
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    pub fn set_clock(&mut self, clock: impl Fn() -> Instant + 'static) {
        self.ui.set_clock(clock);
    }

    pub fn open(&mut self, menu: Menu) {
        self.request(Action::SetValue, Some(ActionData::Value("".into())));
        self.request(Action::Focus, None);
        let mut model = self.model.borrow_mut();
        model.menu = menu;
        model.set_query("");
        model.chosen = None;
        model.closing = false;
        drop(model);
        let reduce_motion = self.ui.context().reduce_motion;
        let now = self.ui.context().now();
        if let Some(card) = self.ui.widget_as::<Card>(self.card) {
            card.appear.jump_to(0.0);
            card.appear
                .animate_at(now, reduce_motion, 1.0, Spring::QUICK);
        }
    }

    fn request(&mut self, action: Action, data: Option<ActionData>) {
        self.ui.perform(&ActionRequest {
            action,
            target_tree: TreeId::ROOT,
            target_node: self.field.into(),
            data,
        });
    }

    pub fn results(&self) -> Vec<(String, Option<String>)> {
        self.model
            .borrow()
            .found
            .iter()
            .map(|item| (item.title.clone(), item.shortcut.clone()))
            .collect()
    }

    pub fn selected(&self) -> usize {
        self.model.borrow().selected
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        if let Event::Key { key, modifiers } = event {
            let mut model = self.model.borrow_mut();
            let selected = model.selected;
            match key {
                Key::Down => model.select(selected + 1),
                Key::Up => model.select(selected.saturating_sub(1)),
                Key::PageDown => model.select(selected + VISIBLE_ROWS),
                Key::PageUp => model.select(selected.saturating_sub(VISIBLE_ROWS)),
                Key::Enter if !modifiers.logo => model.choose(selected),
                Key::Escape => {
                    let query_empty = model.query.is_empty();
                    drop(model);
                    if query_empty {
                        self.model.borrow_mut().closing = true;
                    } else {
                        self.request(Action::SetValue, Some(ActionData::Value("".into())));
                        self.model.borrow_mut().set_query("");
                    }
                }
                _ => {
                    drop(model);
                    return self.ui.handle(event);
                }
            }
            return true;
        }
        self.ui.handle(event)
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        self.ui.perform(request)
    }

    pub fn take_chosen(&mut self) -> Option<String> {
        self.model.borrow_mut().chosen.take()
    }

    pub fn take_closing(&mut self) -> bool {
        std::mem::take(&mut self.model.borrow_mut().closing)
    }

    pub fn cursor(&self) -> unbound_toolkit::ui::Cursor {
        self.ui.cursor()
    }

    pub fn is_animating(&self) -> bool {
        self.ui.is_animating()
    }

    pub fn accessibility(&self) -> accesskit::TreeUpdate {
        self.ui.accessibility()
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        let mut scene = self.ui.scene(width, height);
        scene.background = Color::TRANSPARENT;
        scene
    }
}
