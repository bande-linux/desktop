mod button;
mod checkbox;
mod clock;
mod combo_box;
mod context_menu;
mod control;
mod disclosure;
mod editor;
mod form;
mod free_layout;
mod label;
mod menu;
mod pop_up_button;
mod progress;
mod radio_group;
mod scroll;
mod search_field;
mod secure_field;
mod segmented;
mod sheet;
mod shortcut;
mod slider;
mod stack;
mod stepper;
mod table;
mod text_field;
mod text_view;
mod toggle;
mod tooltip;

use std::any::Any;
use std::cell;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};

pub use bande_spring::Spring;
pub use button::{BUTTON_HEIGHT, Button};
pub use checkbox::{CHECKBOX_SIZE, CheckState, Checkbox};
pub use clock::{FrameTime, frame_time, request_wake, set_frame_time, take_wake};
pub use combo_box::ComboBox;
pub use context_menu::ContextMenu;
pub use disclosure::{DISCLOSURE_HEIGHT, Disclosure};
pub use form::{Form, FormLine};
pub use free_layout::FreeLayout;
pub use label::Label;
pub use menu::{
    MENU_RADIUS, MENU_SHADOW_MARGIN, Menu, MenuItem, MenuOutcome, MenuPanel, MenuTracker, Search,
};
pub use pop_up_button::PopUpButton;
pub use progress::{LevelIndicator, PROGRESS_HEIGHT, ProgressBar, SPINNER_SIZE, Spinner};
pub use radio_group::RadioGroup;
pub use scroll::{ScrollState, ScrollView};
pub use search_field::SearchField;
pub use secure_field::SecureField;
pub use segmented::{SEGMENTED_HEIGHT, SegmentedControl};
pub use sheet::Alert;
use sheet::Sheet;
pub use shortcut::{Shortcut, ShortcutText, parse_shortcut, shortcut_label, shortcut_pressed};
pub use slider::{SLIDER_HEIGHT, Slider};
pub use stack::{Axis, Stack};
pub use stepper::{STEPPER_SIZE, Stepper};
pub use table::{Cell, Column, ROW_HEIGHT, Row, Sort, Table, access_rect, group_heading};
pub use text_field::{TEXT_FIELD_HEIGHT, TextField};
pub use text_view::TextView;
pub use toggle::{SWITCH_SIZE, Toggle};
pub use tooltip::TOOLTIP_DELAY;
use tooltip::Tooltips;

pub type Animated = bande_spring::Animated<Instant>;

use bande_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/bande-widgets.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

use bande_graphics::{
    Appearance, Color, ControlSize, Item, Palette, Point, Rect, Scene, TextAlign, TextStyle,
    TextSystem, grown_line,
};
pub use bande_graphics::{
    Cursor, Event, Key, Modifiers, Pointer, PointerButton, Scroll, ScrollPhase, ScrollSource,
};
pub use control::{focus_ring, focus_ring_for};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragAction {
    Copy,
    Move,
}

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
    pub window_active: bool,
    pub reduce_motion: bool,
    pub caps_lock: bool,
    focus: Option<WidgetId>,
    now: Instant,
    start: Instant,
    animating: cell::Cell<bool>,
    wake: cell::Cell<Option<Instant>>,
    clipboard: Option<ClipboardRequest>,
    input_method_reset: bool,
    emoji_picker: bool,
    menu: Option<(Menu, Point)>,
    drag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardRequest {
    Copy(String),
    Paste,
    SelectPrimary(String),
    PastePrimary,
}

impl Context {
    pub fn new(text: TextSystem, palette: Palette) -> Self {
        Self {
            text,
            palette,
            keyboard_navigation: false,
            window_active: true,
            reduce_motion: false,
            caps_lock: false,
            focus: None,
            now: Instant::now(),
            start: Instant::now(),
            animating: cell::Cell::new(false),
            wake: cell::Cell::new(None),
            clipboard: None,
            input_method_reset: false,
            emoji_picker: false,
            menu: None,
            drag: None,
        }
    }

    pub fn copy(&mut self, text: &str) {
        self.clipboard = Some(ClipboardRequest::Copy(text.to_owned()));
    }

    pub fn start_text_drag(&mut self, text: String) {
        self.drag = Some(text);
    }

    pub fn show_emoji_picker(&mut self) {
        self.emoji_picker = true;
    }

    pub fn show_menu(&mut self, menu: Menu, at: Point) {
        self.menu = Some((menu, at));
    }

    pub fn now(&self) -> Instant {
        self.now
    }

