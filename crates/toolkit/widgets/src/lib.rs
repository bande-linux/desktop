mod animation;
mod button;
mod context_menu;
mod disclosure;
mod editor;
mod free_layout;
mod label;
mod menu;
mod scroll;
mod secure_field;
mod sheet;
mod slider;
mod stack;
mod table;
mod text_field;
mod text_view;
mod toggle;

use std::any::Any;
use std::cell;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};

pub use animation::Animated;
pub use button::{BUTTON_HEIGHT, Button};
pub use context_menu::ContextMenu;
pub use disclosure::{DISCLOSURE_HEIGHT, Disclosure};
pub use free_layout::FreeLayout;
pub use label::Label;
pub use menu::{Menu, MenuItem, MenuOutcome, MenuPanel, MenuTracker};
pub use scroll::{ScrollState, ScrollView};
pub use secure_field::SecureField;
pub use sheet::Alert;
use sheet::Sheet;
pub use slider::{SLIDER_HEIGHT, Slider};
pub use stack::{Axis, Stack};
pub use table::{Cell, Column, HEADER_HEIGHT, ROW_HEIGHT, Row, Sort, Table};
pub use text_field::{TEXT_FIELD_HEIGHT, TextField};
pub use text_view::TextView;
pub use toggle::{SWITCH_SIZE, Toggle};
pub use unbound_spring::Spring;

use unbound_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-widgets.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

use unbound_graphics::{Appearance, Item, Palette, Point, Rect, Scene, Shape, TextSystem};
pub use unbound_graphics::{
    Cursor, Event, Key, Modifiers, Pointer, PointerButton, Scroll, ScrollPhase, ScrollSource,
};

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

    pub fn part(self, index: u64) -> NodeId {
        NodeId((self.0 << PART_SHIFT) | (index + 1))
    }

    fn owner_of_part(node: NodeId) -> Option<(Self, u64)> {
        let owner = node.0 >> PART_SHIFT;
        (owner != 0).then(|| (Self(owner), (node.0 & PART_MASK) - 1))
    }
}

const PART_SHIFT: u32 = 32;
const PART_MASK: u64 = (1 << PART_SHIFT) - 1;

#[derive(Debug, Default)]
pub struct Parts {
    pub children: Vec<NodeId>,
    pub nodes: Vec<(NodeId, Node)>,
    pub focus: Option<NodeId>,
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
    pub reduce_motion: bool,
    focus: Option<WidgetId>,
    now: Instant,
    animating: cell::Cell<bool>,
    clipboard: Option<ClipboardRequest>,
    input_method_reset: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardRequest {
    Copy(String),
    Paste,
}

impl Context {
    pub fn new(text: TextSystem, palette: Palette) -> Self {
        Self {
            text,
            palette,
            keyboard_navigation: false,
            reduce_motion: false,
            focus: None,
            now: Instant::now(),
            animating: cell::Cell::new(false),
            clipboard: None,
            input_method_reset: false,
        }
    }

    pub fn copy(&mut self, text: &str) {
        self.clipboard = Some(ClipboardRequest::Copy(text.to_owned()));
    }

    pub fn now(&self) -> Instant {
        self.now
    }

    pub fn is_focused(&self, id: WidgetId) -> bool {
        self.focus == Some(id)
    }
}

const FOCUS_RING: f32 = 3.0;
const DIALOG_RISE: f32 = 12.0;

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

pub trait AsAny {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

pub trait Widget: AsAny {
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

    fn cursor(&self, _point: Point) -> Option<Cursor> {
        None
    }

    fn context_menu(&mut self, _cx: &mut Context, _point: Point) -> Option<Menu> {
        None
    }

    fn menu_action(&mut self, _cx: &mut Context, _action: &str) -> bool {
        false
    }

    fn parts(&self, _cx: &Context) -> Parts {
        Parts::default()
    }

    fn perform_part(&mut self, _cx: &mut Context, _part: u64, _request: &ActionRequest) -> bool {
        false
    }

    fn key_equivalent(&self) -> Option<Key> {
        None
    }

    fn dismissed(&self) -> bool {
        false
    }

    fn hit_children(&self, _point: Point) -> bool {
        true
    }

