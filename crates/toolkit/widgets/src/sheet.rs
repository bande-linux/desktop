use std::cell::{Cell, RefCell};
use std::rc::Rc;

use accesskit::{Node, Role};
use bande_graphics::{
    ControlSize, Elevation, Image, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign,
    TextRole, TextStyle,
};

use super::{Animated, BUTTON_HEIGHT, Button, Context, Size, Widget, WidgetId};

const ALERT_WIDTH: f32 = 400.0;
const PADDING: f32 = 24.0;
const ICON_SIZE: f32 = 36.0;
const ICON_GAP: f32 = 16.0;
const TITLE_GAP: f32 = 6.0;
const FOOTER_PADDING: f32 = 14.0;
const FOOTER_INSET: f32 = 16.0;
const BUTTON_SPACING: f32 = 8.0;
const BUTTON_MIN_WIDTH: f32 = 96.0;

pub(crate) struct Sheet {
    pub(crate) widget: Box<dyn Widget>,
    pub(crate) shown: Animated,
    pub(crate) closing: bool,
    pub(crate) previous_focus: Option<WidgetId>,
    pub(crate) floating: bool,
}

impl Sheet {
    pub(crate) fn paint(
        &self,
        cx: &Context,
        shown: f32,
        area: Size,
        top: f32,
        items: &mut Vec<Item>,
    ) {
        let shown = shown.clamp(0.0, 1.0);
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(0.0, 0.0, area.width, area.height)),
            paint: cx.palette.scrim.with_opacity(shown).into(),
        });
        let frame = self.widget.frame();
        let panel = match self.floating {
            true => frame,
            false => Rect::new(
                frame.x,
                frame.y - Radius::XL,
                frame.width,
                frame.height + Radius::XL,
            ),
        };
        let shape = Shape::rounded(panel, Radius::XL);
        let mut content = Item::shadows(shape, Elevation::Frame, cx.palette.scheme, true).to_vec();
        content.push(Item::Fill {
            shape,
            paint: cx.palette.overlay.into(),
        });
        let mut inner = Vec::new();
        self.widget.paint(cx, &mut inner);
        content.push(Item::Clip {
            shape,
            items: inner,
        });
        let content = match self.floating {
            true => content,
            false => vec![Item::Clip {
                shape: Shape::rect(Rect::new(
                    0.0,
                    top,
                    area.width,
                    (area.height - top).max(0.0),
                )),
                items: content,
            }],
        };
        items.push(Item::Opacity {
            alpha: shown,
            items: content,
        });
    }
}

type Choice = Rc<RefCell<Box<dyn FnMut(&str)>>>;

pub struct Alert {
    id: WidgetId,
    title: String,
    message: String,
    icon: Option<Image>,
    buttons: Vec<Box<dyn Widget>>,
    primary_count: usize,
    other_count: usize,
    chosen: Rc<Cell<bool>>,
    on_choice: Choice,
    title_text: Option<Paragraph>,
    message_text: Option<Paragraph>,
    button_sizes: Vec<Size>,
    stacked: bool,
    button_height: f32,
    frame: Rect,
}