    pub fn animate(&self, animated: &mut Animated, target: f32, spring: Spring) {
        if animated.animate_at(self.start, self.reduce_motion, target, spring) {
            self.animating.set(true);
        }
    }

    pub fn sample(&self, animated: &Animated) -> f32 {
        let (value, moving) = animated.value_at(self.now);
        if moving {
            self.animating.set(true);
        }
        value
    }

    pub fn wake_at(&self, at: Instant) {
        let wake = self.wake.get().map_or(at, |wake| wake.min(at));
        self.wake.set(Some(wake));
    }

    pub fn is_focused(&self, id: WidgetId) -> bool {
        self.focus == Some(id)
    }

    pub fn control_height(&self, size: ControlSize) -> f32 {
        size.height_for(self.text.size())
    }

    pub fn grown_line(&self) -> f32 {
        grown_line(self.text.size())
    }
}

const DIALOG_RISE: f32 = 12.0;
const DRAG_CHARACTERS: usize = 40;
const DRAG_FONT_SIZE: f32 = 13.0;
const DRAG_OPACITY: f32 = 0.75;
const DRAG_WIDTH: f32 = 320.0;

pub trait Widget: Any {
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

    fn drop_target(&mut self, _cx: &mut Context, _point: Option<Point>) -> Option<DragAction> {
        None
    }

    fn drop_text(&mut self, _cx: &mut Context, _point: Point, _text: &str, _action: DragAction) {}

    fn drag_ended(&mut self, _action: Option<DragAction>) {}

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

    fn tooltip(&self) -> Option<&str> {
        None
    }

    fn focused_by_keyboard(&mut self) {}

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
    clock: Box<dyn Fn() -> FrameTime>,
    captured: Option<(WidgetId, PointerButton)>,
    hovered: Vec<WidgetId>,
    pointer: Option<Point>,
    menu_owner: Option<WidgetId>,
    menu_request: Option<(Menu, Point)>,
    awaiting_paste: bool,
    sheet: Option<Sheet>,
    sheet_top: f32,
    tooltips: Tooltips,
    drag_source: Option<WidgetId>,
    drop_owner: Option<WidgetId>,
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
    if !widget.frame().contains(point) {
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
    if let Some(tooltip) = widget.tooltip()
        && node.description().is_none()
    {
        node.set_description(tooltip.to_owned());
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
            clock: Box::new(clock::frame_time),
            captured: None,
            hovered: Vec::new(),
            pointer: None,
            menu_owner: None,
            menu_request: None,
            awaiting_paste: false,
            sheet: None,
            sheet_top: 0.0,
            tooltips: Tooltips::default(),
            drag_source: None,
            drop_owner: None,
        }
    }

    pub fn measure(&mut self, available: Size) -> Size {
        self.root.measure(&mut self.cx, available)
    }

    pub fn context(&mut self) -> &mut Context {
        &mut self.cx
    }

    pub fn focus(&self) -> Option<WidgetId> {
        self.cx.focus
    }

    pub fn set_focus(&mut self, id: WidgetId) {
        if self.accepts_focus(id) {
            self.cx.focus = Some(id);
        }
    }

    pub fn clear_focus(&mut self) {
        self.cx.focus = None;
    }

    pub fn widget(&mut self, id: WidgetId) -> Option<&mut dyn Widget> {
        self.lookup_mut(id)
    }

