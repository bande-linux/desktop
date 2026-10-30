use std::cell::Cell;
use std::rc::Rc;

use accesskit::{Node, Role};
use unbound_toolkit::ui::{BUTTON_HEIGHT, Button, Context, Size, Toggle, Widget, WidgetId};
use unbound_toolkit::{Item, Paragraph, Point, Rect, TextAlign, TextStyle};

const WIDTH: f32 = 440.0;
const PADDING: f32 = 20.0;
const TITLE_GAP: f32 = 6.0;
const ACCESSORY_GAP: f32 = 14.0;
const BUTTONS_GAP: f32 = 18.0;
const BUTTON_SPACING: f32 = 8.0;
const BUTTON_MIN_WIDTH: f32 = 76.0;
const GROUP_GAP: f32 = 24.0;
const TOGGLE_LABEL_GAP: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Other,
    Cancel,
    Primary,
}

type Choose = Rc<dyn Fn(&str)>;

pub(crate) struct Prompt {
    id: WidgetId,
    title: String,
    message: String,
    toggle_label: Option<String>,
    children: Vec<Box<dyn Widget>>,
    roles: Vec<Kind>,
    accessory: bool,
    chosen: Rc<Cell<bool>>,
    choose: Choose,
    title_text: Option<Paragraph>,
    message_text: Option<Paragraph>,
    toggle_text: Option<Paragraph>,
    sizes: Vec<Size>,
    frame: Rect,
}

impl Prompt {
    pub(crate) fn new(title: &str, message: &str, choose: impl Fn(&str) + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            title: title.to_owned(),
            message: message.to_owned(),
            toggle_label: None,
            children: Vec::new(),
            roles: Vec::new(),
            accessory: false,
            chosen: Rc::new(Cell::new(false)),
            choose: Rc::new(choose),
            title_text: None,
            message_text: None,
            toggle_text: None,
            sizes: Vec::new(),
            frame: Rect::default(),
        }
    }

    pub(crate) fn accessory(mut self, widget: impl Widget + 'static) -> Self {
        self.children.insert(0, Box::new(widget));
        self.accessory = true;
        self
    }

    pub(crate) fn toggle(self, label: &str, toggle: Toggle) -> Self {
        Self {
            toggle_label: Some(label.to_owned()),
            ..self.accessory(toggle)
        }
    }

    pub(crate) fn button(mut self, title: &str, action: &str, role: Kind) -> Self {
        let (chosen, choose, action) =
            (self.chosen.clone(), self.choose.clone(), action.to_owned());
        let button = Button::new(title, move || {
            if !chosen.replace(true) {
                choose(&action);
            }
        });
        let button = match role {
            Kind::Other => button,
            Kind::Cancel => button.cancel(),
            Kind::Primary => button.primary(),
        };
        self.children.push(Box::new(button));
        self.roles.push(role);
        self
    }

    pub(crate) fn cancel_button(&self) -> Option<WidgetId> {
        let first = self.first_button();
        self.roles
            .iter()
            .position(|role| *role == Kind::Cancel)
            .map(|index| self.children[first + index].id())
    }

    fn first_button(&self) -> usize {
        usize::from(self.accessory)
    }
}

impl Widget for Prompt {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = WIDTH.min(available.width - 2.0 * PADDING);
        let text_width = width - 2.0 * PADDING;
        let title = cx.text.paragraph(
            &self.title,
            TextStyle::new(13.0, cx.palette.label_primary).weight(TextStyle::SEMIBOLD),
            Some(text_width),
            TextAlign::Start,
        );
        let message = (!self.message.is_empty()).then(|| {
            cx.text.paragraph(
                &self.message,
                TextStyle::new(12.0, cx.palette.label_muted),
                Some(text_width),
                TextAlign::Start,
            )
        });
        self.toggle_text = self.toggle_label.as_ref().map(|label| {
            cx.text.paragraph(
                label,
                TextStyle::new(13.0, cx.palette.label_primary),
                None,
                TextAlign::Start,
            )
        });
        let first = self.first_button();
        self.sizes = self
            .children
            .iter_mut()
            .enumerate()
            .map(|(index, child)| {
                let size = child.measure(cx, Size::new(text_width, available.height));
                match index < first {
                    true => size,
                    false => Size::new(size.width.max(BUTTON_MIN_WIDTH), BUTTON_HEIGHT),
                }
            })
            .collect();
        let accessory = match self.accessory {
            true => ACCESSORY_GAP + self.sizes[0].height,
            false => 0.0,
        };
        let message_height = message
            .as_ref()
            .map_or(0.0, |message| TITLE_GAP + message.height());
        let height = PADDING
            + title.height()
            + message_height
            + accessory
            + BUTTONS_GAP
            + BUTTON_HEIGHT
            + PADDING;
        self.title_text = Some(title);
        self.message_text = message;
        Size::new(width, height.ceil())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let first = self.first_button();
        if self.accessory {
            let size = self.sizes[0];
            let y = frame.y + frame.height - PADDING - BUTTON_HEIGHT - BUTTONS_GAP - size.height;
            let width = match self.toggle_label {
                Some(_) => size.width,
                None => frame.width - 2.0 * PADDING,
            };
            self.children[0].place(cx, Rect::new(frame.x + PADDING, y, width, size.height));
        }
        let y = frame.y + frame.height - PADDING - BUTTON_HEIGHT;
        let mut x = frame.x + frame.width - PADDING;
        let mut previous = None;
        for index in (first..self.children.len()).rev() {
            let role = self.roles[index - first];
            if previous.is_some_and(|previous| previous != Kind::Other) && role == Kind::Other {
                x -= GROUP_GAP - BUTTON_SPACING;
            }
            let size = self.sizes[index];
            x -= size.width;
            self.children[index].place(cx, Rect::new(x, y, size.width, size.height));
            x -= BUTTON_SPACING;
            previous = Some(role);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let mut y = self.frame.y + PADDING;
        if let Some(title) = &self.title_text {
            items.push(Item::Text {
                origin: Point::new(self.frame.x + PADDING, y),
                paragraph: title.clone(),
            });
            y += title.height() + TITLE_GAP;
        }
        if let Some(message) = &self.message_text {
            items.push(Item::Text {
                origin: Point::new(self.frame.x + PADDING, y),
                paragraph: message.clone(),
            });
        }
        if let (Some(label), Some(toggle)) = (&self.toggle_text, self.children.first()) {
            let frame = toggle.frame();
            items.push(Item::Text {
                origin: Point::new(
                    frame.x + frame.width + TOGGLE_LABEL_GAP,
                    frame.y + ((frame.height - label.height()) / 2.0).round(),
                ),
                paragraph: label.clone(),
            });
        }
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::AlertDialog);
        node.set_label(self.title.clone());
        node.set_description(self.message.clone());
        node.set_modal();
        node
    }

    fn dismissed(&self) -> bool {
        self.chosen.get()
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}