    fn reveal(&mut self, _cx: &mut Context, _rect: Rect) -> bool {
        false
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
    clock: Box<dyn Fn() -> Instant>,
    captured: Option<(WidgetId, PointerButton)>,
    hovered: Vec<WidgetId>,
    pointer: Option<Point>,
    menu_owner: Option<WidgetId>,
    menu_request: Option<(Menu, Point)>,
    awaiting_paste: bool,
    sheet: Option<Sheet>,
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

fn hit_path(widget: &dyn Widget, point: Point, path: &mut Vec<WidgetId>) -> bool {
    if !contains(widget.frame(), point) {
        return false;
    }
    path.push(widget.id());
    if !widget.hit_children(point) {
        return true;
    }
    for child in widget.children().iter().rev() {
        if hit_path(child.as_ref(), point, path) {
            break;
        }
    }
    true
}

fn equivalent(widget: &dyn Widget, key: Key) -> Option<WidgetId> {
    if widget.key_equivalent() == Some(key) {
        return Some(widget.id());
    }
    widget
        .children()
        .iter()
        .find_map(|child| equivalent(child.as_ref(), key))
}

fn lookup_in<'a>(
    root: &'a mut Box<dyn Widget>,
    sheet: &'a mut Option<Sheet>,
    id: WidgetId,
) -> Option<&'a mut dyn Widget> {
    if find(root.as_ref(), id).is_some() {
        return find_mut(root.as_mut(), id);
    }
    find_mut(sheet.as_mut()?.widget.as_mut(), id)
}

fn path_to(widget: &dyn Widget, id: WidgetId, path: &mut Vec<WidgetId>) -> bool {
    path.push(widget.id());
    if widget.id() == id
        || widget
            .children()
            .iter()
            .any(|child| path_to(child.as_ref(), id, path))
    {
        return true;
    }
    path.pop();
    false
}

fn opens_menu(key: Key, modifiers: Modifiers) -> bool {
    match key {
        Key::Menu => true,
        Key::Function(10) => {
            modifiers
                == Modifiers {
                    shift: true,
                    ..Modifiers::default()
                }
        }
        _ => false,
    }
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
    let parts = widget.parts(cx);
    node.set_children(
        widget
            .children()
            .iter()
            .map(|child| child.id().node())
            .chain(parts.children)
            .collect::<Vec<_>>(),
    );
    nodes.push((widget.id().node(), node));
    nodes.extend(parts.nodes);
    for child in widget.children() {
        collect(child.as_ref(), cx, nodes);
    }
}

impl Ui {
    pub fn new(root: Box<dyn Widget>, cx: Context) -> Self {
        Self {
            root,
            cx,
            clock: Box::new(Instant::now),
            captured: None,
            hovered: Vec::new(),
            pointer: None,
            menu_owner: None,
            menu_request: None,
            awaiting_paste: false,
            sheet: None,
        }
    }

    pub fn context(&mut self) -> &mut Context {
        &mut self.cx
    }

    pub fn focus(&self) -> Option<WidgetId> {
        self.cx.focus
    }

    pub fn clear_focus(&mut self) {
        self.cx.focus = None;
    }

    pub fn widget(&mut self, id: WidgetId) -> Option<&mut dyn Widget> {
        self.lookup_mut(id)
    }

    pub fn widget_as<T: Widget + 'static>(&mut self, id: WidgetId) -> Option<&mut T> {
        self.lookup_mut(id)?.as_any_mut().downcast_mut()
    }

    pub fn widget_ref<T: Widget + 'static>(&self, id: WidgetId) -> Option<&T> {
        self.lookup(id)?.as_any().downcast_ref()
    }

    fn lookup(&self, id: WidgetId) -> Option<&dyn Widget> {
        find(self.root.as_ref(), id).or_else(|| {
            self.sheet
                .as_ref()
                .and_then(|sheet| find(sheet.widget.as_ref(), id))
        })
    }

    fn lookup_mut(&mut self, id: WidgetId) -> Option<&mut dyn Widget> {
        lookup_in(&mut self.root, &mut self.sheet, id)
    }

    fn active(&self) -> &dyn Widget {
        match &self.sheet {
            Some(sheet) if !sheet.closing => sheet.widget.as_ref(),
            _ => self.root.as_ref(),
        }
    }

    pub fn present_sheet(&mut self, sheet: impl Widget + 'static) {
        self.present(Box::new(sheet), false);
    }

    pub fn present_dialog(&mut self, dialog: impl Widget + 'static) {
        self.present(Box::new(dialog), true);
    }

    fn present(&mut self, widget: Box<dyn Widget>, floating: bool) {
        self.cx.now = (self.clock)();
        self.captured = None;
        let previous_focus = self.cx.focus.take();
        let mut shown = Animated::new(0.0);
        shown.animate_to(&self.cx, 1.0, Spring::QUICK);
        self.sheet = Some(Sheet {
            widget,
            shown,
            closing: false,
            previous_focus,
            floating,
        });
        let mut order = Vec::new();
        focus_order(self.active(), &self.cx, &mut order);
        let primary = equivalent(self.active(), Key::Enter);
        self.cx.focus = order
            .into_iter()
            .find(|id| {
                self.lookup(*id)
                    .is_some_and(|widget| widget.key_equivalent().is_none())
            })
            .or(primary.filter(|id| self.accepts_focus(*id)));
    }

