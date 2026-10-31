use jiff::civil::Date;
use unbound_datetime::{DateFormats, Month};
use unbound_toolkit::ui::focus_ring_for;
use unbound_toolkit::{
    Item, Palette, Paragraph, Point, Rect, Shape, Spacing, TextAlign, TextRole, TextStyle,
    TextSystem,
};

use crate::card::{RADIUS, surface_with_edge};

const PADDING: f32 = Spacing::M as f32;
const HEADER: f32 = 28.0;
const WEEKDAYS: f32 = 22.0;
const DAY: f32 = 28.0;
const TODAY: f32 = 24.0;
const WEEK_COLUMN: f32 = 26.0;
const ARROW: f32 = 24.0;
const ARROW_ARM: f32 = 3.5;
const HOVER_INK: f32 = 0.08;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    Previous,
    Next,
}

impl Turn {
    pub fn months(self) -> i32 {
        match self {
            Self::Previous => -1,
            Self::Next => 1,
        }
    }
}

pub struct Calendar {
    pub height: f32,
    width: f32,
    title: Paragraph,
    weekdays: Vec<(f32, Paragraph)>,
    weeks: Vec<(f32, Paragraph)>,
    days: Vec<(Rect, Paragraph, bool)>,
    previous: Rect,
    next: Rect,
}

impl Calendar {
    pub fn new(
        month: &Month,
        today: Date,
        dates: &DateFormats,
        width: f32,
        palette: &Palette,
        text: &mut TextSystem,
    ) -> Self {
        let title = text.paragraph(
            &dates.month_title(month.first),
            TextStyle::role(TextRole::BodyStrong, palette.ink),
            None,
            TextAlign::Start,
        );
        let numbered = month.weeks.iter().any(|week| week.number.is_some());
        let left = PADDING + if numbered { WEEK_COLUMN } else { 0.0 };
        let column = (width - left - PADDING) / 7.0;
        let column_x = |index: usize| left + column * index as f32;
        let label = |text: &mut TextSystem, content: &str, color, role| {
            text.paragraph(
                content,
                TextStyle::role(role, color).tabular(),
                None,
                TextAlign::Start,
            )
        };
        let weekdays = month
            .weeks
            .first()
            .map(|week| {
                week.days
                    .iter()
                    .enumerate()
                    .map(|(index, day)| {
                        let paragraph = label(
                            text,
                            &dates.short_weekday(*day),
                            palette.ink3,
                            TextRole::Label,
                        );
                        (
                            column_x(index) + (column - paragraph.width()) / 2.0,
                            paragraph,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let grid_top = PADDING + HEADER + WEEKDAYS;
        let mut weeks = Vec::new();
        let mut days = Vec::new();
        for (row, week) in month.weeks.iter().enumerate() {
            let top = grid_top + DAY * row as f32;
            if let Some(number) = week.number {
                weeks.push((
                    top,
                    label(text, &number.to_string(), palette.ink3, TextRole::Callout),
                ));
            }
            for (index, day) in week.days.iter().enumerate() {
                let is_today = *day == today;
                let color = match (is_today, month.contains(*day)) {
                    (true, _) => palette.on_accent,
                    (false, true) => palette.ink,
                    (false, false) => palette.ink3,
                };
                let role = if is_today {
                    TextRole::BodyStrong
                } else {
                    TextRole::Body
                };
                days.push((
                    Rect::new(column_x(index), top, column, DAY),
                    label(text, &day.day().to_string(), color, role),
                    is_today,
                ));
            }
        }
        let arrow_top = PADDING + (HEADER - ARROW) / 2.0;
        let next = Rect::new(
            width - PADDING - ARROW + Spacing::XS as f32,
            arrow_top,
            ARROW,
            ARROW,
        );
        let previous = Rect::new(next.x - ARROW, arrow_top, ARROW, ARROW);
        Self {
            height: grid_top + DAY * month.weeks.len() as f32 + PADDING,
            width,
            title,
            weekdays,
            weeks,
            days,
            previous,
            next,
        }
    }

    pub fn turn_at(&self, point: Point) -> Option<Turn> {
        [(self.previous, Turn::Previous), (self.next, Turn::Next)]
            .into_iter()
            .find(|(rect, _)| rect.contains(point))
            .map(|(_, turn)| turn)
    }

    pub fn paint(
        &self,
        origin: Point,
        palette: &Palette,
        hovered: Option<Turn>,
        focused: Option<Turn>,
        items: &mut Vec<Item>,
    ) {
        let mut card = Vec::new();
        surface_with_edge(
            palette,
            Shape::rounded(Rect::new(0.0, 0.0, self.width, self.height), RADIUS),
            palette.overlay,
            &mut card,
        );
        card.push(Item::Text {
            origin: Point::new(
                PADDING + Spacing::XS as f32,
                (PADDING + (HEADER - self.title.height()) / 2.0).round(),
            ),
            paragraph: self.title.clone(),
        });
        for (rect, turn) in [(self.previous, Turn::Previous), (self.next, Turn::Next)] {
            arrow(
                &mut card,
                palette,
                rect,
                turn,
                hovered == Some(turn),
                focused == Some(turn),
            );
        }
        let weekday_top = PADDING + HEADER;
        for (x, paragraph) in &self.weekdays {
            card.push(Item::Text {
                origin: Point::new(
                    x.round(),
                    (weekday_top + (WEEKDAYS - paragraph.height()) / 2.0).round(),
                ),
                paragraph: paragraph.clone(),
            });
        }
        for (top, paragraph) in &self.weeks {
            card.push(Item::Text {
                origin: Point::new(
                    (PADDING + (WEEK_COLUMN - paragraph.width()) / 2.0).round(),
                    (top + (DAY - paragraph.height()) / 2.0).round(),
                ),
                paragraph: paragraph.clone(),
            });
        }
        for (rect, paragraph, today) in &self.days {
            let center = Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
            if *today {
                card.push(Item::Fill {
                    shape: Shape::rounded(
                        Rect::new(center.x - TODAY / 2.0, center.y - TODAY / 2.0, TODAY, TODAY),
                        TODAY / 2.0,
                    ),
                    paint: palette.accent.into(),
                });
            }
            card.push(Item::Text {
                origin: Point::new(
                    (center.x - paragraph.width() / 2.0).round(),
                    (center.y - paragraph.height() / 2.0).round(),
                ),
                paragraph: paragraph.clone(),
            });
        }
        items.push(Item::Translate {
            offset: origin,
            items: card,
        });
    }
}

fn arrow(
    items: &mut Vec<Item>,
    palette: &Palette,
    rect: Rect,
    turn: Turn,
    hovered: bool,
    focused: bool,
) {
    let shape = Shape::rounded(rect, rect.height / 2.0);
    if hovered {
        items.push(Item::Fill {
            shape,
            paint: palette.overlay.mix(palette.ink, HOVER_INK).into(),
        });
    }
    if focused {
        focus_ring_for(palette, shape, items);
    }
    let center = Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    let direction = match turn {
        Turn::Previous => -1.0,
        Turn::Next => 1.0,
    };
    let tip = Point::new(center.x + direction * ARROW_ARM / 2.0, center.y);
    for dy in [-ARROW_ARM, ARROW_ARM] {
        items.push(Item::Line {
            from: Point::new(tip.x - direction * ARROW_ARM, center.y + dy),
            to: tip,
            width: 1.5,
            paint: if hovered { palette.ink } else { palette.ink2 }.into(),
        });
    }
}
