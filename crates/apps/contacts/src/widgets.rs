use accesskit::{Action, ActionRequest, Node, Role};
use bande_l10n::tr;
use bande_toolkit::ui::{
    BUTTON_HEIGHT, Context, Event, Parts, PointerButton, Size, TEXT_FIELD_HEIGHT, Widget, WidgetId,
    access_rect,
};
use bande_toolkit::{
    Color, HEADER_LEADING, HEADER_TRAILING, Image, Item, Paragraph, Point, Radius, Rect,
    SHEET_INSET, Shape, TextAlign, TextRole, TextStyle, WINDOW_HEADER_HEIGHT,
};

pub const SIDEBAR_WIDTH: f32 = 210.0;
pub const LIST_WIDTH: f32 = 300.0;
pub const TOOLBAR_HEIGHT: f32 = WINDOW_HEADER_HEIGHT as f32;
const SIDEBAR_INSET: f32 = 8.0;
const SEARCH_PADDING: f32 = 10.0;
const INDEX_WIDTH: f32 = 20.0;
const TOOL_GAP: f32 = 6.0;

/// The window: the sidebar of address books and groups, the searchable list with its letter
/// index, the contact, and the tools in the title bar.
pub struct Panes {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    sizes: Vec<Size>,
    inset: f32,
    frame: Rect,
}

impl Panes {
    const SIDEBAR: usize = 0;
    const DETAIL: usize = 4;
    const TOOLS: usize = 5;

    pub fn new(
        sidebar: Box<dyn Widget>,
        search: Box<dyn Widget>,
        list: Box<dyn Widget>,
        index: Box<dyn Widget>,
        detail: Box<dyn Widget>,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            children: vec![sidebar, search, list, index, detail],
            sizes: Vec::new(),
            inset: SHEET_INSET as f32,
            frame: Rect::default(),
        }
    }

    pub fn set_inset(&mut self, inset: f32) {
        self.inset = inset;
    }

    pub fn set_detail(&mut self, detail: Box<dyn Widget>) {
        self.children[Self::DETAIL] = detail;
    }

    pub fn set_tools(&mut self, tools: Vec<Box<dyn Widget>>) {
        self.children.truncate(Self::TOOLS);
        self.children.extend(tools);
    }

    fn sheet(&self) -> Rect {
        Rect::new(
            SIDEBAR_WIDTH,
            TOOLBAR_HEIGHT,
            (self.frame.width - SIDEBAR_WIDTH - self.inset).max(0.0),
            (self.frame.height - TOOLBAR_HEIGHT - self.inset).max(0.0),
        )
    }

    pub fn tool_frames(&self) -> Vec<Rect> {
        let trailing = match self.inset > 0.0 {
            true => HEADER_TRAILING,
            false => HEADER_LEADING,
        };
        let mut x = self.frame.width - trailing;
        let mut frames: Vec<Rect> = self
            .sizes
            .iter()
            .rev()
            .map(|size| {
                x -= size.width;
                let frame = Rect::new(
                    x,
                    ((TOOLBAR_HEIGHT - BUTTON_HEIGHT) / 2.0).round(),
                    size.width,
                    BUTTON_HEIGHT,
                );
                x -= TOOL_GAP;
                frame
            })
            .collect();
        frames.reverse();
        frames
    }

    fn frames(&self) -> Vec<Rect> {
        let sheet = self.sheet();
        let list = LIST_WIDTH.min(sheet.width / 2.0);
        let search = TEXT_FIELD_HEIGHT + 2.0 * SEARCH_PADDING;
        vec![
            Rect::new(
                SIDEBAR_INSET,
                TOOLBAR_HEIGHT,
                SIDEBAR_WIDTH - 2.0 * SIDEBAR_INSET,
                (self.frame.height - TOOLBAR_HEIGHT).max(0.0),
            ),
            Rect::new(
                sheet.x + SEARCH_PADDING,
                sheet.y + SEARCH_PADDING,
                list - 2.0 * SEARCH_PADDING,
                TEXT_FIELD_HEIGHT,
            ),
            Rect::new(
                sheet.x,
                sheet.y + search,
                (list - INDEX_WIDTH).max(0.0),
                (sheet.height - search).max(0.0),
            ),
            Rect::new(
                sheet.x + list - INDEX_WIDTH,
                sheet.y + search,
                INDEX_WIDTH,
                (sheet.height - search).max(0.0),
            ),
            Rect::new(
                sheet.x + list + 1.0,
                sheet.y,
                (sheet.width - list - 1.0).max(0.0),
                sheet.height,
            ),
        ]
    }
}