    pub fn has_sheet(&self) -> bool {
        self.sheet.as_ref().is_some_and(|sheet| !sheet.closing)
    }

    fn dismiss_finished_sheet(&mut self) {
        let Some(sheet) = &mut self.sheet else {
            return;
        };
        if sheet.closing || !sheet.widget.dismissed() {
            return;
        }
        sheet.closing = true;
        sheet.shown.animate_to(&self.cx, 0.0, Spring::QUICK);
        self.cx.focus = sheet.previous_focus;
        self.hovered.clear();
        self.captured = None;
    }

    pub fn layout(&mut self, size: Size) {
        self.root.measure(&mut self.cx, size);
        self.root
            .place(&mut self.cx, Rect::new(0.0, 0.0, size.width, size.height));
        if let Some(sheet) = &mut self.sheet {
            let measured = sheet.widget.measure(&mut self.cx, size);
            let shown = sheet.shown.value(&self.cx);
            let y = match sheet.floating {
                true => {
                    ((size.height - measured.height) / 3.0).round() - DIALOG_RISE * (1.0 - shown)
                }
                false => -measured.height * (1.0 - shown),
            };
            sheet.widget.place(
                &mut self.cx,
                Rect::new(
                    ((size.width - measured.width) / 2.0).round(),
                    y,
                    measured.width,
                    measured.height,
                ),
            );
        }
    }

    pub fn set_clock(&mut self, clock: impl Fn() -> Instant + 'static) {
        self.clock = Box::new(clock);
        self.cx.now = (self.clock)();
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.cx.palette = appearance.palette();
        self.cx.reduce_motion = appearance.reduce_motion;
    }

    pub fn is_animating(&self) -> bool {
        self.cx.animating.get()
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        let (mut items, sheet) = self.paint_layers(width, height);
        items.extend(sheet);
        Scene {
            width,
            height,
            background: self.cx.palette.surface_background,
            items,
        }
    }

    pub fn paint_layers(&mut self, width: u32, height: u32) -> (Vec<Item>, Vec<Item>) {
        self.cx.now = (self.clock)();
        self.cx.animating.set(false);
        self.layout(Size::new(width as f32, height as f32));
        let mut items = Vec::new();
        self.root.paint(&self.cx, &mut items);
        let mut above = Vec::new();
        if let Some(sheet) = &self.sheet {
            let shown = sheet.shown.value(&self.cx);
            if sheet.closing && shown <= 0.001 {
                self.sheet = None;
            } else {
                sheet.paint(
                    &self.cx,
                    shown,
                    Size::new(width as f32, height as f32),
                    &mut above,
                );
            }
        }
        (items, above)
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        self.cx.now = (self.clock)();
        match event {
            Event::PointerDown(pointer) => {
                let hovered = self.hover(Some(pointer));
                let Some(&target) = self.hovered.last() else {
                    return hovered;
                };
                if self.captured.is_none() {
                    self.captured = Some((target, pointer.button));
                }
                if pointer.is_primary() && self.accepts_focus(target) {
                    self.cx.focus = Some(target);
                }
                let delivered = self.deliver(target, event);
                if pointer.button == PointerButton::Secondary {
                    self.captured = None;
                    return self.request_menu(pointer.position) | delivered | hovered;
                }
                delivered | hovered
            }
            Event::PointerMove(pointer) | Event::PointerEnter(pointer) => {
                let moved = Event::PointerMove(*pointer);
                match self.captured {
                    Some((target, _)) => {
                        self.pointer = Some(pointer.position);
                        self.deliver(target, &moved)
                    }
                    None => {
                        let hovered = self.hover(Some(pointer));
                        let delivered = self
                            .hovered
                            .last()
                            .copied()
                            .is_some_and(|target| self.deliver(target, &moved));
                        hovered | delivered
                    }
                }
            }
            Event::PointerUp(pointer) => {
                let Some((target, button)) = self.captured else {
                    return false;
                };
                if button != pointer.button {
                    return false;
                }
                self.captured = None;
                let delivered = self.deliver(target, event);
                delivered | self.hover(Some(pointer))
            }
            Event::PointerLeave => {
                self.captured = None;
                self.hover(None)
            }
            Event::Scroll(scroll) => {
                let pointer = Pointer::at(scroll.position).with_modifiers(scroll.modifiers);
                let hovered = self.hover(Some(&pointer));
                let path = self.hovered.clone();
                path.iter().rev().any(|target| self.deliver(*target, event)) | hovered
            }
            Event::ModifiersChanged(_) => {
                let mut targets = self.hovered.clone();
                targets.extend(self.cx.focus.filter(|focus| !targets.contains(focus)));
                targets.into_iter().fold(false, |handled, target| {
                    self.deliver(target, event) | handled
                })
            }
            Event::Key {
                key: Key::Tab,
                modifiers,
            } if !modifiers.logo && !modifiers.control && !modifiers.alt => {
                self.move_focus(!modifiers.shift);
                true
            }
            Event::Key { key, modifiers } if opens_menu(*key, *modifiers) => {
                self.request_keyboard_menu()
            }
            Event::Key { key, modifiers } => {
                let delivered = self
                    .cx
                    .focus
                    .is_some_and(|target| self.deliver(target, event));
                delivered || (*modifiers == Modifiers::default() && self.press_equivalent(*key))
            }
            Event::Commit(_) | Event::Preedit { .. } => self
                .cx
                .focus
                .is_some_and(|target| self.deliver(target, event)),
        }
    }

