use std::time::{Duration, Instant};

use unbound_graphics::{
    Color, Item, Palette, Point, Radius, Rect, Shape, TextAlign, TextStyle, TextSystem,
};

use unbound_spring::Spring;

use super::{Animated, Size, WidgetId};

pub const TOOLTIP_DELAY: Duration = Duration::from_millis(700);
const WARM: Duration = Duration::from_millis(600);
const POINTER_HEIGHT: f32 = 20.0;
const FONT_SIZE: f32 = 12.0;
const MAX_WIDTH: f32 = 280.0;
const HORIZONTAL_PADDING: f32 = 10.0;
const VERTICAL_PADDING: f32 = 5.0;
const MARGIN: f32 = 4.0;
const SHADOW: f32 = 0.18;

struct Pending {
    owner: WidgetId,
    since: Instant,
    avoid: Rect,
}

#[derive(Default)]
pub(crate) struct Tooltips {
    pending: Option<Pending>,
    dismissed: Option<WidgetId>,
    warm_until: Option<Instant>,
}

impl Tooltips {
    fn start(&mut self, now: Instant, owner: Option<WidgetId>, avoid: Rect) {
        if self.pending.as_ref().map(|pending| pending.owner) == owner {
            if let Some(pending) = &mut self.pending
                && now < pending.since + TOOLTIP_DELAY
            {
                pending.avoid = avoid;
            }
            return;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| now >= pending.since + TOOLTIP_DELAY)
        {
            self.warm_until = Some(now + WARM);
        }
        let warm = self.warm_until.is_some_and(|until| now < until);
        self.pending = owner.map(|owner| Pending {
            owner,
            since: match warm {
                true => now.checked_sub(TOOLTIP_DELAY).unwrap_or(now),
                false => now,
            },
            avoid,
        });
    }

    pub(crate) fn hover(&mut self, now: Instant, owner: Option<WidgetId>, pointer: Option<Point>) {
        if self.dismissed.is_some() && self.dismissed != owner {
            self.dismissed = None;
        }
        if self.dismissed.is_some() {
            return;
        }
        let avoid = pointer.map_or(Rect::default(), |point| {
            Rect::new(point.x, point.y, 0.0, POINTER_HEIGHT)
        });
        self.start(now, owner, avoid);
    }

    pub(crate) fn focus(&mut self, now: Instant, owner: Option<WidgetId>, frame: Rect) {
        self.dismissed = None;
        self.start(now, owner, frame);
    }

    pub(crate) fn dismiss(&mut self) {
        if let Some(pending) = self.pending.take() {
            self.dismissed = Some(pending.owner);
        }
    }

    pub(crate) fn deadline(&self, now: Instant) -> Option<Instant> {
        self.pending
            .as_ref()
            .map(|pending| pending.since + TOOLTIP_DELAY)
            .filter(|shown| *shown > now)
    }

    pub(crate) fn owner(&self) -> Option<WidgetId> {
        self.pending.as_ref().map(|pending| pending.owner)
    }

    pub(crate) fn paint(
        &self,
        now: Instant,
        text: &str,
        system: &mut TextSystem,
        palette: &Palette,
        window: Size,
        items: &mut Vec<Item>,
    ) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        let shown = pending.since + TOOLTIP_DELAY;
        if now < shown {
            return false;
        }
        let mut fade = Animated::new(0.0);
        fade.animate_at(shown, false, 1.0, Spring::FADE);
        let (alpha, fading) = fade.value_at(now);
        let paragraph = system.paragraph(
            text,
            TextStyle::new(FONT_SIZE, palette.chassis).weight(TextStyle::MEDIUM),
            Some(MAX_WIDTH),
            TextAlign::Start,
        );
        let size = Size::new(
            (paragraph.width() + 2.0 * HORIZONTAL_PADDING).ceil(),
            (paragraph.height() + 2.0 * VERTICAL_PADDING).ceil(),
        );
        let avoid = pending.avoid;
        let below = avoid.y + avoid.height + 2.0;
        let y = match below + size.height + MARGIN <= window.height {
            true => below,
            false => (avoid.y - size.height - 2.0).max(MARGIN),
        };
        let x = avoid.x.min(window.width - size.width - MARGIN).max(MARGIN);
        let frame = Rect::new(x.round(), y.round(), size.width, size.height);
        let shape = Shape::rounded(frame, Radius::S);
        items.push(Item::Opacity {
            alpha,
            items: vec![
                Item::Shadow {
                    shape,
                    offset: Point::new(0.0, 2.0),
                    blur_radius: 6.0,
                    color: Color::BLACK.with_opacity(SHADOW),
                },
                Item::Fill {
                    shape,
                    paint: palette.ink.into(),
                },
                Item::Text {
                    origin: Point::new(frame.x + HORIZONTAL_PADDING, frame.y + VERTICAL_PADDING),
                    paragraph,
                },
            ],
        });
        fading
    }
}
