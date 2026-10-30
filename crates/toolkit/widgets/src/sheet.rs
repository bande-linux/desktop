use std::cell::{Cell, RefCell};
use std::rc::Rc;

use accesskit::{Node, Role};
use unbound_graphics::{Color, Item, Paragraph, Point, Rect, Shape, TextAlign, TextStyle};

use super::{Animated, BUTTON_HEIGHT, Button, Context, Size, Widget, WidgetId};

const ALERT_WIDTH: f32 = 380.0;
const PADDING: f32 = 20.0;
const TITLE_GAP: f32 = 6.0;
const BUTTONS_GAP: f32 = 18.0;
const BUTTON_SPACING: f32 = 8.0;
const BUTTON_MIN_WIDTH: f32 = 76.0;
const RADIUS: f32 = 10.0;

pub(crate) struct Sheet {
    pub(crate) widget: Box<dyn Widget>,
    pub(crate) shown: Animated,
    pub(crate) closing: bool,
    pub(crate) previous_focus: Option<WidgetId>,
    pub(crate) floating: bool,
}

impl Sheet {
    pub(crate) fn paint(&self, cx: &Context, shown: f32, area: Size, items: &mut Vec<Item>) {
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(0.0, 0.0, area.width, area.height)),
            paint: Color::hex(0x000000)
                .with_alpha((shown.clamp(0.0, 1.0) * 40.0) as u8)
                .into(),
        });
        let frame = self.widget.frame();
        let panel = match self.floating {
            true => frame,
            false => Rect::new(
                frame.x,
                frame.y - RADIUS,
                frame.width,
                frame.height + RADIUS,
            ),
        };
        let shape = Shape::rounded(panel, RADIUS);
        let mut content = vec![
            Item::Shadow {
                shape,
                offset: Point::new(0.0, 8.0),
                blur_radius: 24.0,
                color: Color::hex(0x000000).with_alpha(72),
            },
            Item::Fill {
                shape,
                paint: cx.palette.surface_elevated.into(),
            },
        ];
        self.widget.paint(cx, &mut content);
        items.push(Item::Opacity {
            alpha: shown.clamp(0.0, 1.0),
            items: content,
        });
    }
}

type Choice = Rc<RefCell<Box<dyn FnMut(&str)>>>;

pub struct Alert {
    id: WidgetId,
    title: String,
    message: String,
    buttons: Vec<Box<dyn Widget>>,
    primary_count: usize,
    chosen: Rc<Cell<bool>>,
    on_choice: Choice,
    title_text: Option<Paragraph>,
    message_text: Option<Paragraph>,
    button_sizes: Vec<Size>,
    frame: Rect,
}

impl Alert {
    pub fn new(title: &str, message: &str, on_choice: impl FnMut(&str) + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            title: title.to_owned(),
            message: message.to_owned(),
            buttons: Vec::new(),
            primary_count: 0,
            chosen: Rc::new(Cell::new(false)),
            on_choice: Rc::new(RefCell::new(Box::new(on_choice))),
            title_text: None,
            message_text: None,
            button_sizes: Vec::new(),
            frame: Rect::default(),
        }
    }

    fn make(&self, title: &str, action: &str) -> Button {
        let (chosen, on_choice, action) = (
            self.chosen.clone(),
            self.on_choice.clone(),
            action.to_owned(),
        );
        Button::new(title, move || {
            if !chosen.replace(true) {
                (on_choice.borrow_mut())(&action);
            }
        })
    }

    pub fn primary(mut self, title: &str, action: &str) -> Self {
        let button = self.make(title, action).primary();
        self.buttons.push(Box::new(button));
        self.primary_count += 1;
        self
    }

    pub fn cancel(mut self, title: &str, action: &str) -> Self {
        let button = self.make(title, action).cancel();
        let at = self.buttons.len() - self.primary_count;
        self.buttons.insert(at, Box::new(button));
        self
    }

    pub fn other(mut self, title: &str, action: &str) -> Self {
        let button = self.make(title, action);
        self.buttons.insert(0, Box::new(button));
        self
    }
}

impl Widget for Alert {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = ALERT_WIDTH.min(available.width);
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
        self.button_sizes = self
            .buttons
            .iter_mut()
            .map(|button| {
                let size = button.measure(cx, available);
                Size::new(size.width.max(BUTTON_MIN_WIDTH), BUTTON_HEIGHT)
            })
            .collect();
        let message_height = message
            .as_ref()
            .map_or(0.0, |message| TITLE_GAP + message.height());
        let height =
            PADDING + title.height() + message_height + BUTTONS_GAP + BUTTON_HEIGHT + PADDING;
        self.title_text = Some(title);
        self.message_text = message;
        Size::new(width, height.ceil())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let mut x = frame.x + frame.width - PADDING;
        let y = frame.y + frame.height - PADDING - BUTTON_HEIGHT;
        for (button, size) in self.buttons.iter_mut().zip(&self.button_sizes).rev() {
            x -= size.width;
            button.place(cx, Rect::new(x, y, size.width, size.height));
            x -= BUTTON_SPACING;
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
        for button in &self.buttons {
            button.paint(cx, items);
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
        &self.buttons
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.buttons
    }
}
