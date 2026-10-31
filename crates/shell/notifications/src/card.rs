use jiff::Zoned;
use unbound_datetime::DateFormats;
use unbound_notify::Notification;
use unbound_toolkit::ui::focus_ring_for;
use unbound_toolkit::{
    Color, ControlSize, Elevation, Image, Item, Palette, Paragraph, Point, Radius, Rect, Scheme,
    Shape, Spacing, TextAlign, TextRole, TextStyle, TextSystem,
};

use crate::L10N;

pub const RADIUS: f32 = Radius::XL;
const PADDING_X: f32 = Spacing::L as f32;
const PADDING_Y: f32 = Spacing::M as f32;
const HEADER: f32 = 18.0;
const HEADER_GAP: f32 = Spacing::S as f32;
const ICON: f32 = 18.0;
const ICON_GAP: f32 = Spacing::S as f32;
const LINE_GAP: f32 = Spacing::XXS as f32;
const TITLE_SIZE: f32 = 13.5;
const ACTION_HEIGHT: f32 = ControlSize::Regular.height();
const ACTION_INSET: f32 = Spacing::S as f32;
const ACTION_GAP: f32 = Spacing::XS as f32;
const CLOSE_SIZE: f32 = 18.0;
const SLIVER_STEP: f32 = 5.0;
const SLIVER_INSET: f32 = 8.0;
const SLIVER_SHADE: f32 = 0.03;
const HOVER_INK: f32 = 0.08;
const MAX_ACTIONS: usize = 3;
const PICTURE: f32 = 40.0;
const PICTURE_GAP: f32 = Spacing::M as f32;
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
    pub picture: Option<Image>,
}

