use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Node, Role};
use unbound_toolkit::ui::{Context, Event, Key, Parts, Size, Widget, WidgetId, focus_ring_for};
use unbound_toolkit::{
    Color, Elevation, Item, Paragraph, Point, Radius, Rect, Scheme, Shape, TextAlign, TextRole,
    TextStyle,
};

use crate::L10N;

const HEIGHT: f32 = 52.0;
const SEGMENT: f32 = 44.0;
const PADDING: f32 = 6.0;
const GAP: f32 = 2.0;
const DIVIDER: f32 = 13.0;
const RADIUS: f32 = Radius::XL;
const SEGMENT_RADIUS: f32 = Radius::S;
const HOVER_INK: f32 = 0.06;
const CHOSEN_INK: f32 = 0.1;
const PRESSED_INK: f32 = 0.14;
const BUTTON_PADDING: f32 = 14.0;
const BUTTON_HEIGHT: f32 = 28.0;
const STROKE: f32 = 1.6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Screen,
    Window,
    Area,
    RecordScreen,
    RecordArea,
}

impl Mode {
    pub fn records(self) -> bool {
        matches!(self, Self::RecordScreen | Self::RecordArea)
    }

    fn label(self) -> String {
        L10N.tr(match self {
            Self::Screen => "capture-entire-screen",
            Self::Window => "capture-window",
            Self::Area => "capture-area",
            Self::RecordScreen => "record-entire-screen",
            Self::RecordArea => "record-area",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PaletteAction {
    Close,
    Capture(Mode),
    Options(Rect),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Segment {
    Close,
    Mode(Mode),
    Options,
    Go,
}

const SEGMENTS: [Segment; 8] = [
    Segment::Close,
    Segment::Mode(Mode::Screen),
    Segment::Mode(Mode::Window),
    Segment::Mode(Mode::Area),
    Segment::Mode(Mode::RecordScreen),
    Segment::Mode(Mode::RecordArea),
    Segment::Options,
    Segment::Go,
];

pub type Chosen = Rc<RefCell<Option<PaletteAction>>>;

pub struct Palette {
    id: WidgetId,
    mode: Mode,
    cursor: usize,
    pressed: Option<usize>,
    hovered: Option<usize>,
    chosen: Chosen,
    frames: [Rect; 8],
    options_text: Option<Paragraph>,
    go_text: Option<Paragraph>,
    frame: Rect,
}

impl Palette {
    pub fn new(mode: Mode) -> (Self, Chosen) {
        let chosen = Chosen::default();
        let cursor = SEGMENTS
            .iter()
            .position(|segment| *segment == Segment::Mode(mode))
            .unwrap_or(1);
        (
            Self {
                id: WidgetId::next(),
                mode,
                cursor,
                pressed: None,
                hovered: None,
                chosen: Rc::clone(&chosen),
                frames: [Rect::default(); 8],
                options_text: None,
                go_text: None,
                frame: Rect::default(),
            },
            chosen,
        )
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    fn label(&self, segment: Segment) -> String {
        match segment {
            Segment::Close => L10N.tr("close"),
            Segment::Mode(mode) => mode.label(),
            Segment::Options => L10N.tr("options"),
            Segment::Go => self.go_title(),
        }
    }

    fn go_title(&self) -> String {
        match self.mode.records() {
            true => L10N.tr("record"),
            false => L10N.tr("capture"),
        }
    }

    fn activate(&mut self, index: usize) {
        let action = match SEGMENTS[index] {
            Segment::Close => Some(PaletteAction::Close),
            Segment::Mode(mode) => {
                self.mode = mode;
                None
            }
            Segment::Options => Some(PaletteAction::Options(self.frames[index])),
            Segment::Go => Some(PaletteAction::Capture(self.mode)),
        };
        if action.is_some() {
            *self.chosen.borrow_mut() = action;
        }
    }

    fn segment_at(&self, point: Point) -> Option<usize> {
        self.frames.iter().position(|frame| {
            point.x >= frame.x
                && point.y >= frame.y
                && point.x < frame.x + frame.width
                && point.y < frame.y + frame.height
        })
    }

    fn glyph(&self, mode: Mode, center: Point, color: Color, items: &mut Vec<Item>) {
        let screen = Rect::new(center.x - 10.0, center.y - 7.0, 20.0, 14.0);
        let outline = |rect: Rect, items: &mut Vec<Item>| {
            items.push(Item::Stroke {
                shape: Shape::rounded(rect, 2.5),
                width: STROKE,
                paint: color.into(),
            });
        };
        let corners = |rect: Rect, items: &mut Vec<Item>| {
            let arm = 4.0;
            for (x, y, dx, dy) in [
                (rect.x, rect.y, 1.0, 1.0),
                (rect.x + rect.width, rect.y, -1.0, 1.0),
                (rect.x, rect.y + rect.height, 1.0, -1.0),
                (rect.x + rect.width, rect.y + rect.height, -1.0, -1.0),
            ] {
                items.push(Item::Line {
                    from: Point::new(x, y),
                    to: Point::new(x + dx * arm, y),
                    width: STROKE,
                    paint: color.into(),
                });
                items.push(Item::Line {
                    from: Point::new(x, y),
                    to: Point::new(x, y + dy * arm),
                    width: STROKE,
                    paint: color.into(),
                });
            }
        };
        let dot = |items: &mut Vec<Item>| {
            items.push(Item::Fill {
                shape: Shape::rounded(Rect::new(center.x - 3.0, center.y - 3.0, 6.0, 6.0), 3.0),
                paint: color.into(),
            });
        };
        match mode {
            Mode::Screen => outline(screen, items),
            Mode::Window => {
                let window = Rect::new(center.x - 9.0, center.y - 7.0, 18.0, 14.0);
                outline(window, items);
                items.push(Item::Fill {
                    shape: Shape::rect(Rect::new(window.x, window.y + 3.5, window.width, STROKE)),
                    paint: color.into(),
                });
            }
            Mode::Area => corners(screen, items),
            Mode::RecordScreen => {
                outline(screen, items);
                dot(items);
            }
            Mode::RecordArea => {
                corners(screen, items);
                dot(items);
            }
        }
    }
}

impl Widget for Palette {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        let style = TextStyle::role(TextRole::Body, cx.palette.ink);
        let options = cx.text.paragraph(
            &format!("{} ▾", L10N.tr("options")),
            style,
            None,
            TextAlign::Start,
        );
        let go = cx.text.paragraph(
            &self.go_title(),
            TextStyle::role(TextRole::BodyStrong, cx.palette.on_accent),
            None,
            TextAlign::Start,
        );
        let width = PADDING * 2.0
            + SEGMENT * 6.0
            + GAP * 5.0
            + DIVIDER * 3.0
            + options.width()
            + BUTTON_PADDING * 2.0
            + GAP
            + go.width()
            + BUTTON_PADDING * 2.0;
        self.options_text = Some(options);
        self.go_text = Some(go);
        Size::new(width.ceil(), HEIGHT)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let mut x = frame.x + PADDING;
        let top = frame.y + (HEIGHT - SEGMENT) / 2.0;
        let button_top = frame.y + (HEIGHT - BUTTON_HEIGHT) / 2.0;
        for (index, segment) in SEGMENTS.iter().enumerate() {
            if matches!(index, 1 | 4 | 6) {
                x += DIVIDER - GAP;
            }
            self.frames[index] = match segment {
                Segment::Close | Segment::Mode(_) => Rect::new(x, top, SEGMENT, SEGMENT),
                Segment::Options => Rect::new(
                    x,
                    button_top,
                    self.options_text.as_ref().map_or(0.0, Paragraph::width) + BUTTON_PADDING * 2.0,
                    BUTTON_HEIGHT,
                ),
                Segment::Go => Rect::new(
                    x,
                    button_top,
                    self.go_text.as_ref().map_or(0.0, Paragraph::width) + BUTTON_PADDING * 2.0,
                    BUTTON_HEIGHT,
                ),
            };
            x += self.frames[index].width + GAP;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, RADIUS);
        items.extend(Item::shadows(
            shape,
            Elevation::Overlay,
            cx.palette.scheme,
            true,
        ));
        items.push(Item::Fill {
            shape,
            paint: cx.palette.overlay.into(),
        });
        if cx.palette.scheme == Scheme::Dark {
            items.push(Item::Stroke {
                shape: Shape::rounded(self.frame.inset(0.5), RADIUS - 0.5),
                width: 1.0,
                paint: cx.palette.hairline.into(),
            });
        }
        for divider in [1, 4, 6] {
            let x = self.frames[divider].x - (DIVIDER + GAP) / 2.0;
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(x, self.frame.y + 14.0, 1.0, HEIGHT - 28.0)),
                paint: cx.palette.hairline.into(),
            });
        }
        let focused = cx.is_focused(self.id) && cx.keyboard_navigation;
        for (index, segment) in SEGMENTS.iter().enumerate() {
            let frame = self.frames[index];
            let center = Point::new(frame.x + frame.width / 2.0, frame.y + frame.height / 2.0);
            let chosen = *segment == Segment::Mode(self.mode);
            let highlighted = self.pressed == Some(index) || self.hovered == Some(index);
            if chosen || highlighted {
                let ink = match (self.pressed == Some(index), chosen) {
                    (true, _) => PRESSED_INK,
                    (false, true) => CHOSEN_INK,
                    (false, false) => HOVER_INK,
                };
                let fill = cx.palette.overlay.mix(cx.palette.ink, ink);
                items.push(Item::Fill {
                    shape: Shape::rounded(frame, SEGMENT_RADIUS),
                    paint: fill.into(),
                });
            }
            match segment {
                Segment::Close => {
                    let color = cx.palette.ink2;
                    let arm = 5.0;
                    items.push(Item::Line {
                        from: Point::new(center.x - arm, center.y - arm),
                        to: Point::new(center.x + arm, center.y + arm),
                        width: STROKE,
                        paint: color.into(),
                    });
                    items.push(Item::Line {
                        from: Point::new(center.x + arm, center.y - arm),
                        to: Point::new(center.x - arm, center.y + arm),
                        width: STROKE,
                        paint: color.into(),
                    });
                }
                Segment::Mode(mode) => {
                    let color = match chosen {
                        true => cx.palette.accent,
                        false => cx.palette.ink,
                    };
                    self.glyph(*mode, center, color, items);
                }
                Segment::Options => {
                    if let Some(text) = &self.options_text {
                        items.push(Item::Text {
                            origin: Point::new(
                                frame.x + BUTTON_PADDING,
                                (center.y - text.height() / 2.0).round(),
                            ),
                            paragraph: text.clone(),
                        });
                    }
                }
                Segment::Go => {
                    items.push(Item::Fill {
                        shape: Shape::rounded(frame, SEGMENT_RADIUS),
                        paint: cx.palette.accent.into(),
                    });
                    if let Some(text) = &self.go_text {
                        items.push(Item::Text {
                            origin: Point::new(
                                frame.x + BUTTON_PADDING,
                                (center.y - text.height() / 2.0).round(),
                            ),
                            paragraph: text.clone(),
                        });
                    }
                }
            }
            if focused && index == self.cursor {
                focus_ring_for(&cx.palette, Shape::rounded(frame, SEGMENT_RADIUS), items);
            }
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Toolbar);
        node.set_label(L10N.tr("capture-options"));
        node
    }

    fn parts(&self, cx: &Context) -> Parts {
        let nodes: Vec<_> = SEGMENTS
            .iter()
            .enumerate()
            .map(|(index, segment)| {
                let role = match segment {
                    Segment::Mode(_) => Role::RadioButton,
                    _ => Role::Button,
                };
                let mut node = Node::new(role);
                node.set_label(self.label(*segment));
                if let Segment::Mode(mode) = segment {
                    node.set_toggled(match *mode == self.mode {
                        true => accesskit::Toggled::True,
                        false => accesskit::Toggled::False,
                    });
                }
                if *segment == Segment::Options {
                    node.set_has_popup(accesskit::HasPopup::Menu);
                }
                node.add_action(Action::Click);
                let frame = self.frames[index];
                node.set_bounds(accesskit::Rect {
                    x0: f64::from(frame.x),
                    y0: f64::from(frame.y),
                    x1: f64::from(frame.x + frame.width),
                    y1: f64::from(frame.y + frame.height),
                });
                (self.id.part(index as u64), node)
            })
            .collect();
        Parts {
            children: nodes.iter().map(|(id, _)| *id).collect(),
            nodes,
            focus: cx
                .is_focused(self.id)
                .then(|| self.id.part(self.cursor as u64)),
        }
    }

    fn perform_part(&mut self, _cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        let index = part as usize;
        if index >= SEGMENTS.len() {
            return false;
        }
        match request.action {
            Action::Click => self.activate(index),
            Action::Focus => self.cursor = index,
            _ => return false,
        }
        true
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerMove(pointer) => {
                let hovered = self.segment_at(pointer.position);
                let changed = hovered != self.hovered;
                self.hovered = hovered;
                changed
            }
            Event::PointerDown(pointer) if pointer.is_primary() => {
                self.pressed = self.segment_at(pointer.position);
                if let Some(index) = self.pressed {
                    self.cursor = index;
                }
                true
            }
            Event::PointerUp(pointer) => {
                let released = self.segment_at(pointer.position);
                if let Some(index) = self.pressed.take().filter(|index| Some(*index) == released) {
                    self.activate(index);
                }
                true
            }
            Event::Key { key, .. } => match key {
                Key::Left => {
                    self.cursor = self.cursor.saturating_sub(1);
                    true
                }
                Key::Right => {
                    self.cursor = (self.cursor + 1).min(SEGMENTS.len() - 1);
                    true
                }
                Key::Space => {
                    self.activate(self.cursor);
                    true
                }
                Key::Enter => {
                    match SEGMENTS[self.cursor] {
                        Segment::Mode(mode) => {
                            self.mode = mode;
                            *self.chosen.borrow_mut() = Some(PaletteAction::Capture(mode));
                        }
                        _ => self.activate(self.cursor),
                    }
                    true
                }
                Key::Escape => {
                    *self.chosen.borrow_mut() = Some(PaletteAction::Close);
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_toolkit::ui::{FreeLayout, Modifiers, Ui};
    use unbound_toolkit::{Appearance, TextSystem};

    fn shown(mode: Mode) -> (Ui, Chosen) {
        let appearance = Appearance::default();
        let mut layout = FreeLayout::new();
        let (palette, chosen) = Palette::new(mode);
        layout.insert(palette, Rect::new(100.0, 600.0, 520.0, HEIGHT));
        let mut ui = Ui::new(
            Box::new(layout),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.layout(Size::new(1280.0, 800.0));
        (ui, chosen)
    }

    fn key(ui: &mut Ui, key: Key) {
        ui.handle(&Event::Key {
            key,
            modifiers: Modifiers::default(),
        });
    }

    fn focus(ui: &mut Ui) {
        key(ui, Key::Tab);
    }

    #[test]
    fn return_captures_with_the_mode_under_the_keyboard_cursor() {
        let (mut ui, chosen) = shown(Mode::Area);
        focus(&mut ui);

        key(&mut ui, Key::Left);
        key(&mut ui, Key::Enter);

        assert_eq!(*chosen.borrow(), Some(PaletteAction::Capture(Mode::Window)));
    }

    #[test]
    fn escape_closes_the_palette() {
        let (mut ui, chosen) = shown(Mode::Screen);
        focus(&mut ui);

        key(&mut ui, Key::Escape);

        assert_eq!(*chosen.borrow(), Some(PaletteAction::Close));
    }

    #[test]
    fn space_on_a_recording_mode_chooses_it_without_starting() {
        let (mut ui, chosen) = shown(Mode::Screen);
        focus(&mut ui);

        for _ in 0..3 {
            key(&mut ui, Key::Right);
        }
        key(&mut ui, Key::Space);
        for _ in 0..3 {
            key(&mut ui, Key::Right);
        }
        key(&mut ui, Key::Space);

        assert_eq!(
            *chosen.borrow(),
            Some(PaletteAction::Capture(Mode::RecordScreen))
        );
    }
}
