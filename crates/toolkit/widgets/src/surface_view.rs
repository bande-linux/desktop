use std::time::{Duration, Instant};

use accesskit::{Node, Role};
use bande_graphics::{
    Captured, Cursor, Item, Paragraph, Point, Radius, Rect, Shape, Spacing, SurfaceId,
    SurfaceScaling, TextAlign, TextRole, TextStyle, surface_area,
};

use super::{
    Animated, Context, Event, L10N, PointerButton, PointerCapture, ScrollSource, Size, Spring,
    Widget, WidgetId, shortcut_label,
};

pub const RELEASE_CHORD: &str = "Ctrl+Alt";
const CONTROL_KEYS: [u32; 2] = [29, 97];
const ALT_KEYS: [u32; 2] = [56, 100];
const HINT_SHOWN: Duration = Duration::from_secs(3);
const HINT_PADDING_X: f32 = Spacing::M as f32;
const HINT_PADDING_Y: f32 = Spacing::S as f32;
const HINT_TOP: f32 = Spacing::L as f32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SurfaceInput {
    PointerMoved(Point),
    PointerLeft,
    Button {
        button: PointerButton,
        pressed: bool,
    },
    Scroll {
        delta: Point,
        source: ScrollSource,
    },
    Motion {
        delta: Point,
        unaccelerated: Point,
    },
    Key {
        code: u32,
        pressed: bool,
    },
    Captured(bool),
}

pub struct SurfaceView {
    id: WidgetId,
    surface: SurfaceId,
    label: String,
    size: (u32, u32),
    scaling: SurfaceScaling,
    capture: Option<PointerCapture>,
    cursor: Option<Cursor>,
    captured: bool,
    inside: bool,
    held_keys: Vec<u32>,
    held_buttons: Vec<PointerButton>,
    hint: Animated,
    hint_until: Option<Instant>,
    hint_text: Option<Paragraph>,
    frame: Rect,
    on_input: Box<dyn FnMut(SurfaceInput)>,
}

impl SurfaceView {
    pub fn new(
        surface: SurfaceId,
        label: &str,
        on_input: impl FnMut(SurfaceInput) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            surface,
            label: label.to_owned(),
            size: (0, 0),
            scaling: SurfaceScaling::Fit,
            capture: None,
            cursor: None,
            captured: false,
            inside: false,
            held_keys: Vec::new(),
            held_buttons: Vec::new(),
            hint: Animated::new(0.0),
            hint_until: None,
            hint_text: None,
            frame: Rect::default(),
            on_input: Box::new(on_input),
        }
    }

    pub fn scaling(mut self, scaling: SurfaceScaling) -> Self {
        self.scaling = scaling;
        self
    }

    pub fn capture_on_click(mut self, pointer: PointerCapture) -> Self {
        self.capture = Some(pointer);
        self
    }

    pub fn set_size(&mut self, width: u32, height: u32) {
        self.size = (width, height);
    }

    pub fn set_scaling(&mut self, scaling: SurfaceScaling) {
        self.scaling = scaling;
    }

    pub fn set_cursor(&mut self, cursor: Cursor) {
        self.cursor = Some(cursor);
    }

    pub fn is_captured(&self) -> bool {
        self.captured
    }

    fn area(&self, cx: &Context) -> Option<Rect> {
        surface_area(self.frame, self.size, self.scaling, cx.display_scale)
    }

    fn content_point(&self, area: Rect, point: Point) -> Point {
        let along = |at: f32, start: f32, length: f32, pixels: u32| {
            ((at - start) * pixels as f32 / length).clamp(0.0, pixels.saturating_sub(1) as f32)
        };
        Point::new(
            along(point.x, area.x, area.width, self.size.0),
            along(point.y, area.y, area.height, self.size.1),
        )
    }

    fn pointer_at(&mut self, cx: &Context, point: Point) -> bool {
        let area = self.area(cx).filter(|area| area.contains(point));
        match area {
            Some(area) => {
                self.inside = true;
                let moved = SurfaceInput::PointerMoved(self.content_point(area, point));
                (self.on_input)(moved);
            }
            None if std::mem::take(&mut self.inside) => (self.on_input)(SurfaceInput::PointerLeft),
            None => {}
        }
        self.inside
    }

    fn start_capture(&mut self, cx: &mut Context, area: Rect, pointer: PointerCapture) {
        cx.capture_input(area, pointer);
        self.captured = true;
        (self.on_input)(SurfaceInput::Captured(true));
        self.hint_until = Some(cx.now() + HINT_SHOWN);
        cx.animate(&mut self.hint, 1.0, Spring::FADE);
    }

    fn end_capture(&mut self, cx: &Context) {
        for code in std::mem::take(&mut self.held_keys) {
            (self.on_input)(SurfaceInput::Key {
                code,
                pressed: false,
            });
        }
        self.captured = false;
        self.hint_until = None;
        cx.animate(&mut self.hint, 0.0, Spring::FADE);
        (self.on_input)(SurfaceInput::Captured(false));
    }

    fn key(&mut self, cx: &mut Context, code: u32, pressed: bool) {
        match pressed {
            true if !self.held_keys.contains(&code) => self.held_keys.push(code),
            true => {}
            false => self.held_keys.retain(|held| *held != code),
        }
        (self.on_input)(SurfaceInput::Key { code, pressed });
        let held = |keys: [u32; 2]| keys.iter().any(|key| self.held_keys.contains(key));
        if pressed && held(CONTROL_KEYS) && held(ALT_KEYS) {
            cx.release_input();
            self.end_capture(cx);
        }
    }
}

