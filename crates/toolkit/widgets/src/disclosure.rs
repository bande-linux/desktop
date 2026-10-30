use accesskit::{Action, ActionRequest, Node, Role};
use unbound_graphics::{Color, Item, Paragraph, Point, Rect, Shape, TextAlign, TextStyle};
use unbound_spring::Spring;

use super::{Animated, Context, Event, Key, Size, Widget, WidgetId, contains};

pub const DISCLOSURE_HEIGHT: f32 = 22.0;
const CHEVRON_SPACE: f32 = 18.0;
const GAP: f32 = 6.0;

pub struct Disclosure {
    id: WidgetId,
    title: String,
    expanded: bool,
    turn: Animated,
    content: [Box<dyn Widget>; 1],
    content_size: Size,
    on_toggle: Box<dyn FnMut(bool)>,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl Disclosure {
    pub fn new(
        title: &str,
        expanded: bool,
        content: impl Widget + 'static,
        on_toggle: impl FnMut(bool) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            title: title.to_owned(),
            expanded,
            turn: Animated::new(if expanded { 1.0 } else { 0.0 }),
            content: [Box::new(content)],
            content_size: Size::default(),
            on_toggle: Box::new(on_toggle),
            paragraph: None,
            frame: Rect::default(),
        }
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    fn set_expanded(&mut self, cx: &Context, expanded: bool) -> bool {
        if self.expanded == expanded {
            return false;
        }
        self.expanded = expanded;
        self.turn
            .animate_to(cx, if expanded { 1.0 } else { 0.0 }, Spring::QUICK);
        (self.on_toggle)(expanded);
        true
    }

    fn header(&self) -> Rect {
        Rect::new(
            self.frame.x,
            self.frame.y,
            self.frame.width,
            DISCLOSURE_HEIGHT,
        )
    }
}

pub(crate) fn chevron(centre: Point, turn: f32, color: Color, items: &mut Vec<Item>) {
    let angle = turn * std::f32::consts::FRAC_PI_2;
    let rotate = |x: f32, y: f32| {
        Point::new(
            centre.x + x * angle.cos() - y * angle.sin(),
            centre.y + x * angle.sin() + y * angle.cos(),
        )
    };
    for (from, to) in [((-2.0, -4.0), (2.0, 0.0)), ((2.0, 0.0), (-2.0, 4.0))] {
        items.push(Item::Line {
            from: rotate(from.0, from.1),
            to: rotate(to.0, to.1),
            width: 1.5,
            paint: color.into(),
        });
    }
}

impl Widget for Disclosure {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.paragraph = Some(cx.text.paragraph(
            &self.title,
            TextStyle::new(13.0, cx.palette.ink).weight(TextStyle::MEDIUM),
            None,
            TextAlign::Start,
        ));
        let below = Size::new(
            available.width,
            (available.height - DISCLOSURE_HEIGHT - GAP).max(0.0),
        );
        self.content_size = self.content[0].measure(cx, below);
        let shown = self.turn.value(cx);
        let content = (self.content_size.height + GAP) * shown;
        Size::new(available.width, DISCLOSURE_HEIGHT + content.ceil())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let content = Rect::new(
            frame.x,
            frame.y + DISCLOSURE_HEIGHT + GAP,
            frame.width,
            self.content_size.height,
        );
        let hidden = Rect::new(frame.x, frame.y + DISCLOSURE_HEIGHT, 0.0, 0.0);
        let shown = self.expanded || self.turn.value(cx) > 0.0;
        self.content[0].place(cx, if shown { content } else { hidden });
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let header = self.header();
        let turn = self.turn.value(cx);
        let centre = Point::new(header.x + 7.0, header.y + header.height / 2.0);
        chevron(centre, turn, cx.palette.ink2, items);
        if let Some(paragraph) = &self.paragraph {
            items.push(Item::Text {
                origin: Point::new(
                    header.x + CHEVRON_SPACE,
                    header.y + ((header.height - paragraph.height()) / 2.0).round(),
                ),
                paragraph: paragraph.clone(),
            });
        }
        if cx.is_focused(self.id) {
            super::focus_ring(cx, Shape::rounded(header, 4.0), items);
        }
        if turn > 0.0 {
            let mut content = Vec::new();
            self.content[0].paint(cx, &mut content);
            let reveal = Rect::new(
                self.frame.x,
                header.y + header.height,
                self.frame.width,
                (self.frame.height - header.height).max(0.0),
            );
            items.push(Item::Clip {
                shape: Shape::rect(reveal),
                items: vec![Item::Opacity {
                    alpha: turn.clamp(0.0, 1.0),
                    items: content,
                }],
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::DisclosureTriangle);
        node.set_label(self.title.clone());
        node.set_expanded(self.expanded);
        node.add_action(Action::Click);
        node.add_action(if self.expanded {
            Action::Collapse
        } else {
            Action::Expand
        });
        node
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer)
                if pointer.is_primary() && contains(self.header(), pointer.position) =>
            {
                let expanded = !self.expanded;
                self.set_expanded(cx, expanded)
            }
            Event::Key { key, .. } => match key {
                Key::Space => {
                    let expanded = !self.expanded;
                    self.set_expanded(cx, expanded)
                }
                Key::Right => self.set_expanded(cx, true),
                Key::Left => self.set_expanded(cx, false),
                _ => false,
            },
            _ => false,
        }
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        match request.action {
            Action::Click => {
                let expanded = !self.expanded;
                self.set_expanded(cx, expanded)
            }
            Action::Expand => self.set_expanded(cx, true),
            Action::Collapse => self.set_expanded(cx, false),
            _ => false,
        }
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }

    fn hit_children(&self, point: Point) -> bool {
        self.expanded && !contains(self.header(), point)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        if self.expanded { &self.content } else { &[] }
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        if self.expanded {
            &mut self.content
        } else {
            &mut []
        }
    }
}
