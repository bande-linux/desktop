use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Node, Role};
use bande_toolkit::ui::{Context, Event, Key, Size, Widget, WidgetId, focus_ring};
use bande_toolkit::{
    Image, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle,
};

const PEER_AVATAR: f32 = 48.0;
const PEER_RADIUS: f32 = 12.0;
const PEER_WIDTH: f32 = 96.0;
const PEER_GAP: f32 = 8.0;
const PEER_NAME_GAP: f32 = 6.0;
const PEER_INITIAL: f32 = 20.0;
const SOURCE_PADDING: f32 = 6.0;

#[derive(Debug, Clone)]
pub struct Peer {
    pub name: String,
    pub initial: String,
    pub picture: Option<Image>,
}

fn activates(event: &Event) -> bool {
    matches!(
        event,
        Event::Key {
            key: Key::Enter | Key::Space,
            ..
        }
    )
}

struct PeerButton {
    id: WidgetId,
    index: usize,
    peer: Peer,
    selected: bool,
    chosen: Rc<Cell<Option<usize>>>,
    pressed: bool,
    initial: Option<Paragraph>,
    name: Option<Paragraph>,
    frame: Rect,
}

impl PeerButton {
    fn avatar(&self) -> Rect {
        Rect::new(
            self.frame.x + ((self.frame.width - PEER_AVATAR) / 2.0).round(),
            self.frame.y,
            PEER_AVATAR,
            PEER_AVATAR,
        )
    }
}

impl Widget for PeerButton {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        let ink = match self.selected {
            true => cx.palette.ink,
            false => cx.palette.ink2,
        };
        self.initial = Some(cx.text.paragraph(
            &self.peer.initial,
            TextStyle::new(PEER_INITIAL, cx.palette.on_accent).weight(TextStyle::BOLD),
            None,
            TextAlign::Start,
        ));
        let name = cx.text.paragraph(
            &self.peer.name,
            TextStyle::role(TextRole::Callout, ink),
            Some(PEER_WIDTH),
            TextAlign::Center,
        );
        let height = PEER_AVATAR + PEER_NAME_GAP + name.height();
        self.name = Some(name);
        Size::new(PEER_WIDTH, height)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let avatar = self.avatar();
        let shape = Shape::rounded(avatar, PEER_RADIUS);
        match &self.peer.picture {
            Some(picture) => items.push(Item::Clip {
                shape,
                items: vec![Item::Image {
                    rect: avatar,
                    image: picture.clone(),
                }],
            }),
            None => {
                items.push(Item::Fill {
                    shape,
                    paint: cx.palette.accent.into(),
                });
                if let Some(initial) = &self.initial {
                    items.push(Item::Text {
                        origin: Point::new(
                            (avatar.x + (avatar.width - initial.width()) / 2.0).round(),
                            (avatar.y + (avatar.height - initial.height()) / 2.0).round(),
                        ),
                        paragraph: initial.clone(),
                    });
                }
            }
        }
        if self.selected {
            items.push(Item::Stroke {
                shape: Shape::rounded(avatar.inset(-3.0), PEER_RADIUS + 3.0),
                width: 2.0,
                paint: cx.palette.ink.into(),
            });
        }
        if cx.is_focused(self.id) {
            focus_ring(cx, Shape::rounded(self.frame, Radius::S), items);
        }
        if let Some(name) = &self.name {
            items.push(Item::Text {
                origin: Point::new(self.frame.x, avatar.y + PEER_AVATAR + PEER_NAME_GAP),
                paragraph: name.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::ListBoxOption);
        node.set_label(self.peer.name.clone());
        node.set_selected(self.selected);
        node.add_action(Action::Click);
        node.add_action(Action::Focus);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => self.pressed = true,
            Event::PointerUp(pointer) if pointer.is_primary() => {
                if std::mem::take(&mut self.pressed) && self.frame.contains(pointer.position) {
                    self.chosen.set(Some(self.index));
                }
            }
            Event::PointerLeave => self.pressed = false,
            event if activates(event) => self.chosen.set(Some(self.index)),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action != Action::Click {
            return false;
        }
        self.chosen.set(Some(self.index));
        true
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }
}

pub struct UserList {
    id: WidgetId,
    label: String,
    peers: Vec<Box<dyn Widget>>,
    frame: Rect,
}

impl UserList {
    pub fn new(label: String) -> Self {
        Self {
            id: WidgetId::next(),
            label,
            peers: Vec::new(),
            frame: Rect::default(),
        }
    }

