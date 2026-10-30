use jiff::Zoned;
use unbound_datetime::DateFormats;
use unbound_notify::Notification;
use unbound_toolkit::{
    Color, Image, Item, Palette, Paragraph, Point, Rect, Shape, TextAlign, TextStyle, TextSystem,
};

use crate::L10N;

pub const RADIUS: f32 = 16.0;
const PADDING: f32 = 12.0;
const ICON: f32 = 32.0;
const ICON_GAP: f32 = 10.0;
const LINE_GAP: f32 = 2.0;
const TITLE_SIZE: f32 = 13.0;
const BODY_SIZE: f32 = 13.0;
const SMALL_SIZE: f32 = 11.0;
const ACTION_HEIGHT: f32 = 26.0;
const ACTION_GAP: f32 = 6.0;
const CLOSE_SIZE: f32 = 18.0;
const SLIVER_STEP: f32 = 5.0;
const SLIVER_INSET: f32 = 8.0;
const MAX_ACTIONS: usize = 3;
pub const CLOSE_REACH: f32 = CLOSE_SIZE / 2.0 + 1.0;
pub const DEFAULT_ACTION: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Body,
    Close,
    Action(String),
}

pub struct Sender {
    pub name: String,
    pub icon: Option<Image>,
}

pub struct Card {
    pub id: u32,
    pub group: String,
    pub count: usize,
    pub height: f32,
    width: f32,
    icon: Option<Image>,
    initial: Option<Paragraph>,
    title: Paragraph,
    age: Paragraph,
    body: Option<(f32, Paragraph)>,
    more: Option<(f32, Paragraph)>,
    actions: Vec<(String, Paragraph, Rect)>,
}

pub fn age_label(arrived: &Zoned, now: &Zoned, dates: &DateFormats) -> String {
    let minutes = now.duration_since(arrived).as_secs().max(0) / 60;
    let days = arrived
        .date()
        .until(now.date())
        .map_or(0, |span| span.get_days());
    match (minutes, days) {
        (0, _) => L10N.tr("age-now"),
        (1..60, _) => L10N.tr_args("age-minutes", [("count", minutes.into())]),
        (_, ..=0) => L10N.tr_args("age-hours", [("count", (minutes / 60).into())]),
        (_, 1) => L10N.tr("age-yesterday"),
        (_, 2..7) => dates.weekday(arrived),
        _ => dates.date(arrived, arrived.year() != now.year()),
    }
}

fn fitted(
    text: &mut TextSystem,
    content: &str,
    style: TextStyle,
    width: f32,
    lines: usize,
) -> Paragraph {
    let whole = text.paragraph(content, style, Some(width), TextAlign::Start);
    if whole.line_count() <= lines {
        return whole;
    }
    let characters: Vec<char> = content.chars().collect();
    let (mut fits, mut too_long) = (0, characters.len());
    while too_long - fits > 1 {
        let middle = (fits + too_long) / 2;
        let candidate = format!(
            "{}…",
            characters[..middle].iter().collect::<String>().trim_end()
        );
        if text
            .paragraph(&candidate, style, Some(width), TextAlign::Start)
            .line_count()
            <= lines
        {
            fits = middle;
        } else {
            too_long = middle;
        }
    }
    let shortened = format!(
        "{}…",
        characters[..fits].iter().collect::<String>().trim_end()
    );
    text.paragraph(&shortened, style, Some(width), TextAlign::Start)
}

