use accesskit::{Action, ActionRequest, Node, Role, Toggled};
use unbound_graphics::{Item, Paragraph, Point, Rect, Shape, TextAlign, TextStyle};

use super::checkbox::{CHECKBOX_SIZE, paint_box};
use super::{Animated, Context, Event, Key, Parts, Size, Spring, Widget, WidgetId};

const GAP: f32 = 8.0;
const ROW_GAP: f32 = 8.0;
const CENTRE: f32 = 5.0;
const CENTRE_RADIUS: f32 = 0.5;
const FONT_SIZE: f32 = 13.0;

struct Choice {
    title: String,
    paragraph: Option<Paragraph>,
    shown: Animated,
    frame: Rect,
}

pub struct RadioGroup {
    id: WidgetId,
    label: String,
    choices: Vec<Choice>,
    selected: Option<usize>,
    on_select: Box<dyn FnMut(usize)>,
    pressed: Option<usize>,
    frame: Rect,
}

impl RadioGroup {
    pub fn new(
        label: &str,
        choices: &[impl AsRef<str>],
        selected: Option<usize>,
        on_select: impl FnMut(usize) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            choices: choices
                .iter()
                .enumerate()
                .map(|(index, title)| Choice {
                    title: title.as_ref().to_owned(),
                    paragraph: None,
                    shown: Animated::new(if selected == Some(index) { 1.0 } else { 0.0 }),
                    frame: Rect::default(),
                })
                .collect(),
            selected,
            on_select: Box::new(on_select),
            pressed: None,
            frame: Rect::default(),
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    fn select(&mut self, cx: &Context, index: usize) {
        if index >= self.choices.len() || self.selected == Some(index) {
            return;
        }
        if let Some(previous) = self.selected.and_then(|index| self.choices.get_mut(index)) {
            previous.shown.animate_to(cx, 0.0, Spring::QUICK);
        }
        self.choices[index].shown.animate_to(cx, 1.0, Spring::QUICK);
        self.selected = Some(index);
        (self.on_select)(index);
    }

    fn choice_at(&self, point: Point) -> Option<usize> {
        self.choices
            .iter()
            .position(|choice| choice.frame.contains(point))
    }

    fn circle(frame: Rect) -> Rect {
        Rect::new(
            frame.x,
            frame.y + ((frame.height - CHECKBOX_SIZE) / 2.0).round(),
            CHECKBOX_SIZE,
            CHECKBOX_SIZE,
        )
    }
}

impl Widget for RadioGroup {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        let mut size = Size::default();
        for choice in &mut self.choices {
            let paragraph = cx.text.paragraph(
                &choice.title,
                TextStyle::new(FONT_SIZE, cx.palette.ink),
                None,
                TextAlign::Start,
            );
            let height = paragraph.height().max(CHECKBOX_SIZE).ceil();
            size.width = size
                .width
                .max((CHECKBOX_SIZE + GAP + paragraph.width()).ceil());
            choice.frame = Rect::new(0.0, 0.0, 0.0, height);
            size.height += height + ROW_GAP;
            choice.paragraph = Some(paragraph);
        }
        size.height = (size.height - ROW_GAP).max(0.0);
        size
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let mut y = frame.y;
        for choice in &mut self.choices {
            let width = choice
                .paragraph
                .as_ref()
                .map_or(CHECKBOX_SIZE, |paragraph| {
                    (CHECKBOX_SIZE + GAP + paragraph.width()).ceil()
                });
            choice.frame = Rect::new(frame.x, y, width, choice.frame.height);
            y += choice.frame.height + ROW_GAP;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let focused = cx.is_focused(self.id);
        for (index, choice) in self.choices.iter().enumerate() {
            let circle = Self::circle(choice.frame);
            let shown = choice.shown.value(cx);
            paint_box(
                cx,
                circle,
                CHECKBOX_SIZE / 2.0,
                shown,
                self.pressed == Some(index),
                items,
            );
            if shown > 0.0 {
                let centre = CENTRE * shown;
                items.push(Item::Fill {
                    shape: Shape::rounded(
                        Rect::new(
                            circle.x + (CHECKBOX_SIZE - centre) / 2.0,
                            circle.y + (CHECKBOX_SIZE - centre) / 2.0,
                            centre,
                            centre,
                        ),
                        CENTRE_RADIUS,
                    ),
                    paint: cx.palette.on_accent.into(),
                });
            }
            if focused && self.selected.unwrap_or(0) == index {
                super::focus_ring(cx, Shape::rounded(circle, CHECKBOX_SIZE / 2.0), items);
            }
            if let Some(paragraph) = &choice.paragraph {
                items.push(Item::Text {
                    origin: Point::new(
                        circle.x + CHECKBOX_SIZE + GAP,
                        choice.frame.y + ((choice.frame.height - paragraph.height()) / 2.0).round(),
                    ),
                    paragraph: paragraph.clone(),
                });
            }
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::RadioGroup);
        node.set_label(self.label.clone());
        node
    }

    fn parts(&self, _cx: &Context) -> Parts {
        let nodes: Vec<_> = self
            .choices
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                let mut node = Node::new(Role::RadioButton);
                node.set_label(choice.title.clone());
                node.set_toggled(match self.selected == Some(index) {
                    true => Toggled::True,
                    false => Toggled::False,
                });
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                node.set_bounds(accesskit::Rect {
                    x0: f64::from(choice.frame.x),
                    y0: f64::from(choice.frame.y),
                    x1: f64::from(choice.frame.x + choice.frame.width),
                    y1: f64::from(choice.frame.y + choice.frame.height),
                });
                (self.id.part(index as u64), node)
            })
            .collect();
        Parts {
            children: nodes.iter().map(|(id, _)| *id).collect(),
            focus: Some(self.id.part(self.selected.unwrap_or(0) as u64)),
            nodes,
        }
    }

    fn perform_part(&mut self, cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        match request.action {
            Action::Click => {
                self.select(cx, part as usize);
                true
            }
            Action::Focus => true,
            _ => false,
        }
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                self.pressed = self.choice_at(pointer.position);
            }
            Event::PointerUp(pointer) if pointer.is_primary() => {
                let chosen = self.pressed.take();
                if let Some(index) =
                    chosen.filter(|index| Some(*index) == self.choice_at(pointer.position))
                {
                    self.select(cx, index);
                }
            }
            Event::Key {
                key: Key::Up | Key::Left,
                ..
            } => {
                let index = self.selected.map_or(0, |index| index.saturating_sub(1));
                self.select(cx, index);
            }
            Event::Key {
                key: Key::Down | Key::Right,
                ..
            } => {
                let index = self.selected.map_or(0, |index| index + 1);
                self.select(cx, index);
            }
            _ => return false,
        }
        true
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }
}