impl Widget for Panes {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.sizes = self
            .children
            .iter_mut()
            .skip(Self::TOOLS)
            .map(|tool| tool.measure(cx, Size::new(available.width, BUTTON_HEIGHT)))
            .collect();
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let mut frames = self.frames();
        frames.extend(self.tool_frames());
        for (index, (child, frame)) in self.children.iter_mut().zip(frames).enumerate() {
            if index < Self::TOOLS {
                child.measure(cx, Size::new(frame.width, frame.height));
            }
            child.place(cx, frame);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let sheet = self.sheet();
        let radius = match self.inset > 0.0 {
            true => Radius::concentric(Radius::XL, self.inset),
            false => 0.0,
        };
        let mut content = Vec::new();
        for (index, child) in self.children.iter().enumerate() {
            match index {
                Self::SIDEBAR => child.paint(cx, items),
                index if index >= Self::TOOLS => child.paint(cx, items),
                _ => child.paint(cx, &mut content),
            }
        }
        let list = LIST_WIDTH.min(sheet.width / 2.0);
        content.push(Item::Fill {
            shape: Shape::rect(Rect::new(sheet.x + list, sheet.y, 1.0, sheet.height)),
            paint: cx.palette.hairline.into(),
        });
        items.push(Item::Clip {
            shape: Shape::rounded(sheet, radius),
            items: content,
        });
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::Window)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

/// A contact's photo, or its initials on a tint.
pub struct Avatar {
    id: WidgetId,
    photo: Option<Image>,
    initials: String,
    name: String,
    size: f32,
    text: Option<Paragraph>,
    frame: Rect,
}

pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().find(|character| character.is_alphanumeric()))
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

impl Avatar {
    pub fn new(photo: Option<Image>, name: &str, size: f32) -> Self {
        Self {
            id: WidgetId::next(),
            photo,
            initials: initials(name),
            name: name.to_owned(),
            size,
            text: None,
            frame: Rect::default(),
        }
    }
}

impl Widget for Avatar {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, _available: Size) -> Size {
        Size::new(self.size, self.size)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let style = TextStyle::role(TextRole::Title, cx.palette.accent_text);
        self.text =
            Some(
                cx.text
                    .paragraph(&self.initials, style, Some(self.size), TextAlign::Center),
            );
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let rect = Rect::new(self.frame.x, self.frame.y, self.size, self.size);
        let circle = Shape::rounded(rect, self.size / 2.0);
        match &self.photo {
            Some(photo) => {
                let (width, height) = (photo.width() as f32, photo.height() as f32);
                let scale = self.size / width.min(height).max(1.0);
                let shown = Rect::new(
                    rect.x - (width * scale - self.size) / 2.0,
                    rect.y - (height * scale - self.size) / 2.0,
                    width * scale,
                    height * scale,
                );
                items.push(Item::Clip {
                    shape: circle,
                    items: vec![Item::Image {
                        rect: shown,
                        image: photo.clone(),
                    }],
                });
            }
            None => {
                items.push(Item::Fill {
                    shape: circle,
                    paint: cx.palette.accent_soft.into(),
                });
                if let Some(text) = &self.text {
                    items.push(Item::Text {
                        origin: Point::new(rect.x, rect.y + (self.size - text.height()) / 2.0),
                        paragraph: text.clone(),
                    });
                }
            }
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Image);
        node.set_label(match self.photo.is_some() {
            true => tr!("Photo of {name}", name = self.name.clone()),
            false => tr!("Initials of {name}", name = self.name.clone()),
        });
        node
    }
}

/// The letters of the list: clicking or dragging over one shows the contacts it starts.
pub struct LetterIndex {
    id: WidgetId,
    letters: Vec<String>,
    texts: Vec<Paragraph>,
    on_letter: Box<dyn FnMut(&str)>,
    pressed: bool,
    frame: Rect,
}

