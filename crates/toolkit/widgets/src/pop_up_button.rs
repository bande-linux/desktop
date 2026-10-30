use accesskit::{Action, ActionRequest, HasPopup, Node, Role};
use unbound_graphics::{Item, Paragraph, Path, Point, Rect, Shape, TextAlign, TextStyle};

use super::menu::{ROW_HEIGHT, TEXT_START, VERTICAL_PADDING};
use super::{BUTTON_HEIGHT, Context, Event, Key, Menu, MenuItem, Size, Widget, WidgetId};

const RADIUS: f32 = 5.0;
const PADDING: f32 = 9.0;
const CHEVRONS: f32 = 18.0;
const CHEVRON_WIDTH: f32 = 3.5;
const CHEVRON_HEIGHT: f32 = 2.5;
const CHEVRON_GAP: f32 = 2.0;
const FONT_SIZE: f32 = 13.0;

pub struct PopUpButton {
    id: WidgetId,
    label: String,
    choices: Vec<String>,
    selected: usize,
    on_select: Box<dyn FnMut(usize)>,
    paragraphs: Vec<Paragraph>,
    tooltip: Option<String>,
    width: f32,
    frame: Rect,
}

impl PopUpButton {
    pub fn new(
        label: &str,
        choices: &[impl AsRef<str>],
        selected: usize,
        on_select: impl FnMut(usize) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            choices: choices
                .iter()
                .map(|choice| choice.as_ref().to_owned())
                .collect(),
            selected,
            on_select: Box::new(on_select),
            paragraphs: Vec::new(),
            tooltip: None,
            width: 0.0,
            frame: Rect::default(),
        }
    }

    pub fn tooltip(self, tooltip: &str) -> Self {
        Self {
            tooltip: Some(tooltip.to_owned()),
            ..self
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    fn open(&self, cx: &mut Context) {
        let menu = Menu::new(
            self.choices
                .iter()
                .enumerate()
                .map(|(index, choice)| {
                    MenuItem::new(choice, &index.to_string()).checked(index == self.selected)
                })
                .collect(),
        );
        let row = VERTICAL_PADDING + self.selected as f32 * ROW_HEIGHT;
        let at = Point::new(
            self.frame.x + PADDING - TEXT_START,
            self.frame.y + ((self.frame.height - ROW_HEIGHT) / 2.0).round() - row,
        );
        cx.show_menu(menu, at);
    }
}

impl Widget for PopUpButton {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        self.paragraphs = self
            .choices
            .iter()
            .map(|choice| {
                cx.text.paragraph(
                    choice,
                    TextStyle::new(FONT_SIZE, cx.palette.ink),
                    None,
                    TextAlign::Start,
                )
            })
            .collect();
        let widest = self
            .paragraphs
            .iter()
            .map(Paragraph::width)
            .fold(0.0, f32::max);
        self.width = (PADDING + widest + CHEVRONS).ceil();
        Size::new(self.width, BUTTON_HEIGHT)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, self.width.min(frame.width), BUTTON_HEIGHT);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, RADIUS);
        items.push(Item::Fill {
            shape,
            paint: cx.palette.overlay.into(),
        });
        items.push(Item::Stroke {
            shape: Shape::rounded(self.frame.inset(0.5), RADIUS - 0.5),
            width: 1.0,
            paint: cx.palette.hairline.into(),
        });
        if cx.is_focused(self.id) {
            super::focus_ring(cx, shape, items);
        }
        if let Some(paragraph) = self.paragraphs.get(self.selected) {
            items.push(Item::Clip {
                shape: Shape::rect(Rect::new(
                    self.frame.x,
                    self.frame.y,
                    (self.frame.width - CHEVRONS).max(0.0),
                    self.frame.height,
                )),
                items: vec![Item::Text {
                    origin: Point::new(
                        self.frame.x + PADDING,
                        self.frame.y + ((self.frame.height - paragraph.height()) / 2.0).round(),
                    ),
                    paragraph: paragraph.clone(),
                }],
            });
        }
        let centre = Point::new(
            self.frame.x + self.frame.width - CHEVRONS / 2.0 - 1.0,
            self.frame.y + self.frame.height / 2.0,
        );
        let chevron = |tip: f32, base: f32| {
            Path::new()
                .move_to(Point::new(centre.x - CHEVRON_WIDTH, base))
                .line_to(Point::new(centre.x, tip))
                .line_to(Point::new(centre.x + CHEVRON_WIDTH, base))
        };
        let up = centre.y - CHEVRON_GAP;
        let down = centre.y + CHEVRON_GAP;
        for path in [
            chevron(up - CHEVRON_HEIGHT, up),
            chevron(down + CHEVRON_HEIGHT, down),
        ] {
            items.push(Item::StrokePath {
                path,
                width: 1.5,
                paint: cx.palette.ink.into(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::ComboBox);
        node.set_label(self.label.clone());
        if let Some(choice) = self.choices.get(self.selected) {
            node.set_value(choice.clone());
        }
        node.set_has_popup(HasPopup::Menu);
        node.add_action(Action::Click);
        node
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => self.open(cx),
            Event::Key {
                key: Key::Space | Key::Up | Key::Down,
                modifiers,
            } if !modifiers.logo => self.open(cx),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action != Action::Click {
            return false;
        }
        self.open(cx);
        true
    }

    fn menu_action(&mut self, _cx: &mut Context, action: &str) -> bool {
        let Some(index) = action
            .parse::<usize>()
            .ok()
            .filter(|index| *index < self.choices.len())
        else {
            return false;
        };
        if index != self.selected {
            self.selected = index;
            (self.on_select)(index);
        }
        true
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }

    fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }
}
