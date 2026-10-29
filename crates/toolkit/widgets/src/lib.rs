mod button;
mod label;
mod secure_field;
mod slider;
mod stack;
mod text_field;
mod toggle;

use std::sync::atomic::{AtomicU64, Ordering};

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};

pub use button::{BUTTON_HEIGHT, Button};
pub use label::Label;
pub use secure_field::SecureField;
pub use slider::{SLIDER_HEIGHT, Slider};
pub use stack::{Axis, Stack};
pub use text_field::{TEXT_FIELD_HEIGHT, TextField};
pub use toggle::{SWITCH_SIZE, Toggle};

pub use unbound_graphics::{Event, Key, Modifiers};
use unbound_graphics::{Item, Palette, Point, Rect, Scene, Shape, TextSystem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WidgetId(u64);

impl WidgetId {
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    fn node(self) -> NodeId {
        NodeId(self.0)
    }
}

impl From<WidgetId> for NodeId {
    fn from(id: WidgetId) -> Self {
        id.node()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

pub struct Context {
    pub text: TextSystem,
    pub palette: Palette,
    pub keyboard_navigation: bool,
    focus: Option<WidgetId>,
}

impl Context {
    pub fn new(text: TextSystem, palette: Palette) -> Self {
        Self {
            text,
            palette,
            keyboard_navigation: false,
            focus: None,
        }
    }

    pub fn is_focused(&self, id: WidgetId) -> bool {
        self.focus == Some(id)
    }
}

const FOCUS_RING: f32 = 3.0;

fn focus_ring(cx: &Context, shape: Shape, items: &mut Vec<Item>) {
    items.push(Item::Stroke {
        shape: Shape::rounded(
            shape.rect.inset(-FOCUS_RING / 2.0),
            shape.radius + FOCUS_RING / 2.0,
        ),
        width: FOCUS_RING,
        paint: cx.palette.accent_color.with_alpha(128).into(),
    });
}

pub trait Widget {
    fn id(&self) -> WidgetId;
    fn measure(&mut self, cx: &mut Context, available: Size) -> Size;
    fn place(&mut self, cx: &mut Context, frame: Rect);
    fn frame(&self) -> Rect;
    fn paint(&self, cx: &Context, items: &mut Vec<Item>);
    fn access(&self, cx: &Context) -> Node;

    fn handle(&mut self, _cx: &mut Context, _event: &Event) -> bool {
        false
    }

    fn perform(&mut self, _cx: &mut Context, _request: &ActionRequest) -> bool {
        false
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        false
    }

    fn text_input_area(&self) -> Option<Rect> {
        None
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut []
    }
}

pub struct Ui {
    root: Box<dyn Widget>,
    cx: Context,
    captured: Option<WidgetId>,
}

fn contains(frame: Rect, point: Point) -> bool {
    point.x >= frame.x
        && point.y >= frame.y
        && point.x < frame.x + frame.width
        && point.y < frame.y + frame.height
}

fn find_mut(widget: &mut dyn Widget, id: WidgetId) -> Option<&mut dyn Widget> {
    if widget.id() == id {
        return Some(widget);
    }
    widget
        .children_mut()
        .iter_mut()
        .find_map(|child| find_mut(child.as_mut(), id))
}

fn find(widget: &dyn Widget, id: WidgetId) -> Option<&dyn Widget> {
    if widget.id() == id {
        return Some(widget);
    }
    widget
        .children()
        .iter()
        .find_map(|child| find(child.as_ref(), id))
}

fn hit(widget: &dyn Widget, point: Point) -> Option<WidgetId> {
    if !contains(widget.frame(), point) {
        return None;
    }
    widget
        .children()
        .iter()
        .rev()
        .find_map(|child| hit(child.as_ref(), point))
        .or(Some(widget.id()))
}

fn focus_order(widget: &dyn Widget, cx: &Context, order: &mut Vec<WidgetId>) {
    if widget.accepts_focus(cx) {
        order.push(widget.id());
    }
    for child in widget.children() {
        focus_order(child.as_ref(), cx, order);
    }
}

fn collect(widget: &dyn Widget, cx: &Context, nodes: &mut Vec<(NodeId, Node)>) {
    let mut node = widget.access(cx);
    let frame = widget.frame();
    node.set_bounds(accesskit::Rect {
        x0: f64::from(frame.x),
        y0: f64::from(frame.y),
        x1: f64::from(frame.x + frame.width),
        y1: f64::from(frame.y + frame.height),
    });
    if widget.accepts_focus(cx) {
        node.add_action(Action::Focus);
    }
    node.set_children(
        widget
            .children()
            .iter()
            .map(|child| child.id().node())
            .collect::<Vec<_>>(),
    );
    nodes.push((widget.id().node(), node));
    for child in widget.children() {
        collect(child.as_ref(), cx, nodes);
    }
}

impl Ui {
    pub fn new(root: Box<dyn Widget>, cx: Context) -> Self {
        Self {
            root,
            cx,
            captured: None,
        }
    }

    pub fn context(&mut self) -> &mut Context {
        &mut self.cx
    }

    pub fn focus(&self) -> Option<WidgetId> {
        self.cx.focus
    }

    pub fn widget(&mut self, id: WidgetId) -> Option<&mut dyn Widget> {
        find_mut(self.root.as_mut(), id)
    }

    pub fn layout(&mut self, size: Size) {
        self.root.measure(&mut self.cx, size);
        self.root
            .place(&mut self.cx, Rect::new(0.0, 0.0, size.width, size.height));
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.layout(Size::new(width as f32, height as f32));
        let mut items = Vec::new();
        self.root.paint(&self.cx, &mut items);
        Scene {
            width,
            height,
            background: self.cx.palette.surface_background,
            items,
        }
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        match event {
            Event::PointerDown(point) => {
                let Some(target) = hit(self.root.as_ref(), *point) else {
                    return false;
                };
                self.captured = Some(target);
                let accepts = self.accepts_focus(target);
                if accepts {
                    self.cx.focus = Some(target);
                }
                self.deliver(target, event)
            }
            Event::PointerMove(point) => match self.captured {
                Some(target) => self.deliver(target, event),
                None => hit(self.root.as_ref(), *point)
                    .is_some_and(|target| self.deliver(target, event)),
            },
            Event::PointerUp(_) => self
                .captured
                .take()
                .is_some_and(|target| self.deliver(target, event)),
            Event::Key {
                key: Key::Tab,
                modifiers,
            } if !modifiers.logo && !modifiers.control && !modifiers.alt => {
                self.move_focus(!modifiers.shift);
                true
            }
            Event::Key { .. } | Event::Commit(_) | Event::Preedit { .. } => self
                .cx
                .focus
                .is_some_and(|target| self.deliver(target, event)),
        }
    }

    pub fn text_input_area(&self) -> Option<Rect> {
        find(self.root.as_ref(), self.cx.focus?)?.text_input_area()
    }

    fn accepts_focus(&self, target: WidgetId) -> bool {
        find(self.root.as_ref(), target).is_some_and(|widget| widget.accepts_focus(&self.cx))
    }

    fn deliver(&mut self, target: WidgetId, event: &Event) -> bool {
        let Some(widget) = find_mut(self.root.as_mut(), target) else {
            return false;
        };
        widget.handle(&mut self.cx, event)
    }

    fn move_focus(&mut self, forward: bool) {
        let mut order = Vec::new();
        focus_order(self.root.as_ref(), &self.cx, &mut order);
        if order.is_empty() {
            self.cx.focus = None;
            return;
        }
        let current = self
            .cx
            .focus
            .and_then(|focus| order.iter().position(|id| *id == focus));
        let next = match (current, forward) {
            (None, true) => 0,
            (None, false) => order.len() - 1,
            (Some(index), true) => (index + 1) % order.len(),
            (Some(index), false) => (index + order.len() - 1) % order.len(),
        };
        self.cx.focus = Some(order[next]);
    }

    pub fn accessibility(&self) -> TreeUpdate {
        let mut nodes = Vec::new();
        collect(self.root.as_ref(), &self.cx, &mut nodes);
        if let Some((_, root)) = nodes.first_mut()
            && root.role() == Role::Unknown
        {
            root.set_role(Role::Window);
        }
        TreeUpdate {
            nodes,
            tree: Some(TreeInfo {
                toolkit_name: Some("unbound-toolkit".to_owned()),
                toolkit_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
                ..TreeInfo::new(self.root.id().node())
            }),
            tree_id: TreeId::ROOT,
            focus: self.cx.focus.unwrap_or(self.root.id()).node(),
        }
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        let target = WidgetId(request.target_node.0);
        if request.action == Action::Focus {
            let accepts = self.accepts_focus(target);
            if accepts {
                self.cx.focus = Some(target);
            }
            return accepts;
        }
        let Some(widget) = find_mut(self.root.as_mut(), target) else {
            return false;
        };
        widget.perform(&mut self.cx, request)
    }
}