impl LetterIndex {
    pub fn new(on_letter: impl FnMut(&str) + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            letters: Vec::new(),
            texts: Vec::new(),
            on_letter: Box::new(on_letter),
            pressed: false,
            frame: Rect::default(),
        }
    }

    pub fn set_letters(&mut self, letters: Vec<String>) {
        self.letters = letters;
    }

    fn step(&self) -> f32 {
        (self.frame.height / self.letters.len().max(1) as f32).min(18.0)
    }

    fn top(&self) -> f32 {
        self.frame.y + (self.frame.height - self.step() * self.letters.len() as f32) / 2.0
    }

    fn letter_frame(&self, index: usize) -> Rect {
        Rect::new(
            self.frame.x,
            self.top() + index as f32 * self.step(),
            self.frame.width,
            self.step(),
        )
    }

    fn pick(&mut self, point: Point) -> bool {
        let index = ((point.y - self.top()) / self.step()).floor();
        if index < 0.0 {
            return false;
        }
        match self.letters.get(index as usize).cloned() {
            Some(letter) => {
                (self.on_letter)(&letter);
                true
            }
            None => false,
        }
    }
}

impl Widget for LetterIndex {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(INDEX_WIDTH, available.height)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let color: Color = cx.palette.accent_text;
        self.texts = self
            .letters
            .iter()
            .map(|letter| {
                cx.text.paragraph(
                    letter,
                    TextStyle::role(TextRole::Label, color),
                    Some(frame.width),
                    TextAlign::Center,
                )
            })
            .collect();
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        for (index, text) in self.texts.iter().enumerate() {
            let frame = self.letter_frame(index);
            items.push(Item::Text {
                origin: Point::new(frame.x, frame.y + (frame.height - text.height()) / 2.0),
                paragraph: text.clone(),
            });
        }
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer)
                if pointer.button == PointerButton::Primary
                    && self.frame.contains(pointer.position) =>
            {
                self.pressed = true;
                self.pick(pointer.position)
            }
            Event::PointerMove(pointer) if self.pressed => self.pick(pointer.position),
            Event::PointerUp(_) if self.pressed => {
                self.pressed = false;
                true
            }
            _ => false,
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Toolbar);
        node.set_label(tr!("Index"));
        node
    }

    fn parts(&self, _cx: &Context) -> Parts {
        let nodes: Vec<_> = self
            .letters
            .iter()
            .enumerate()
            .map(|(index, letter)| {
                let mut node = Node::new(Role::Button);
                node.set_label(tr!(
                    "Contacts starting with {letter}",
                    letter = letter.clone()
                ));
                node.set_bounds(access_rect(self.letter_frame(index)));
                node.add_action(Action::Click);
                (self.id.part(index as u64), node)
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
        match self.letters.get(part as usize).cloned() {
            Some(letter) => {
                (self.on_letter)(&letter);
                true
            }
            None => false,
        }
    }
}

/// A fixed-width control beside one that takes the rest of the line, such as an address's
/// label and the address.
pub struct Pair {
    id: WidgetId,
    children: [Box<dyn Widget>; 2],
    width: f32,
    frame: Rect,
}

impl Pair {
    pub fn new(left: Box<dyn Widget>, width: f32, right: Box<dyn Widget>) -> Self {
        Self {
            id: WidgetId::next(),
            children: [left, right],
            width,
            frame: Rect::default(),
        }
    }
}

const PAIR_GAP: f32 = 8.0;

impl Widget for Pair {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let left = self.children[0].measure(cx, Size::new(self.width, available.height));
        let right = self.children[1].measure(
            cx,
            Size::new(
                (available.width - self.width - PAIR_GAP).max(0.0),
                available.height,
            ),
        );
        Size::new(available.width, left.height.max(right.height))
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let left = Rect::new(frame.x, frame.y, self.width, frame.height);
        let right = Rect::new(
            frame.x + self.width + PAIR_GAP,
            frame.y,
            (frame.width - self.width - PAIR_GAP).max(0.0),
            frame.height,
        );
        self.children[0].measure(cx, Size::new(left.width, left.height));
        self.children[0].place(cx, left);
        self.children[1].measure(cx, Size::new(right.width, right.height));
        self.children[1].place(cx, right);
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
        Node::new(Role::GenericContainer)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}