pub struct Card {
    pub id: u32,
    pub group: String,
    pub count: usize,
    pub height: f32,
    width: f32,
    icon: Option<Image>,
    picture: Option<(Rect, Image)>,
    initial: Option<Paragraph>,
    app: Paragraph,
    headline: Option<(f32, Paragraph)>,
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
        let text_width = width - 2.0 * PADDING_X;
        let age = text.paragraph(
            age,
            TextStyle::role(TextRole::Callout, palette.ink3).tabular(),
            None,
            TextAlign::Start,
        );
        let app = fitted(
            text,
            &sender.name,
            TextStyle::role(TextRole::Label, palette.ink3),
            text_width - ICON - 2.0 * ICON_GAP - age.width(),
            1,
        );
        let mut bottom = PADDING_Y + HEADER + HEADER_GAP;
        let picture = sender.picture.map(|image| {
            let rect = Rect::new(width - PADDING_X - PICTURE, bottom, PICTURE, PICTURE);
            (rect, image)
        });
        let content_width = match picture {
            Some(_) => text_width - PICTURE - PICTURE_GAP,
            None => text_width,
        };
        let headline = (!notification.summary.is_empty()).then(|| {
            let paragraph = fitted(
                text,
                &notification.summary,
                TextStyle::new(TITLE_SIZE, palette.ink).weight(TextStyle::SEMIBOLD),
                content_width,
                1,
            );
            let top = bottom;
            bottom = top + paragraph.height();
            (top, paragraph)
        });
        let body = (!notification.body.is_empty()).then(|| {
            let paragraph = fitted(
                text,
                &notification.body,
                TextStyle::role(TextRole::Body, palette.ink2),
                content_width,
                body_lines,
            );
            let top = bottom;
            bottom = top + paragraph.height();
            (top, paragraph)
        });
        let more = (count > 1).then(|| {
            let paragraph = text.paragraph(
                &L10N.tr_args("more", [("count", (count - 1).into())]),
                TextStyle::role(TextRole::Callout, palette.ink3),
                Some(text_width),
                TextAlign::Start,
            );
            let top = bottom + LINE_GAP;
            bottom = top + paragraph.height();
            (top, paragraph)
        });
        if let Some((rect, _)) = &picture {
            bottom = bottom.max(rect.y + rect.height);
        }
        let mut height = bottom + PADDING_Y;
        let shown: Vec<&(String, String)> = notification
            .actions
            .iter()
            .filter(|(key, _)| key != DEFAULT_ACTION)
            .take(MAX_ACTIONS)
            .collect();
        let mut actions = Vec::new();
        if !shown.is_empty() {
            let each = (text_width + 2.0 * ACTION_INSET - ACTION_GAP * (shown.len() - 1) as f32)
                / shown.len() as f32;
            let mut left = PADDING_X - ACTION_INSET;
            for (key, label) in shown {
                let label = fitted(
                    text,
                    label,
                    TextStyle::role(TextRole::BodyStrong, palette.accent_text),
                    each - 2.0 * ACTION_INSET,
                    1,
                );
                let rect = Rect::new(
                    left,
                    height - PADDING_Y + ACTION_GAP,
                    (label.width() + 2.0 * ACTION_INSET).ceil(),
                    ACTION_HEIGHT,
                );
                left += rect.width + ACTION_GAP;
                actions.push((key.clone(), label, rect));
            }
            height += ACTION_HEIGHT + ACTION_GAP;
        }
        let initial = sender.icon.is_none().then(|| {
            let letter = sender.name.chars().next().unwrap_or('?').to_uppercase();
            text.paragraph(
                &letter.to_string(),
                TextStyle::role(TextRole::Label, palette.on_accent),
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
            picture,
            initial,
            app,
            headline,
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
        elevated: bool,
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
            surface_with_edge(
                palette,
                Shape::rounded(sliver, RADIUS),
                surface.mix(palette.ink, SLIVER_SHADE * step),
                &mut card,
            );
        }
        let body = Shape::rounded(Rect::new(0.0, 0.0, self.width, self.height), RADIUS);
        if elevated {
            card.extend(Item::shadows(
                body,
                Elevation::Overlay,
                palette.scheme,
                true,
            ));
        }
        surface_with_edge(palette, body, surface, &mut card);
        if focused {
            focus_ring_for(palette, body, &mut card);
        }
        let icon = Rect::new(PADDING_X, PADDING_Y + (HEADER - ICON) / 2.0, ICON, ICON);
        match (&self.icon, &self.initial) {
            (Some(image), _) => card.push(Item::Image {
                rect: icon,
                image: image.clone(),
            }),
            (None, Some(initial)) => {
                card.push(Item::Fill {
                    shape: Shape::rounded(icon, Radius::XS),
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
        card.push(Item::Text {
            origin: Point::new(
                icon.x + ICON + ICON_GAP,
                (PADDING_Y + (HEADER - self.app.height()) / 2.0).round(),
            ),
            paragraph: self.app.clone(),
        });
        card.push(Item::Text {
            origin: Point::new(
                (self.width - PADDING_X - self.age.width()).round(),
                (PADDING_Y + (HEADER - self.age.height()) / 2.0).round(),
            ),
            paragraph: self.age.clone(),
        });
        if let Some((rect, image)) = &self.picture {
            picture(*rect, image, palette, &mut card);
        }
        for (top, paragraph) in self.headline.iter().chain(&self.body).chain(&self.more) {
            card.push(Item::Text {
                origin: Point::new(PADDING_X, top.round()),
                paragraph: paragraph.clone(),
            });
        }
        for (key, label, rect) in &self.actions {
            if matches!(hovered, Some(Part::Action(hovered)) if hovered == key) {
                card.push(Item::Fill {
                    shape: Shape::rounded(*rect, Radius::S),
                    paint: surface.mix(palette.ink, HOVER_INK).into(),
                });
            }
            card.push(Item::Text {
                origin: Point::new(
                    (rect.x + (rect.width - label.width()) / 2.0).round(),
                    (rect.y + (rect.height - label.height()) / 2.0).round(),
                ),
                paragraph: label.clone(),
            });
        }
        if hovered.is_some() || focused {
            let close = Shape::rounded(self.close_rect(), CLOSE_SIZE / 2.0);
            let over = hovered == Some(&Part::Close);
            card.extend(Item::shadows(
                close,
                Elevation::Overlay,
                palette.scheme,
                true,
            ));
            card.push(Item::Fill {
                shape: close,
                paint: if over {
                    surface.mix(palette.ink, HOVER_INK)
                } else {
                    surface
                }
                .into(),
            });
            card.push(Item::Stroke {
                shape: Shape::rounded(close.rect.inset(0.5), CLOSE_SIZE / 2.0 - 0.5),
                width: 1.0,
                paint: palette.hairline.into(),
            });
            let center = Point::new(
                close.rect.x + CLOSE_SIZE / 2.0,
                close.rect.y + CLOSE_SIZE / 2.0,
            );
            let arm = 3.5;
            for (dx, dy) in [(arm, arm), (arm, -arm)] {
                card.push(Item::Line {
                    from: Point::new(center.x - dx, center.y - dy),
                    to: Point::new(center.x + dx, center.y + dy),
                    width: 1.5,
                    paint: if over { palette.ink } else { palette.ink2 }.into(),
                });
            }
        }
        items.push(Item::Translate {
            offset: origin,
            items: card,
        });
    }
}

fn picture(frame: Rect, image: &Image, palette: &Palette, items: &mut Vec<Item>) {
    let (width, height) = (image.width() as f32, image.height() as f32);
    let scale = (frame.width / width).max(frame.height / height);
    let (fill_width, fill_height) = (width * scale, height * scale);
    let shape = Shape::rounded(frame, Radius::S);
    items.push(Item::Clip {
        shape,
        items: vec![Item::Image {
            rect: Rect::new(
                frame.x + (frame.width - fill_width) / 2.0,
                frame.y + (frame.height - fill_height) / 2.0,
                fill_width,
                fill_height,
            ),
            image: image.clone(),
        }],
    });
    items.push(Item::Stroke {
        shape: Shape::rounded(frame.inset(0.5), Radius::S - 0.5),
        width: 1.0,
        paint: palette.hairline.into(),
    });
}

pub(crate) fn surface_with_edge(
    palette: &Palette,
    shape: Shape,
    fill: Color,
    items: &mut Vec<Item>,
) {
    items.push(Item::Fill {
        shape,
        paint: fill.into(),
    });
    if palette.scheme == Scheme::Dark {
        items.push(Item::Stroke {
            shape: Shape::rounded(shape.rect.inset(0.5), shape.radius - 0.5),
            width: 1.0,
            paint: palette.hairline.into(),
        });
    }
}

pub fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.y >= rect.y
        && point.x < rect.x + rect.width
        && point.y < rect.y + rect.height
}
