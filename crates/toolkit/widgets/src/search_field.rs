use std::ops::Range;

use accesskit::{Action, ActionRequest, Node, Role};
use bande_graphics::{Color, Item, Path, Point, Rect, Shape};

use super::{
    Context, Cursor, DragAction, Event, Key, Menu, Parts, Size, TextField, Widget, WidgetId,
};
use crate::L10N;

const LEADING: f32 = 24.0;
const TRAILING: f32 = 22.0;
const LENS_RADIUS: f32 = 4.5;
const STROKE: f32 = 1.5;
const CLEAR_DIAMETER: f32 = 14.0;
const CLEAR_ARM: f32 = 2.75;
const CLEAR_PART: u64 = 0xFFFF;

pub struct SearchField {
    field: TextField,
    clear_pressed: bool,
}

impl SearchField {
    pub fn new(placeholder: &str, text: &str, on_change: impl FnMut(&str) + 'static) -> Self {
        Self {
            field: TextField::new(placeholder, text, on_change).search(LEADING, TRAILING),
            clear_pressed: false,
        }
    }

    pub fn text(&self) -> String {
        self.field.text()
    }

    pub fn selection(&self) -> Range<usize> {
        self.field.selection()
    }

    pub fn select(&mut self, range: Range<usize>) {
        self.field.select(range);
    }

    fn clear_button(&self) -> Option<Rect> {
        if self.field.is_empty() {
            return None;
        }
        let frame = self.field.frame();
        Some(Rect::new(
            frame.x + frame.width - (TRAILING + CLEAR_DIAMETER) / 2.0,
            frame.y + ((frame.height - CLEAR_DIAMETER) / 2.0).round(),
            CLEAR_DIAMETER,
            CLEAR_DIAMETER,
        ))
    }

    fn paint_lens(&self, color: Color, items: &mut Vec<Item>) {
        let frame = self.field.frame();
        let centre = Point::new(frame.x + 11.0, frame.y + (frame.height / 2.0).round() - 1.0);
        items.push(Item::Stroke {
            shape: Shape::rounded(
                Rect::new(
                    centre.x - LENS_RADIUS,
                    centre.y - LENS_RADIUS,
                    2.0 * LENS_RADIUS,
                    2.0 * LENS_RADIUS,
                ),
                LENS_RADIUS,
            ),
            width: STROKE,
            paint: color.into(),
        });
        let reach = LENS_RADIUS * std::f32::consts::FRAC_1_SQRT_2;
        items.push(Item::Line {
            from: Point::new(centre.x + reach + 0.5, centre.y + reach + 0.5),
            to: Point::new(centre.x + reach + 3.5, centre.y + reach + 3.5),
            width: STROKE + 0.25,
            paint: color.into(),
        });
    }

    fn paint_clear(&self, cx: &Context, button: Rect, items: &mut Vec<Item>) {
        let fill = match self.clear_pressed {
            true => cx.palette.ink.with_alpha(160),
            false => cx.palette.ink2.with_alpha(150),
        };
        items.push(Item::Fill {
            shape: Shape::rounded(button, CLEAR_DIAMETER / 2.0),
            paint: fill.into(),
        });
        let centre = Point::new(
            button.x + button.width / 2.0,
            button.y + button.height / 2.0,
        );
        let cross = Path::new()
            .move_to(Point::new(centre.x - CLEAR_ARM, centre.y - CLEAR_ARM))
            .line_to(Point::new(centre.x + CLEAR_ARM, centre.y + CLEAR_ARM))
            .move_to(Point::new(centre.x + CLEAR_ARM, centre.y - CLEAR_ARM))
            .line_to(Point::new(centre.x - CLEAR_ARM, centre.y + CLEAR_ARM));
        items.push(Item::StrokePath {
            path: cross,
            width: STROKE,
            paint: cx.palette.overlay.into(),
        });
    }
}

impl Widget for SearchField {
    fn id(&self) -> WidgetId {
        self.field.id()
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.field.measure(cx, available)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.field.place(cx, frame);
    }

    fn frame(&self) -> Rect {
        self.field.frame()
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.field.paint(cx, items);
        self.paint_lens(cx.palette.ink3, items);
        if let Some(button) = self.clear_button() {
            self.paint_clear(cx, button, items);
        }
    }

    fn access(&self, cx: &Context) -> Node {
        let mut node = self.field.access(cx);
        node.set_role(Role::SearchInput);
        node
    }

    fn parts(&self, cx: &Context) -> Parts {
        let mut parts = self.field.parts(cx);
        if let Some(button) = self.clear_button() {
            let id = self.id().part(CLEAR_PART);
            let mut node = Node::new(Role::Button);
            node.set_label(L10N.tr("clear-search"));
            node.add_action(Action::Click);
            node.set_bounds(accesskit::Rect {
                x0: f64::from(button.x),
                y0: f64::from(button.y),
                x1: f64::from(button.x + button.width),
                y1: f64::from(button.y + button.height),
            });
            parts.children.push(id);
            parts.nodes.push((id, node));
        }
        parts
    }

    fn perform_part(&mut self, _cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        if part != CLEAR_PART || request.action != Action::Click || self.field.is_empty() {
            return false;
        }
        self.field.clear();
        true
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        let button = self.clear_button();
        let on_clear = |point: Point| button.is_some_and(|button| button.contains(point));
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() && on_clear(pointer.position) => {
                self.clear_pressed = true;
                true
            }
            Event::PointerMove(_) if self.clear_pressed => true,
            Event::PointerLeave if self.clear_pressed => {
                self.clear_pressed = false;
                self.field.handle(cx, event);
                true
            }
            Event::PointerUp(pointer) if self.clear_pressed => {
                self.clear_pressed = false;
                if on_clear(pointer.position) {
                    self.field.clear();
                }
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if *modifiers == Default::default() && !self.field.is_empty() => {
                self.field.clear();
                true
            }
            _ => self.field.handle(cx, event),
        }
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        self.field.perform(cx, request)
    }

    fn drop_target(&mut self, cx: &mut Context, point: Option<Point>) -> Option<DragAction> {
        self.field.drop_target(cx, point)
    }

    fn drop_text(&mut self, cx: &mut Context, point: Point, text: &str, action: DragAction) {
        self.field.drop_text(cx, point, text, action);
    }

    fn drag_ended(&mut self, action: Option<DragAction>) {
        self.field.drag_ended(action);
    }

    fn cursor(&self, point: Point) -> Option<Cursor> {
        match self
            .clear_button()
            .is_some_and(|button| button.contains(point))
        {
            true => Some(Cursor::Default),
            false => self.field.cursor(point),
        }
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        self.field.accepts_focus(cx)
    }

    fn focused_by_keyboard(&mut self) {
        self.field.focused_by_keyboard();
    }

    fn context_menu(&mut self, cx: &mut Context, point: Point) -> Option<Menu> {
        self.field.context_menu(cx, point)
    }

    fn menu_action(&mut self, cx: &mut Context, action: &str) -> bool {
        self.field.menu_action(cx, action)
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.field.text_input_area()
    }
}