    pub fn widget_as<T: Widget + 'static>(&mut self, id: WidgetId) -> Option<&mut T> {
        let widget: &mut dyn Any = self.lookup_mut(id)?;
        widget.downcast_mut()
    }

    pub fn update_widget<T: Widget + 'static>(
        &mut self,
        id: WidgetId,
        update: impl FnOnce(&mut T, &Context),
    ) -> bool {
        self.sync_event_clock();
        let widget = lookup_in(&mut self.root, &mut self.sheet, id)
            .and_then(|widget| (widget as &mut dyn Any).downcast_mut::<T>());
        match widget {
            Some(widget) => {
                update(widget, &self.cx);
                true
            }
            None => false,
        }
    }

    pub fn widget_ref<T: Widget + 'static>(&self, id: WidgetId) -> Option<&T> {
        let widget: &dyn Any = self.lookup(id)?;
        widget.downcast_ref()
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
        self.sync_event_clock();
        self.cancel_capture();
        let previous_focus = self.cx.focus.take();
        let mut shown = Animated::new(0.0);
        self.cx.animate(&mut shown, 1.0, Spring::QUICK);
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

    pub fn set_sheet_top(&mut self, top: f32) {
        self.sheet_top = top;
    }

    pub fn pointer_captured(&self) -> bool {
        self.captured.is_some()
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
        self.cx.animate(&mut sheet.shown, 0.0, Spring::QUICK);
        self.cx.focus = sheet.previous_focus;
        self.cancel_capture();
        self.hovered.clear();
    }

    fn cancel_capture(&mut self) -> bool {
        let Some((target, _)) = self.captured.take() else {
            return false;
        };
        self.hovered.retain(|id| *id != target);
        self.deliver(target, &Event::PointerLeave)
    }

    pub fn layout(&mut self, size: Size) {
        self.root.measure(&mut self.cx, size);
        self.root
            .place(&mut self.cx, Rect::new(0.0, 0.0, size.width, size.height));
        if let Some(sheet) = &mut self.sheet {
            let measured = sheet.widget.measure(&mut self.cx, size);
            let shown = self.cx.sample(&sheet.shown);
            let y = match sheet.floating {
                true => {
                    ((size.height - measured.height) / 3.0).round() - DIALOG_RISE * (1.0 - shown)
                }
                false => self.sheet_top - measured.height * (1.0 - shown),
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
        self.clock = Box::new(move || FrameTime::immediate(clock()));
        self.sync_event_clock();
    }

    fn sync_event_clock(&mut self) {
        let on_screen = (self.clock)().on_screen();
        self.cx.now = on_screen;
        self.cx.start = on_screen;
    }

    pub fn set_window_active(&mut self, active: bool) -> bool {
        let changed = self.cx.window_active != active;
        self.cx.window_active = active;
        changed
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.cx.palette = appearance.palette();
        self.cx.reduce_motion = appearance.reduce_motion;
        self.cx.text.set_size(appearance.text_size);
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
            background: self.cx.palette.sheet,
            items,
        }
    }

    pub fn paint_layers(&mut self, width: u32, height: u32) -> (Vec<Item>, Vec<Item>) {
        let time = (self.clock)();
        self.cx.now = time.presentation;
        self.cx.start = time.on_screen();
        self.cx.animating.set(false);
        self.cx.wake.set(None);
        self.layout(Size::new(width as f32, height as f32));
        let mut items = Vec::new();
        self.root.paint(&self.cx, &mut items);
        let mut above = Vec::new();
        if let Some(sheet) = &self.sheet {
            let shown = self.cx.sample(&sheet.shown);
            if sheet.closing && shown <= 0.001 {
                self.sheet = None;
            } else {
                sheet.paint(
                    &self.cx,
                    shown,
                    Size::new(width as f32, height as f32),
                    self.sheet_top,
                    &mut above,
                );
            }
        }
        let text = self
            .tooltips
            .owner()
            .and_then(|owner| self.lookup(owner))
            .and_then(|widget| widget.tooltip())
            .map(str::to_owned);
        if let Some(text) = text
            && self.tooltips.paint(
                self.cx.now,
                &text,
                &mut self.cx.text,
                &self.cx.palette,
                Size::new(width as f32, height as f32),
                &mut above,
            )
        {
            self.cx.animating.set(true);
        }
        if let Some(wake) = self.deadline() {
            request_wake(wake);
        }
        (items, above)
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        self.sync_event_clock();
        match event {
            Event::PointerDown(pointer) => {
                let hovered = self.hover(Some(pointer));
                self.tooltips.dismiss();
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
                if self.cx.clipboard == Some(ClipboardRequest::PastePrimary)
                    && self.accepts_focus(target)
                {
                    self.cx.focus = Some(target);
                }
                if pointer.button == PointerButton::Secondary {
                    if self
                        .captured
                        .is_some_and(|(_, button)| button == PointerButton::Secondary)
                    {
                        self.captured = None;
                    }
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
            Event::PointerLeave => self.cancel_capture() | self.hover(None),
            Event::Scroll(scroll) => {
                self.tooltips.dismiss();
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
                self.tooltips.dismiss();
                self.move_focus(!modifiers.shift);
                self.show_focus_tooltip();
                true
            }
            Event::Key { key, modifiers } if opens_menu(*key, *modifiers) => {
                self.request_keyboard_menu()
            }
            Event::Key { key, modifiers } => {
                self.tooltips.dismiss();
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
        if matches!(
            request,
            Some(ClipboardRequest::Paste | ClipboardRequest::PastePrimary)
        ) {
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

    pub fn take_text_drag(&mut self) -> Option<(Scene, Point, String)> {
        let text = self.cx.drag.take()?;
        let shown: String = text
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(DRAG_CHARACTERS)
            .collect();
        let paragraph = self.cx.text.paragraph(
            &shown,
            TextStyle::new(
                DRAG_FONT_SIZE,
                self.cx.palette.ink.with_opacity(DRAG_OPACITY),
            ),
            Some(DRAG_WIDTH),
            TextAlign::Start,
        );
        let (width, height) = (paragraph.width(), paragraph.height());
        let scene = Scene {
            width: width.ceil().max(1.0) as u32,
            height: height.ceil().max(1.0) as u32,
            background: Color::TRANSPARENT,
            items: vec![Item::Text {
                origin: Point::new(0.0, 0.0),
                paragraph,
            }],
        };
        let hotspot = Point::new(0.0, height / 2.0);
        Some((scene, hotspot, text))
    }

    pub fn drag_over(&mut self, position: Option<Point>) -> Option<DragAction> {
        let mut path = Vec::new();
        if let Some(point) = position {
            hit_path(self.active(), point, &mut path);
        }
        let mut accepted = None;
        for id in path.into_iter().rev() {
            let Some(widget) = lookup_in(&mut self.root, &mut self.sheet, id) else {
                continue;
            };
            if let Some(action) = widget.drop_target(&mut self.cx, position) {
                accepted = Some((id, action));
                break;
            }
        }
        let owner = accepted.map(|(id, _)| id);
        if let Some(previous) = self.drop_owner.filter(|previous| Some(*previous) != owner)
            && let Some(widget) = lookup_in(&mut self.root, &mut self.sheet, previous)
        {
            widget.drop_target(&mut self.cx, None);
        }
        self.drop_owner = owner;
        accepted.map(|(_, action)| action)
    }

    pub fn is_drop_target(&self) -> bool {
        self.drop_owner.is_some()
    }

    pub fn dropped(&mut self, position: Point, text: &str, action: DragAction) -> bool {
        let Some(owner) = self.drop_owner.take() else {
            return false;
        };
        let Some(widget) = lookup_in(&mut self.root, &mut self.sheet, owner) else {
            return false;
        };
        widget.drop_text(&mut self.cx, position, text, action);
        widget.drop_target(&mut self.cx, None);
        if widget.accepts_focus(&self.cx) {
            self.cx.focus = Some(owner);
        }
        true
    }

    pub fn drag_finished(&mut self, action: Option<DragAction>) -> bool {
        let Some(source) = self.drag_source.take() else {
            return false;
        };
        if let Some(widget) = lookup_in(&mut self.root, &mut self.sheet, source) {
            widget.drag_ended(action);
        }
        true
    }

    pub fn take_emoji_picker_request(&mut self) -> bool {
        std::mem::take(&mut self.cx.emoji_picker)
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
        self.sync_event_clock();
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
        let owner = path.iter().rev().copied().find(|id| {
            self.lookup(*id)
                .is_some_and(|widget| widget.tooltip().is_some())
        });
        self.tooltips
            .hover(self.cx.now, owner, pointer.map(|pointer| pointer.position));
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
        if self.cx.drag.is_some() {
            self.drag_source = Some(target);
        }
        self.take_widget_menu(target);
        self.dismiss_finished_sheet();
        self.keep_focus_reachable(focus_path);
        handled
    }

    pub fn deadline(&self) -> Option<Instant> {
        let now = (self.clock)().presentation;
        self.tooltips
            .deadline(now)
            .into_iter()
            .chain(self.cx.wake.get())
            .min()
    }

    fn show_focus_tooltip(&mut self) {
        let Some(focus) = self.cx.focus else {
            return;
        };
        let Some(widget) = self.lookup(focus) else {
            return;
        };
        let owner = widget.tooltip().map(|_| focus);
        let frame = widget.frame();
        self.tooltips.focus(self.cx.now, owner, frame);
    }

    fn take_widget_menu(&mut self, owner: WidgetId) {
        if let Some(request) = self.cx.menu.take() {
            self.captured = None;
            self.menu_owner = Some(owner);
            self.menu_request = Some(request);
        }
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
        if let Some(widget) = lookup_in(&mut self.root, &mut self.sheet, order[next]) {
            widget.focused_by_keyboard();
        }
        self.reveal_focus();
    }

    pub fn reveal_focus(&mut self) {
        self.sync_event_clock();
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
                toolkit_name: Some("bande-toolkit".to_owned()),
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
        self.sync_event_clock();
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
        self.take_widget_menu(target);
        self.dismiss_finished_sheet();
        self.keep_focus_reachable(focus_path);
        performed
    }
}