    fn press_equivalent(&mut self, key: Key) -> bool {
        let Some(target) = equivalent(self.active(), key) else {
            return false;
        };
        self.perform(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: target.node(),
            data: None,
        })
    }

    fn request_keyboard_menu(&mut self) -> bool {
        let mut path = Vec::new();
        let active = self.active();
        let target = self.cx.focus.unwrap_or_else(|| active.id());
        if !path_to(active, target, &mut path) {
            path = vec![active.id()];
        }
        let frame = find(active, target).map_or_else(|| active.frame(), |widget| widget.frame());
        let point = Point::new(frame.x, frame.y + frame.height);
        self.request_menu_along(path, point)
    }

    fn request_menu(&mut self, point: Point) -> bool {
        self.request_menu_along(self.hovered.clone(), point)
    }

    fn request_menu_along(&mut self, path: Vec<WidgetId>, point: Point) -> bool {
        for id in path.into_iter().rev() {
            let Some(widget) = lookup_in(&mut self.root, &mut self.sheet, id) else {
                continue;
            };
            if let Some(menu) = widget.context_menu(&mut self.cx, point) {
                self.menu_owner = Some(id);
                self.menu_request = Some((menu, point));
                return true;
            }
        }
        false
    }

    pub fn take_clipboard_request(&mut self) -> Option<ClipboardRequest> {
        let request = self.cx.clipboard.take();
        if request == Some(ClipboardRequest::Paste) {
            self.awaiting_paste = true;
        }
        request
    }

    pub fn pasted(&mut self, text: &str) -> bool {
        if !std::mem::take(&mut self.awaiting_paste) {
            return false;
        }
        self.handle(&Event::Commit(text.to_owned()));
        true
    }

    pub fn take_input_method_reset(&mut self) -> bool {
        std::mem::take(&mut self.cx.input_method_reset)
    }

    pub fn take_menu_request(&mut self) -> Option<(Menu, Point)> {
        self.menu_request.take()
    }

    pub fn menu_closed(&mut self) -> bool {
        self.menu_owner.take().is_some()
    }

    pub fn menu_action(&mut self, action: &str) -> bool {
        self.cx.now = (self.clock)();
        let Some(owner) = self.menu_owner.take() else {
            return false;
        };
        let Some(widget) = lookup_in(&mut self.root, &mut self.sheet, owner) else {
            return false;
        };
        widget.menu_action(&mut self.cx, action)
    }

    pub fn cursor(&self) -> Cursor {
        let Some(point) = self.pointer else {
            return Cursor::Default;
        };
        let path = match self.captured {
            Some((target, _)) => vec![target],
            None => self.hovered.clone(),
        };
        path.iter()
            .rev()
            .filter_map(|id| self.lookup(*id))
            .find_map(|widget| widget.cursor(point))
            .unwrap_or_default()
    }

    fn hover(&mut self, pointer: Option<&Pointer>) -> bool {
        self.pointer = pointer.map(|pointer| pointer.position);
        let mut path = Vec::new();
        if let Some(pointer) = pointer {
            hit_path(self.active(), pointer.position, &mut path);
        }
        let left = self
            .hovered
            .iter()
            .filter(|id| !path.contains(id))
            .copied()
            .collect::<Vec<_>>();
        let entered = path
            .iter()
            .filter(|id| !self.hovered.contains(id))
            .copied()
            .collect::<Vec<_>>();
        self.hovered = path;
        let mut changed = false;
        for id in left.into_iter().rev() {
            changed |= self.deliver(id, &Event::PointerLeave);
        }
        if let Some(pointer) = pointer {
            for id in entered {
                changed |= self.deliver(id, &Event::PointerEnter(*pointer));
            }
        }
        changed
    }

    pub fn text_input_area(&self) -> Option<Rect> {
        self.lookup(self.cx.focus?)?.text_input_area()
    }

    fn accepts_focus(&self, target: WidgetId) -> bool {
        self.lookup(target)
            .is_some_and(|widget| widget.accepts_focus(&self.cx))
    }

    fn deliver(&mut self, target: WidgetId, event: &Event) -> bool {
        let focus_path = self.focus_path();
        let Some(widget) = lookup_in(&mut self.root, &mut self.sheet, target) else {
            return false;
        };
        let handled = widget.handle(&mut self.cx, event);
        self.dismiss_finished_sheet();
        self.keep_focus_reachable(focus_path);
        handled
    }

    fn focus_path(&self) -> Vec<WidgetId> {
        let mut path = Vec::new();
        if let Some(focus) = self.cx.focus {
            path_to(self.active(), focus, &mut path);
        }
        path
    }

    fn keep_focus_reachable(&mut self, before: Vec<WidgetId>) {
        let Some(focus) = self.cx.focus else {
            return;
        };
        if before.last() != Some(&focus) || self.lookup(focus).is_some() {
            return;
        }
        self.cx.focus = before
            .into_iter()
            .rev()
            .find(|id| self.lookup(*id).is_some() && self.accepts_focus(*id));
    }

    fn move_focus(&mut self, forward: bool) {
        let mut order = Vec::new();
        focus_order(self.active(), &self.cx, &mut order);
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
        self.reveal_focus();
    }

    fn reveal_focus(&mut self) {
        let Some(focus) = self.cx.focus else {
            return;
        };
        let mut path = Vec::new();
        if !path_to(self.active(), focus, &mut path) {
            return;
        }
        let Some(rect) = self.lookup(focus).map(|widget| widget.frame()) else {
            return;
        };
        for id in path.into_iter().rev().skip(1) {
            if let Some(widget) = lookup_in(&mut self.root, &mut self.sheet, id) {
                widget.reveal(&mut self.cx, rect);
            }
        }
    }

    pub fn accessibility(&self) -> TreeUpdate {
        let mut nodes = Vec::new();
        collect(self.root.as_ref(), &self.cx, &mut nodes);
        if let Some((_, root)) = nodes.first_mut()
            && root.role() == Role::Unknown
        {
            root.set_role(Role::Window);
        }
        if let Some(sheet) = self.sheet.as_ref().filter(|sheet| !sheet.closing) {
            if let Some((_, root)) = nodes.first_mut() {
                root.push_child(sheet.widget.id().node());
            }
            collect(sheet.widget.as_ref(), &self.cx, &mut nodes);
        }
        TreeUpdate {
            nodes,
            tree: Some(TreeInfo {
                toolkit_name: Some("unbound-toolkit".to_owned()),
                toolkit_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
                ..TreeInfo::new(self.root.id().node())
            }),
            tree_id: TreeId::ROOT,
            focus: self.focus_node(),
        }
    }

    fn focus_node(&self) -> NodeId {
        let focus = self.cx.focus.unwrap_or(self.root.id());
        self.lookup(focus)
            .and_then(|widget| widget.parts(&self.cx).focus)
            .unwrap_or(focus.node())
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        self.cx.now = (self.clock)();
        if let Some((owner, part)) = WidgetId::owner_of_part(request.target_node) {
            if find(self.active(), owner).is_none() {
                return false;
            }
            if matches!(request.action, Action::Focus | Action::Click) && self.accepts_focus(owner)
            {
                self.cx.focus = Some(owner);
            }
            let Some(widget) = lookup_in(&mut self.root, &mut self.sheet, owner) else {
                return false;
            };
            return widget.perform_part(&mut self.cx, part, request);
        }
        let target = WidgetId(request.target_node.0);
        if find(self.active(), target).is_none() {
            return false;
        }
        if request.action == Action::Focus {
            let accepts = self.accepts_focus(target);
            if accepts {
                self.cx.focus = Some(target);
            }
            return accepts;
        }
        let focus_path = self.focus_path();
        let Some(widget) = lookup_in(&mut self.root, &mut self.sheet, target) else {
            return false;
        };
        let performed = widget.perform(&mut self.cx, request);
        self.dismiss_finished_sheet();
        self.keep_focus_reachable(focus_path);
        performed
    }
}