impl Widget for SurfaceView {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        if let Some(until) = self.hint_until {
            match until <= cx.now() {
                true => {
                    self.hint_until = None;
                    cx.animate(&mut self.hint, 0.0, Spring::FADE);
                }
                false => cx.wake_at(until),
            }
        }
        if self.hint_text.is_none() && (self.hint_until.is_some() || cx.sample(&self.hint) > 0.0) {
            let text = L10N.tr_args(
                "release-capture",
                [("chord", shortcut_label(RELEASE_CHORD).into())],
            );
            let style = TextStyle::role(TextRole::Callout, cx.palette.chassis);
            self.hint_text = Some(cx.text.paragraph(&text, style, None, TextAlign::Start));
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        items.push(Item::Surface {
            id: self.surface,
            frame: self.frame,
            size: self.size,
            scaling: self.scaling,
        });
        let shown = cx.sample(&self.hint);
        let (Some(text), true) = (&self.hint_text, shown > 0.0) else {
            return;
        };
        let pill = Rect::new(
            (self.frame.x + (self.frame.width - text.width()) / 2.0 - HINT_PADDING_X).round(),
            self.frame.y + HINT_TOP,
            text.width().ceil() + 2.0 * HINT_PADDING_X,
            text.height().ceil() + 2.0 * HINT_PADDING_Y,
        );
        items.push(Item::Opacity {
            alpha: shown,
            items: vec![
                Item::Fill {
                    shape: Shape::rounded(pill, Radius::S),
                    paint: cx.palette.ink.into(),
                },
                Item::Text {
                    origin: Point::new(pill.x + HINT_PADDING_X, pill.y + HINT_PADDING_Y),
                    paragraph: text.clone(),
                },
            ],
        });
    }

    fn cursor(&self, point: Point) -> Option<Cursor> {
        self.cursor.filter(|_| self.frame.contains(point))
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Canvas);
        node.set_label(self.label.clone());
        if self.captured {
            node.set_description(L10N.tr_args(
                "release-capture",
                [("chord", shortcut_label(RELEASE_CHORD).into())],
            ));
        }
        node
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerEnter(pointer) | Event::PointerMove(pointer) => {
                self.pointer_at(cx, pointer.position);
                false
            }
            Event::PointerLeave => {
                if std::mem::take(&mut self.inside) {
                    (self.on_input)(SurfaceInput::PointerLeft);
                }
                false
            }
            Event::PointerDown(pointer) => {
                if !self.pointer_at(cx, pointer.position) && !self.captured {
                    return false;
                }
                if let (false, Some(mode), Some(area)) =
                    (self.captured, self.capture, self.area(cx))
                {
                    self.start_capture(cx, area, mode);
                }
                self.held_buttons.push(pointer.button);
                (self.on_input)(SurfaceInput::Button {
                    button: pointer.button,
                    pressed: true,
                });
                true
            }
            Event::PointerUp(pointer) => {
                let Some(index) = self
                    .held_buttons
                    .iter()
                    .position(|button| *button == pointer.button)
                else {
                    return false;
                };
                self.held_buttons.remove(index);
                (self.on_input)(SurfaceInput::Button {
                    button: pointer.button,
                    pressed: false,
                });
                false
            }
            Event::Scroll(scroll) if self.captured || self.inside => {
                (self.on_input)(SurfaceInput::Scroll {
                    delta: scroll.delta,
                    source: scroll.source,
                });
                true
            }
            Event::Captured(Captured::Key { code, pressed }) => {
                self.key(cx, *code, *pressed);
                !self.captured
            }
            Event::Captured(Captured::Motion {
                delta,
                unaccelerated,
            }) => {
                (self.on_input)(SurfaceInput::Motion {
                    delta: *delta,
                    unaccelerated: *unaccelerated,
                });
                false
            }
            Event::Captured(Captured::Ended) if self.captured => {
                self.end_capture(cx);
                true
            }
            _ => false,
        }
    }
}