impl Alert {
    pub fn new(title: &str, message: &str, on_choice: impl FnMut(&str) + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            title: title.to_owned(),
            message: message.to_owned(),
            icon: None,
            buttons: Vec::new(),
            primary_count: 0,
            button_height: BUTTON_HEIGHT,
            other_count: 0,
            chosen: Rc::new(Cell::new(false)),
            on_choice: Rc::new(RefCell::new(Box::new(on_choice))),
            title_text: None,
            message_text: None,
            button_sizes: Vec::new(),
            stacked: false,
            frame: Rect::default(),
        }
    }

    pub fn icon(self, icon: Image) -> Self {
        Self {
            icon: Some(icon),
            ..self
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

    pub fn destructive(mut self, title: &str, action: &str) -> Self {
        let button = self.make(title, action).destructive();
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
        let button = self.make(title, action).quiet();
        self.buttons.insert(self.other_count, Box::new(button));
        self.other_count += 1;
        self
    }

    fn text_x(&self) -> f32 {
        match self.icon {
            Some(_) => PADDING + ICON_SIZE + ICON_GAP,
            None => PADDING,
        }
    }

    fn footer_height(&self) -> f32 {
        let rows = match self.stacked {
            true => self.buttons.len().max(1),
            false => 1,
        } as f32;
        rows * self.button_height + (rows - 1.0) * BUTTON_SPACING + 2.0 * FOOTER_PADDING
    }
}

impl Widget for Alert {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = ALERT_WIDTH.min(available.width);
        let text_width = width - self.text_x() - PADDING;
        let title = cx.text.paragraph(
            &self.title,
            TextStyle::role(TextRole::Heading, cx.palette.ink),
            Some(text_width),
            TextAlign::Start,
        );
        let message = (!self.message.is_empty()).then(|| {
            cx.text.paragraph(
                &self.message,
                TextStyle::role(TextRole::Body, cx.palette.ink2),
                Some(text_width),
                TextAlign::Start,
            )
        });
        let other_count = self.other_count;
        self.button_height = cx.control_height(ControlSize::Regular);
        let button_height = self.button_height;
        self.button_sizes = self
            .buttons
            .iter_mut()
            .enumerate()
            .map(|(index, button)| {
                let size = button.measure(cx, available);
                match index < other_count {
                    true => size,
                    false => Size::new(size.width.max(BUTTON_MIN_WIDTH), button_height),
                }
            })
            .collect();
        let message_height = message
            .as_ref()
            .map_or(0.0, |message| TITLE_GAP + message.height());
        let text_height = title.height() + message_height;
        let body = match self.icon {
            Some(_) => text_height.max(ICON_SIZE),
            None => text_height,
        };
        let row = self.button_sizes.iter().map(|size| size.width).sum::<f32>()
            + BUTTON_SPACING * self.button_sizes.len().saturating_sub(1) as f32
            + 2.0 * FOOTER_INSET;
        self.stacked = row > width;
        let height = PADDING + body + PADDING + self.footer_height();
        self.title_text = Some(title);
        self.message_text = message;
        Size::new(width, height.ceil())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        if self.stacked {
            let mut y = frame.y + frame.height - FOOTER_PADDING - self.button_height;
            let width = frame.width - 2.0 * FOOTER_INSET;
            for button in &mut self.buttons {
                button.place(
                    cx,
                    Rect::new(frame.x + FOOTER_INSET, y, width, self.button_height),
                );
                y -= self.button_height + BUTTON_SPACING;
            }
            return;
        }
        let y = frame.y + frame.height - FOOTER_PADDING - self.button_height;
        let mut right = frame.x + frame.width - FOOTER_INSET;
        let mut left = frame.x + FOOTER_INSET;
        let other_count = self.other_count;
        for (index, (button, size)) in self
            .buttons
            .iter_mut()
            .zip(&self.button_sizes)
            .enumerate()
            .rev()
        {
            if index < other_count {
                continue;
            }
            right -= size.width;
            button.place(cx, Rect::new(right, y, size.width, size.height));
            right -= BUTTON_SPACING;
        }
        for (button, size) in self
            .buttons
            .iter_mut()
            .zip(&self.button_sizes)
            .take(other_count)
        {
            button.place(cx, Rect::new(left, y, size.width, size.height));
            left += size.width + BUTTON_SPACING;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let footer = self.footer_height();
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(
                self.frame.x,
                self.frame.y + self.frame.height - footer,
                self.frame.width,
                footer,
            )),
            paint: cx.palette.chassis.into(),
        });
        if let Some(icon) = &self.icon {
            items.push(Item::Image {
                rect: Rect::new(
                    self.frame.x + PADDING,
                    self.frame.y + PADDING,
                    ICON_SIZE,
                    ICON_SIZE,
                ),
                image: icon.clone(),
            });
        }
        let x = self.frame.x + self.text_x();
        let mut y = self.frame.y + PADDING;
        if let Some(title) = &self.title_text {
            items.push(Item::Text {
                origin: Point::new(x, y),
                paragraph: title.clone(),
            });
            y += title.height() + TITLE_GAP;
        }
        if let Some(message) = &self.message_text {
            items.push(Item::Text {
                origin: Point::new(x, y),
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