impl Card {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        notification: &Notification,
        group: &str,
        count: usize,
        sender: Sender,
        age: &str,
        width: f32,
        body_lines: usize,
        palette: &Palette,
        text: &mut TextSystem,
    ) -> Self {
        let column = PADDING + ICON + ICON_GAP;
        let text_width = width - column - PADDING;
        let age = text.paragraph(
            age,
            TextStyle::new(SMALL_SIZE, palette.ink2),
            None,
            TextAlign::Start,
        );
        let headline = if notification.summary.is_empty() {
            &sender.name
        } else {
            &notification.summary
        };
        let title = fitted(
            text,
            headline,
            TextStyle::new(TITLE_SIZE, palette.ink).weight(TextStyle::SEMIBOLD),
            text_width - age.width() - ICON_GAP,
            1,
        );
        let mut bottom = PADDING + title.height();
        let body = (!notification.body.is_empty()).then(|| {
            let paragraph = fitted(
                text,
                &notification.body,
                TextStyle::new(BODY_SIZE, palette.ink),
                text_width,
                body_lines,
            );
            let top = bottom + LINE_GAP;
            bottom = top + paragraph.height();
            (top, paragraph)
        });
        let more = (count > 1).then(|| {
            let paragraph = text.paragraph(
                &L10N.tr_args("more", [("count", (count - 1).into())]),
                TextStyle::new(SMALL_SIZE, palette.ink2),
                Some(text_width),
                TextAlign::Start,
            );
            let top = bottom + LINE_GAP;
            bottom = top + paragraph.height();
            (top, paragraph)
        });
        let mut height = bottom.max(PADDING + ICON) + PADDING;
        let shown: Vec<&(String, String)> = notification
            .actions
            .iter()
            .filter(|(key, _)| key != DEFAULT_ACTION)
            .take(MAX_ACTIONS)
            .collect();
        let mut actions = Vec::new();
        if !shown.is_empty() {
            let row = width - 2.0 * PADDING;
            let each = (row - ACTION_GAP * (shown.len() - 1) as f32) / shown.len() as f32;
            for (index, (key, label)) in shown.into_iter().enumerate() {
                let rect = Rect::new(
                    PADDING + index as f32 * (each + ACTION_GAP),
                    height - PADDING / 2.0,
                    each,
                    ACTION_HEIGHT,
                );
                let label = fitted(
                    text,
                    label,
                    TextStyle::new(BODY_SIZE, palette.ink).weight(TextStyle::MEDIUM),
                    each - PADDING,
                    1,
                );
                actions.push((key.clone(), label, rect));
            }
            height += ACTION_HEIGHT + PADDING / 2.0;
        }
        let initial = sender.icon.is_none().then(|| {
            let letter = sender.name.chars().next().unwrap_or('?').to_uppercase();
            text.paragraph(
                &letter.to_string(),
                TextStyle::new(15.0, Color::hex(0xFFFFFF)).weight(TextStyle::SEMIBOLD),
                None,
                TextAlign::Start,
            )
        });
        Self {
            id: notification.id,
            group: group.to_owned(),
            count,
            height: height.round(),
            width,
            icon: sender.icon,
            initial,
            title,
            age,
            body,
            more,
            actions,
        }
    }

    pub fn depth(&self) -> f32 {
        match self.count {
            0 | 1 => 0.0,
            2 => SLIVER_STEP,
            _ => 2.0 * SLIVER_STEP,
        }
    }

    pub fn bounds(&self, origin: Point) -> Rect {
        Rect::new(
            origin.x - CLOSE_REACH,
            origin.y - CLOSE_REACH,
            self.width + CLOSE_REACH,
            self.height + self.depth() + CLOSE_REACH,
        )
    }

    fn close_rect(&self) -> Rect {
        Rect::new(
            -CLOSE_SIZE / 2.0 + 2.0,
            -CLOSE_SIZE / 2.0 + 2.0,
            CLOSE_SIZE,
            CLOSE_SIZE,
        )
    }

    pub fn part_at(&self, point: Point, hovered: bool) -> Option<Part> {
        if hovered && contains(self.close_rect(), point) {
            return Some(Part::Close);
        }
        if !contains(
            Rect::new(0.0, 0.0, self.width, self.height + self.depth()),
            point,
        ) {
            return None;
        }
        Some(
            self.actions
                .iter()
                .find(|(_, _, rect)| contains(*rect, point))
                .map_or(Part::Body, |(key, _, _)| Part::Action(key.clone())),
        )
    }

    pub fn paint(
        &self,
        origin: Point,
        palette: &Palette,
        hovered: Option<&Part>,
        focused: bool,
        items: &mut Vec<Item>,
    ) {
        let mut card = Vec::new();
        let surface = palette.overlay;
        for level in (1..=(self.depth() / SLIVER_STEP) as usize).rev() {
            let step = level as f32;
            let sliver = Rect::new(
                SLIVER_INSET * step,
                self.height - RADIUS + SLIVER_STEP * step,
                self.width - 2.0 * SLIVER_INSET * step,
                RADIUS,
            );
            card.push(Item::Fill {
                shape: Shape::rounded(sliver, RADIUS),
                paint: surface.mix(palette.sheet, 0.35 * step).into(),
            });
            card.push(Item::Stroke {
                shape: Shape::rounded(sliver, RADIUS),
                width: 0.5,
                paint: palette.hairline.into(),
            });
        }
        let body = Shape::rounded(Rect::new(0.0, 0.0, self.width, self.height), RADIUS);
        card.push(Item::Shadow {
            shape: body,
            offset: Point::new(0.0, 4.0),
            blur_radius: 18.0,
            color: Color::hex(0).with_alpha(46),
        });
        card.push(Item::Fill {
            shape: body,
            paint: surface.into(),
        });
        card.push(Item::Stroke {
            shape: body,
            width: if focused { 2.0 } else { 0.5 },
            paint: if focused {
                palette.accent.into()
            } else {
                palette.hairline.into()
            },
        });
        let icon = Rect::new(PADDING, PADDING, ICON, ICON);
        match (&self.icon, &self.initial) {
            (Some(image), _) => card.push(Item::Image {
                rect: icon,
                image: image.clone(),
            }),
            (None, Some(initial)) => {
                card.push(Item::Fill {
                    shape: Shape::rounded(icon, 8.0),
                    paint: palette.accent.into(),
                });
                card.push(Item::Text {
                    origin: Point::new(
                        (icon.x + (ICON - initial.width()) / 2.0).round(),
                        (icon.y + (ICON - initial.height()) / 2.0).round(),
                    ),
                    paragraph: initial.clone(),
                });
            }
            (None, None) => {}
        }
        let column = PADDING + ICON + ICON_GAP;
        card.push(Item::Text {
            origin: Point::new(column, PADDING),
            paragraph: self.title.clone(),
        });
        card.push(Item::Text {
            origin: Point::new(
                (self.width - PADDING - self.age.width()).round(),
                PADDING + (self.title.height() - self.age.height()) / 2.0,
            ),
            paragraph: self.age.clone(),
        });
        for (top, paragraph) in self.body.iter().chain(&self.more) {
            card.push(Item::Text {
                origin: Point::new(column, top.round()),
                paragraph: paragraph.clone(),
            });
        }
        for (key, label, rect) in &self.actions {
            let pressed_look = matches!(hovered, Some(Part::Action(hovered)) if hovered == key);
            card.push(Item::Fill {
                shape: Shape::rounded(*rect, 7.0),
                paint: palette
                    .ink
                    .with_alpha(if pressed_look { 36 } else { 18 })
                    .into(),
            });
            card.push(Item::Text {
                origin: Point::new(
                    (rect.x + (rect.width - label.width()) / 2.0).round(),
                    (rect.y + (rect.height - label.height()) / 2.0).round(),
                ),
                paragraph: label.clone(),
            });
        }
        if hovered.is_some() || focused {
            let close = self.close_rect();
            let over = hovered == Some(&Part::Close);
            card.push(Item::Shadow {
                shape: Shape::rounded(close, CLOSE_SIZE / 2.0),
                offset: Point::new(0.0, 1.0),
                blur_radius: 4.0,
                color: Color::hex(0).with_alpha(40),
            });
            card.push(Item::Fill {
                shape: Shape::rounded(close, CLOSE_SIZE / 2.0),
                paint: if over {
                    palette.sheet.mix(palette.ink2, 0.25)
                } else {
                    palette.overlay
                }
                .into(),
            });
            card.push(Item::Stroke {
                shape: Shape::rounded(close, CLOSE_SIZE / 2.0),
                width: 0.5,
                paint: palette.hairline.into(),
            });
            let center = Point::new(close.x + CLOSE_SIZE / 2.0, close.y + CLOSE_SIZE / 2.0);
            let arm = 3.5;
            for (dx, dy) in [(arm, arm), (arm, -arm)] {
                card.push(Item::Line {
                    from: Point::new(center.x - dx, center.y - dy),
                    to: Point::new(center.x + dx, center.y + dy),
                    width: 1.5,
                    paint: palette.ink2.into(),
                });
            }
        }
        items.push(Item::Translate {
            offset: origin,
            items: card,
        });
    }
}

pub fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.y >= rect.y
        && point.x < rect.x + rect.width
        && point.y < rect.y + rect.height
}
