use std::path::PathBuf;

use accesskit::{Node, Role};
use unbound_toolkit::ui::{BUTTON_HEIGHT, Button, Context, Size, Widget, WidgetId};
use unbound_toolkit::{
    Item, Paragraph, Point, Rect, Shape, Spacing, TextAlign, TextRole, TextStyle,
};

use crate::modules::symbol;
use crate::now_playing::fitted;
use crate::{Change, Changes, L10N};

pub const HEIGHT: f32 = 32.0;
const AVATAR: f32 = 28.0;
const BUTTON_GAP: f32 = Spacing::XS as f32;

const BUTTONS: [(&str, &str, Change); 3] = [
    ("lock-screen", "system-lock-screen-symbolic", Change::Lock),
    ("power", "system-shutdown-symbolic", Change::Power),
    (
        "system-settings",
        "emblem-system-symbolic",
        Change::Settings,
    ),
];

pub struct PersonRow {
    id: WidgetId,
    name: String,
    buttons: Vec<Box<dyn Widget>>,
    name_text: Option<Paragraph>,
    initial: Option<Paragraph>,
    frame: Rect,
}

impl PersonRow {
    pub fn new(name: &str, icon_dirs: &[PathBuf], changes: &Changes) -> Self {
        let buttons = BUTTONS
            .iter()
            .map(|(label, glyph, change)| {
                let changes = Changes::clone(changes);
                let change = change.clone();
                let push = move || changes.borrow_mut().push(change.clone());
                let label = L10N.tr(label);
                let button: Box<dyn Widget> = match symbol(glyph, icon_dirs) {
                    Some(image) => Box::new(Button::symbol(image, &label, push)),
                    None => Box::new(Button::new(&label, push)),
                };
                button
            })
            .collect();
        Self {
            id: WidgetId::next(),
            name: name.to_owned(),
            buttons,
            name_text: None,
            initial: None,
            frame: Rect::default(),
        }
    }

    fn avatar(&self) -> Rect {
        Rect::new(
            self.frame.x,
            self.frame.y + (HEIGHT - AVATAR) / 2.0,
            AVATAR,
            AVATAR,
        )
    }
}

impl Widget for PersonRow {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let widths: f32 = self
            .buttons
            .iter_mut()
            .map(|button| button.measure(cx, available).width + BUTTON_GAP)
            .sum();
        let gap = Spacing::S as f32;
        let room = available.width - widths - AVATAR - 2.0 * gap;
        let ink = cx.palette.ink;
        self.name_text = Some(fitted(
            &mut cx.text,
            &self.name,
            TextStyle::role(TextRole::BodyStrong, ink),
            room,
        ));
        let initial: String = self
            .name
            .chars()
            .next()
            .map(|first| first.to_uppercase().collect())
            .unwrap_or_default();
        self.initial = Some(cx.text.paragraph(
            &initial,
            TextStyle::role(TextRole::BodyStrong, cx.palette.ink2),
            None,
            TextAlign::Start,
        ));
        Size::new(available.width, HEIGHT)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, frame.width, HEIGHT);
        let mut right = frame.x + frame.width;
        for button in self.buttons.iter_mut().rev() {
            let size = button.measure(cx, Size::new(frame.width, HEIGHT));
            right -= size.width;
            button.place(
                cx,
                Rect::new(
                    right,
                    frame.y + ((HEIGHT - BUTTON_HEIGHT) / 2.0).round(),
                    size.width,
                    size.height,
                ),
            );
            right -= BUTTON_GAP;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let avatar = self.avatar();
        items.push(Item::Fill {
            shape: Shape::rounded(avatar, AVATAR / 2.0),
            paint: cx.palette.well.into(),
        });
        if let Some(initial) = &self.initial {
            items.push(Item::Text {
                origin: Point::new(
                    (avatar.x + (AVATAR - initial.width()) / 2.0).round(),
                    (avatar.y + (AVATAR - initial.height()) / 2.0).round(),
                ),
                paragraph: initial.clone(),
            });
        }
        if let Some(name) = &self.name_text {
            items.push(Item::Text {
                origin: Point::new(
                    avatar.x + AVATAR + Spacing::S as f32,
                    (self.frame.y + (HEIGHT - name.height()) / 2.0).round(),
                ),
                paragraph: name.clone(),
            });
        }
        for button in &self.buttons {
            button.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Group);
        node.set_label(self.name.clone());
        node
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.buttons
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.buttons
    }
}
