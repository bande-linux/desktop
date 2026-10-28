use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Node, Role};
use unbound_toolkit::ui::{Context, Event, Key, Size, TextField, Ui, Widget, WidgetId};
use unbound_toolkit::{
    Color, Image, Item, Palette, Paragraph, Point, Rect, Scene, Shape, TextAlign, TextStyle,
    TextSystem,
};

pub const CARD_WIDTH: f32 = 680.0;
pub const ICON_SIZE: u32 = 32;
pub const MAX_RESULTS: usize = 8;
const CARD_RADIUS: f32 = 12.0;
const FIELD_SIZE: f32 = 22.0;
const PADDING: f32 = 14.0;
const ROW_HEIGHT: f32 = 44.0;
const ROW_RADIUS: f32 = 8.0;
const NAME_SIZE: f32 = 15.0;
const TOP_FRACTION: f32 = 0.24;

#[derive(Debug, Clone)]
pub struct LauncherApp {
    pub app_id: String,
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    pub keywords: Vec<String>,
    pub icon: Option<Image>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherAction {
    Open(String),
    Close,
}

fn words_start_with(text: &str, query: &str) -> bool {
    text.split(|character: char| !character.is_alphanumeric())
        .any(|word| word.starts_with(query))
}

pub fn rank(app: &LauncherApp, query: &str) -> Option<u8> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return None;
    }
    let name = app.name.to_lowercase();
    if name.starts_with(&query) {
        return Some(0);
    }
    if words_start_with(&name, &query) {
        return Some(1);
    }
    let described = app
        .generic_name
        .iter()
        .chain(&app.keywords)
        .any(|text| words_start_with(&text.to_lowercase(), &query));
    if described {
        return Some(2);
    }
    let commented = app
        .comment
        .as_ref()
        .is_some_and(|comment| words_start_with(&comment.to_lowercase(), &query));
    commented.then_some(3)
}

pub fn search(apps: &[LauncherApp], query: &str) -> Vec<usize> {
    let mut ranked: Vec<(u8, usize)> = apps
        .iter()
        .enumerate()
        .filter_map(|(index, app)| rank(app, query).map(|rank| (rank, index)))
        .collect();
    ranked.sort_by(|a, b| {
        a.0.cmp(&b.0).then_with(|| {
            apps[a.1]
                .name
                .to_lowercase()
                .cmp(&apps[b.1].name.to_lowercase())
        })
    });
    ranked
        .into_iter()
        .map(|(_, index)| index)
        .take(MAX_RESULTS)
        .collect()
}

pub struct Model {
    pub apps: Vec<LauncherApp>,
    pub query: String,
    pub results: Vec<usize>,
    pub selected: usize,
    pub actions: Vec<LauncherAction>,
    generation: u64,
}

impl Model {
    pub fn new(apps: Vec<LauncherApp>) -> Self {
        Self {
            apps,
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            actions: Vec::new(),
            generation: 0,
        }
    }

    fn set_query(&mut self, query: &str) {
        self.query = query.to_owned();
        self.results = search(&self.apps, query);
        self.selected = 0;
        self.generation += 1;
    }