    pub fn show(&mut self, peers: Vec<Peer>, selected: usize, chosen: &Rc<Cell<Option<usize>>>) {
        self.peers = peers
            .into_iter()
            .enumerate()
            .map(|(index, peer)| {
                Box::new(PeerButton {
                    id: WidgetId::next(),
                    index,
                    peer,
                    selected: index == selected,
                    chosen: chosen.clone(),
                    pressed: false,
                    initial: None,
                    name: None,
                    frame: Rect::default(),
                }) as Box<dyn Widget>
            })
            .collect();
    }
}

impl Widget for UserList {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let mut height: f32 = 0.0;
        for peer in &mut self.peers {
            height = height.max(peer.measure(cx, available).height);
        }
        let count = self.peers.len() as f32;
        let width = (count * (PEER_WIDTH + PEER_GAP) - PEER_GAP).max(0.0);
        Size::new(width, height)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let mut left = frame.x;
        for peer in &mut self.peers {
            peer.place(cx, Rect::new(left, frame.y, PEER_WIDTH, frame.height));
            left += PEER_WIDTH + PEER_GAP;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for peer in &self.peers {
            peer.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::ListBox);
        node.set_label(self.label.clone());
        if self.peers.is_empty() {
            node.set_hidden();
        }
        node
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.peers
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.peers
    }
}

pub struct SourceButton {
    id: WidgetId,
    name: Option<String>,
    pressed: Rc<Cell<bool>>,
    held: bool,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl SourceButton {
    pub fn new(pressed: Rc<Cell<bool>>) -> Self {
        Self {
            id: WidgetId::next(),
            name: None,
            pressed,
            held: false,
            paragraph: None,
            frame: Rect::default(),
        }
    }

    pub fn set_name(&mut self, name: Option<String>) {
        self.name = name;
    }
}

impl Widget for SourceButton {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        self.paragraph = self.name.as_ref().map(|name| {
            cx.text.paragraph(
                name,
                TextStyle::role(TextRole::Callout, cx.palette.ink),
                None,
                TextAlign::Start,
            )
        });
        self.paragraph.as_ref().map_or(Size::default(), |name| {
            Size::new(
                name.width() + 2.0 * SOURCE_PADDING,
                name.height() + SOURCE_PADDING,
            )
        })
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let Some(paragraph) = &self.paragraph else {
            return;
        };
        if cx.is_focused(self.id) {
            focus_ring(cx, Shape::rounded(self.frame, Radius::XS), items);
        }
        items.push(Item::Text {
            origin: Point::new(
                (self.frame.x + SOURCE_PADDING).round(),
                (self.frame.y + (self.frame.height - paragraph.height()) / 2.0).round(),
            ),
            paragraph: paragraph.clone(),
        });
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Button);
        if let Some(name) = &self.name {
            node.set_label(bande_l10n::tr!("Input source {name}", name = name.as_str()));
            node.add_action(Action::Click);
        } else {
            node.set_hidden();
        }
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        if self.name.is_none() {
            return false;
        }
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => self.held = true,
            Event::PointerUp(pointer) if pointer.is_primary() => {
                if std::mem::take(&mut self.held) && self.frame.contains(pointer.position) {
                    self.pressed.set(true);
                }
            }
            Event::PointerLeave => self.held = false,
            event if activates(event) => self.pressed.set(true),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if self.name.is_none() || request.action != Action::Click {
            return false;
        }
        self.pressed.set(true);
        true
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        self.name.is_some()
    }
}
