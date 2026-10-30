use accesskit::{Action, ActionRequest, HasPopup, Node, Role};
use unbound_graphics::{Item, Point, Rect};

use super::control::chevron_down;
use super::{
    Context, Cursor, Event, Key, Menu, MenuItem, Parts, Size, TextField, Widget, WidgetId, contains,
};
use crate::L10N;

const BUTTON_WIDTH: f32 = 26.0;
const MENU_GAP: f32 = 4.0;
const BUTTON_PART: u64 = 0xFFFF;

pub struct ComboBox {
    field: TextField,
    choices: Vec<String>,
}

impl ComboBox {
    pub fn new(
        placeholder: &str,
        text: &str,
        choices: &[impl AsRef<str>],
        on_change: impl FnMut(&str) + 'static,
    ) -> Self {
        Self {
            field: TextField::new(placeholder, text, on_change).with_trailing(BUTTON_WIDTH),
            choices: choices
                .iter()
                .map(|choice| choice.as_ref().to_owned())
                .collect(),
        }
    }

    pub fn text(&self) -> String {
        self.field.text()
    }

    fn button(&self) -> Rect {
        let frame = self.field.frame();
        Rect::new(
            frame.x + frame.width - BUTTON_WIDTH,
            frame.y,
            BUTTON_WIDTH,
            frame.height,
        )
    }

    fn open(&self, cx: &mut Context) {
        let text = self.field.text();
        let menu = Menu::new(
            self.choices
                .iter()
                .enumerate()
                .map(|(index, choice)| {
                    MenuItem::new(choice, &index.to_string()).checked(*choice == text)
                })
                .collect(),
        );
        let frame = self.field.frame();
        cx.show_menu(menu, Point::new(frame.x, frame.y + frame.height + MENU_GAP));
    }
}

impl Widget for ComboBox {
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
        let button = self.button();
        chevron_down(
            Point::new(
                button.x + button.width / 2.0,
                button.y + button.height / 2.0,
            ),
            cx.palette.ink2,
            items,
        );
    }

    fn access(&self, cx: &Context) -> Node {
        let mut node = self.field.access(cx);
        node.set_role(Role::EditableComboBox);
        node.set_has_popup(HasPopup::Menu);
        node
    }

    fn parts(&self, cx: &Context) -> Parts {
        let mut parts = self.field.parts(cx);
        let button = self.button();
        let id = self.id().part(BUTTON_PART);
        let mut node = Node::new(Role::Button);
        node.set_label(L10N.tr("show-choices"));
        node.add_action(Action::Click);
        node.set_bounds(accesskit::Rect {
            x0: f64::from(button.x),
            y0: f64::from(button.y),
            x1: f64::from(button.x + button.width),
            y1: f64::from(button.y + button.height),
        });
        parts.children.push(id);
        parts.nodes.push((id, node));
        parts
    }

    fn perform_part(&mut self, cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        if part != BUTTON_PART || request.action != Action::Click {
            return false;
        }
        self.open(cx);
        true
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer)
                if pointer.is_primary() && contains(self.button(), pointer.position) =>
            {
                self.open(cx);
                true
            }
            Event::Key {
                key: Key::Down,
                modifiers,
            } if *modifiers == Default::default() => {
                self.open(cx);
                true
            }
            _ => self.field.handle(cx, event),
        }
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        self.field.perform(cx, request)
    }

    fn cursor(&self, point: Point) -> Option<Cursor> {
        match contains(self.button(), point) {
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
        let choice = action
            .parse::<usize>()
            .ok()
            .and_then(|index| self.choices.get(index))
            .cloned();
        match choice {
            Some(choice) => {
                self.field.replace_text(&choice);
                true
            }
            None => self.field.menu_action(cx, action),
        }
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.field.text_input_area()
    }
}
