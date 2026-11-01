use accesskit::{Action, ActionRequest, Node, Role};
use bande_toolkit::ui::{Context, Event, Size, Widget, WidgetId};
use bande_toolkit::{Color, Item, Rect, Shape};

use crate::L10N;
use crate::rows::{SKIN_TONES, tone_name};
use crate::view::Shared;

const SWATCH: f32 = 14.0;
const SPACING: f32 = 8.0;
const RING: f32 = 2.0;
const COLORS: [Color; 6] = [
    Color::hex(0xFFCC4D),
    Color::hex(0xF7DECE),
    Color::hex(0xF3D2A2),
    Color::hex(0xD5AB88),
    Color::hex(0xAF7E57),
    Color::hex(0x7C533E),
];

pub const TONES_WIDTH: f32 = 6.0 * SWATCH + 5.0 * SPACING + 2.0 * RING;

pub struct Tones {
    id: WidgetId,
    state: Shared,
    frame: Rect,
}

impl Tones {
    pub fn new(state: Shared) -> Self {
        Self {
            id: WidgetId::next(),
            state,
            frame: Rect::default(),
        }
    }

    fn swatch(&self, index: usize) -> Rect {
        Rect::new(
            self.frame.x + RING + index as f32 * (SWATCH + SPACING),
            self.frame.y + ((self.frame.height - SWATCH) / 2.0).round(),
            SWATCH,
            SWATCH,
        )
    }

    fn choose(&mut self, index: usize) -> bool {
        let Some(tone) = SKIN_TONES.get(index) else {
            return false;
        };
        self.state.borrow_mut().model.set_skin_tone(*tone);
        true
    }
}

impl Widget for Tones {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, _available: Size) -> Size {
        Size::new(TONES_WIDTH, SWATCH + 2.0 * RING)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        if self.frame.width <= 0.0 {
            return;
        }
        let chosen = self.state.borrow().model.skin_tone;
        for (index, (tone, color)) in SKIN_TONES.iter().zip(COLORS).enumerate() {
            let swatch = self.swatch(index);
            items.push(Item::Fill {
                shape: Shape::rounded(swatch, SWATCH / 2.0),
                paint: color.into(),
            });
            if *tone == chosen {
                items.push(Item::Stroke {
                    shape: Shape::rounded(swatch.inset(-RING), SWATCH / 2.0 + RING),
                    width: RING,
                    paint: cx.palette.accent_text.into(),
                });
            }
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::RadioGroup);
        node.set_label(L10N.tr("skin-tone"));
        node.set_value(tone_name(self.state.borrow().model.skin_tone));
        node.add_action(Action::Increment);
        node.add_action(Action::Decrement);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                match (0..SKIN_TONES.len()).find(|index| {
                    self.swatch(*index)
                        .inset(-SPACING / 2.0)
                        .contains(pointer.position)
                }) {
                    Some(index) => self.choose(index),
                    None => false,
                }
            }
            _ => false,
        }
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        let chosen = self.state.borrow().model.skin_tone;
        let current = SKIN_TONES
            .iter()
            .position(|tone| *tone == chosen)
            .unwrap_or(0);
        match request.action {
            Action::Increment => self.choose((current + 1).min(SKIN_TONES.len() - 1)),
            Action::Decrement => self.choose(current.saturating_sub(1)),
            _ => false,
        }
    }
}