    pub fn select_next(&mut self) {
        if !self.results.is_empty() {
            self.selected = (self.selected + 1).min(self.results.len() - 1);
        }
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn open_selected(&mut self) {
        if let Some(index) = self.results.get(self.selected) {
            let app_id = self.apps[*index].app_id.clone();
            self.actions.push(LauncherAction::Open(app_id));
        }
    }
}

pub type Shared = Rc<RefCell<Model>>;

fn contains(frame: Rect, point: Point) -> bool {
    point.x >= frame.x
        && point.y >= frame.y
        && point.x < frame.x + frame.width
        && point.y < frame.y + frame.height
}

struct ResultRow {
    id: WidgetId,
    model: Shared,
    position: usize,
    name: Option<Paragraph>,
    frame: Rect,
}

impl Widget for ResultRow {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let model = self.model.borrow();
        let name = model
            .results
            .get(self.position)
            .map(|index| model.apps[*index].name.clone())
            .unwrap_or_default();
        let selected = model.selected == self.position;
        let color = if selected {
            Color::hex(0xFFFFFF)
        } else {
            cx.palette.label_primary
        };
        self.name = Some(cx.text.paragraph(
            &name,
            TextStyle::new(NAME_SIZE, color),
            None,
            TextAlign::Start,
        ));
        Size::new(available.width, ROW_HEIGHT)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let model = self.model.borrow();
        let Some(index) = model.results.get(self.position) else {
            return;
        };
        if model.selected == self.position {
            items.push(Item::Fill {
                shape: Shape::rounded(self.frame, ROW_RADIUS),
                paint: cx.palette.accent_color.into(),
            });
        }
        let icon = Rect::new(
            self.frame.x + 8.0,
            self.frame.y + ((self.frame.height - ICON_SIZE as f32) / 2.0).round(),
            ICON_SIZE as f32,
            ICON_SIZE as f32,
        );
        match &model.apps[*index].icon {
            Some(image) => items.push(Item::Image {
                rect: icon,
                image: image.clone(),
            }),
            None => items.push(Item::Fill {
                shape: Shape::rounded(icon, 7.0),
                paint: Color::hex(0x7A8599).into(),
            }),
        }
        if let Some(name) = &self.name {
            items.push(Item::Text {
                origin: Point::new(
                    icon.x + icon.width + 10.0,
                    self.frame.y + ((self.frame.height - name.height()) / 2.0).round(),
                ),
                paragraph: name.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let model = self.model.borrow();
        let mut node = Node::new(Role::ListBoxOption);
        if let Some(index) = model.results.get(self.position) {
            node.set_label(model.apps[*index].name.clone());
        }
        if model.selected == self.position {
            node.set_selected(true);
        }
        node.add_action(Action::Click);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(_) => {
                self.model.borrow_mut().selected = self.position;
                true
            }
            Event::PointerUp(point) if contains(self.frame, *point) => {
                let mut model = self.model.borrow_mut();
                model.selected = self.position;
                model.open_selected();
                true
            }
            _ => false,
        }
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action != Action::Click {
            return false;
        }
        let mut model = self.model.borrow_mut();
        model.selected = self.position;
        model.open_selected();
        true
    }
}

struct Card {
    id: WidgetId,
    model: Shared,
    generation: Option<u64>,
    children: Vec<Box<dyn Widget>>,
    frame: Rect,
}

impl Card {
    fn sync_rows(&mut self) {
        let model = self.model.borrow();
        if self.generation == Some(model.generation) {
            return;
        }
        self.generation = Some(model.generation);
        let count = model.results.len();
        drop(model);
        self.children.truncate(1);
        for position in 0..count {
            self.children.push(Box::new(ResultRow {
                id: WidgetId::next(),
                model: self.model.clone(),
                position,
                name: None,
                frame: Rect::default(),
            }));
        }
    }
}

impl Widget for Card {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        self.sync_rows();
        let inner = CARD_WIDTH - 2.0 * PADDING;
        let rows = self.children.len() > 1;
        let mut height = PADDING;
        for (index, child) in self.children.iter_mut().enumerate() {
            let size = child.measure(cx, Size::new(inner, ROW_HEIGHT));
            height += size.height;
            if index == 0 && rows {
                height += PADDING;
            }
        }
        Size::new(CARD_WIDTH, height + PADDING)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let inner = CARD_WIDTH - 2.0 * PADDING;
        let mut y = frame.y + PADDING;
        let rows = self.children.len() > 1;
        for (index, child) in self.children.iter_mut().enumerate() {
            let size = child.measure(cx, Size::new(inner, ROW_HEIGHT));
            child.place(cx, Rect::new(frame.x + PADDING, y, inner, size.height));
            y += size.height;
            if index == 0 && rows {
                y += PADDING;
            }
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, CARD_RADIUS);
        items.push(Item::Shadow {
            shape,
            offset: Point::new(0.0, 12.0),
            blur_radius: 40.0,
            color: Color::hex(0x000000).with_alpha(90),
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
        if self.children.len() > 1 {
            let field = self.children[0].frame();
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    self.frame.x,
                    field.y + field.height + PADDING / 2.0,
                    self.frame.width,
                    1.0,
                )),
                paint: cx.palette.separator_color.into(),
            });
        }
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Dialog);
        node.set_label("Launcher");
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
        if let Event::PointerDown(_) = event {
            self.model.borrow_mut().actions.push(LauncherAction::Close);
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
    pub model: Shared,
    pub ui: Ui,
    field: WidgetId,
}

impl Launcher {
    pub fn new(apps: Vec<LauncherApp>, palette: Palette, text: TextSystem) -> Self {
        let model: Shared = Rc::new(RefCell::new(Model::new(apps)));
        let changed = model.clone();
        let field = TextField::new("Search", "", move |query| {
            changed.borrow_mut().set_query(query);
        })
        .borderless(FIELD_SIZE);
        let field_id = field.id();
        let card = Card {
            id: WidgetId::next(),
            model: model.clone(),
            generation: None,
            children: vec![Box::new(field)],
            frame: Rect::default(),
        };
        let backdrop = Backdrop {
            id: WidgetId::next(),
            model: model.clone(),
            children: vec![Box::new(card)],
            frame: Rect::default(),
        };
        let mut ui = Ui::new(Box::new(backdrop), Context::new(text, palette));
        ui.perform(&ActionRequest {
            action: Action::Focus,
            target_tree: accesskit::TreeId::ROOT,
            target_node: field_id.into(),
            data: None,
        });
        Self {
            model,
            ui,
            field: field_id,
        }
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        if let Event::Key { key, modifiers } = event
            && !modifiers.logo
        {
            let mut model = self.model.borrow_mut();
            match key {
                Key::Down => {
                    model.select_next();
                    return true;
                }
                Key::Up => {
                    model.select_previous();
                    return true;
                }
                Key::Enter => {
                    model.open_selected();
                    return true;
                }
                Key::Escape => {
                    model.actions.push(LauncherAction::Close);
                    return true;
                }
                _ => {}
            }
        }
        self.ui.handle(event)
    }

    pub fn clear(&mut self) {
        self.ui.perform(&ActionRequest {
            action: Action::SetValue,
            target_tree: accesskit::TreeId::ROOT,
            target_node: self.field.into(),
            data: Some(accesskit::ActionData::Value("".into())),
        });
        self.model.borrow_mut().set_query("");
    }

    pub fn take_actions(&mut self) -> Vec<LauncherAction> {
        std::mem::take(&mut self.model.borrow_mut().actions)
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        let mut scene = self.ui.scene(width, height);
        scene.background = Color::TRANSPARENT;
        scene
    }
}
