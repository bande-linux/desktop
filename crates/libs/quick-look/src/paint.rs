use std::collections::HashMap;
use std::ops::Range;
use std::time::Instant;

use bande_content::pdf::protocol::Use;
use bande_content::pdf::session::Phase;
use bande_content::{Block, Tone};
use bande_fs::{format_date, format_size};
use bande_l10n::{tr, trn};
use bande_toolkit::{
    Color, Elevation, Image, Item, Palette, Paragraph, Point, Radius, Rect, ResizeEdge, Shape,
    TextAlign, TextRole, TextStyle, TextSystem, Transform,
};
use bande_xdg::{ICON_THEMES, data_dirs, find_icon};
use parley::{Alignment, AlignmentOptions, FontFamily, FontFamilyName, StyleProperty};

use crate::content::{CodeView, DocumentView, FontView, Picture, Shown};
use crate::pdf::{GAP, PdfView, render};
use crate::worker::SAMPLE_WIDTH;
use crate::{Hit, MIN_SIZE, QuickLook, Resizing, Subject};

const HEADER: f32 = 44.0;
const PADDING: f32 = 16.0;
const MARGIN: f32 = 24.0;
const BUTTON: f32 = 28.0;
const SYMBOL: f32 = 16.0;
const SYMBOL_PIXELS: u32 = 32;
const ICON: f32 = 128.0;
const CODE_SIZE: f32 = 12.0;
const GRIP: f32 = 6.0;
const READABLE: f32 = 760.0;
const SIDEBAR: f32 = 124.0;
const THUMBNAIL: f32 = 84.0;
const TILE: f32 = 150.0;
const START_SCALE: f32 = 0.85;
const FIELD_WIDTH: f32 = 240.0;

#[derive(Default)]
pub struct Symbols {
    loaded: HashMap<(&'static str, [u8; 4]), Option<Image>>,
    data_dirs: Option<Vec<std::path::PathBuf>>,
}

impl Symbols {
    fn get(&mut self, name: &'static str, color: Color) -> Option<Image> {
        let data_dirs = self.data_dirs.get_or_insert_with(data_dirs);
        self.loaded
            .entry((name, [color.r, color.g, color.b, color.a]))
            .or_insert_with(|| {
                find_icon(name, SYMBOL_PIXELS, &ICON_THEMES, data_dirs)
                    .and_then(|path| Image::load(&path, SYMBOL_PIXELS))
                    .and_then(|image| image.tinted(color))
            })
            .clone()
    }
}

pub fn resized(resizing: &Resizing, at: Point, bounds: Rect) -> (f32, f32) {
    let (dx, dy) = (at.x - resizing.from.x, at.y - resizing.from.y);
    let (horizontal, vertical) = match resizing.edge {
        ResizeEdge::Left => (-1.0, 0.0),
        ResizeEdge::Right => (1.0, 0.0),
        ResizeEdge::Top => (0.0, -1.0),
        ResizeEdge::Bottom => (0.0, 1.0),
        ResizeEdge::TopLeft => (-1.0, -1.0),
        ResizeEdge::TopRight => (1.0, -1.0),
        ResizeEdge::BottomLeft => (-1.0, 1.0),
        ResizeEdge::BottomRight => (1.0, 1.0),
    };
    let most = (
        (bounds.width - 2.0 * MARGIN).max(MIN_SIZE.0),
        (bounds.height - 2.0 * MARGIN).max(MIN_SIZE.1),
    );
    (
        (resizing.size.0 + 2.0 * horizontal * dx).clamp(MIN_SIZE.0, most.0),
        (resizing.size.1 + 2.0 * vertical * dy).clamp(MIN_SIZE.1, most.1),
    )
}

fn lerp(from: f32, to: f32, amount: f32) -> f32 {
    from + (to - from) * amount
}

fn fitted(area: Rect, width: f32, height: f32, upscale: bool) -> Rect {
    let mut scale = (area.width / width.max(1.0)).min(area.height / height.max(1.0));
    if !upscale {
        scale = scale.min(1.0);
    }
    let (width, height) = (width * scale, height * scale);
    Rect::new(
        area.x + (area.width - width) / 2.0,
        area.y + (area.height - height) / 2.0,
        width,
        height,
    )
}

pub(crate) fn colored(
    text: &mut TextSystem,
    content: &str,
    style: TextStyle,
    family: Option<&str>,
    colors: &[(Range<usize>, Color)],
    width: Option<f32>,
) -> Paragraph {
    let scale = text.size().scale();
    let (fonts, layouts) = text.contexts();
    let mut builder = layouts.ranged_builder(fonts, content, scale, true);
    style.apply(&mut builder);
    if let Some(family) = family {
        builder.push_default(StyleProperty::FontFamily(FontFamily::List(
            vec![FontFamilyName::named(family)].into(),
        )));
    }
    for (range, color) in colors {
        if content.get(range.clone()).is_some() {
            builder.push(StyleProperty::Brush(*color), range.clone());
        }
    }
    let mut layout = builder.build(content);
    layout.break_all_lines(width);
    layout.align(Alignment::Start, AlignmentOptions::default());
    Paragraph::from_layout(layout)
}

fn tone_color(tone: Tone, palette: &Palette) -> Color {
    match tone {
        Tone::Keyword => palette.accent_text,
        Tone::Literal => palette.danger_text,
        Tone::Number => palette.warning,
        Tone::Comment => palette.ink3,
    }
}

struct Painter<'a> {
    text: &'a mut TextSystem,
    palette: Palette,
    scale: f32,
    items: Vec<Item>,
}

impl Painter<'_> {
    fn paragraph(&mut self, text: &str, style: TextStyle, width: Option<f32>) -> Paragraph {
        self.text.paragraph(text, style, width, TextAlign::Start)
    }

    fn centered(&mut self, text: &str, style: TextStyle, area: Rect, y: f32) -> f32 {
        let paragraph = self
            .text
            .paragraph(text, style, Some(area.width), TextAlign::Center);
        let height = paragraph.height();
        self.items.push(Item::Text {
            origin: Point::new(area.x, y),
            paragraph,
        });
        height
    }

    fn summary(&mut self, subject: &Subject, area: Rect, lines: &[String]) {
        let name_style = TextStyle::new(17.0, self.palette.ink).weight(TextStyle::SEMIBOLD);
        let line_style = TextStyle::role(TextRole::Body, self.palette.ink2);
        let name_height = self
            .text
            .paragraph(
                &subject.entry.name,
                name_style,
                Some(area.width),
                TextAlign::Center,
            )
            .height();
        let line_height = self.paragraph("M", line_style, None).height();
        let total = ICON + 16.0 + name_height + 6.0 + lines.len() as f32 * (line_height + 2.0);
        let mut y = (area.y + (area.height - total) / 2.0).max(area.y).round();
        if let Some(icon) = &subject.icon {
            let rect = fitted(
                Rect::new(area.x + (area.width - ICON) / 2.0, y, ICON, ICON),
                icon.width() as f32,
                icon.height() as f32,
                true,
            );
            self.items.push(Item::Image {
                rect,
                image: icon.clone(),
            });
        }
        y += ICON + 16.0;
        y += self.centered(&subject.entry.name, name_style, area, y) + 6.0;
        for line in lines.iter().filter(|line| !line.is_empty()) {
            y += self.centered(line, line_style, area, y) + 2.0;
        }
    }

    fn symbol_button(
        &mut self,
        symbols: &mut Symbols,
        hits: &mut Vec<(Rect, Hit)>,
        rect: Rect,
        name: &'static str,
        hit: Hit,
        hover: Option<Hit>,
    ) {
        if hover == Some(hit) {
            self.items.push(Item::Fill {
                shape: Shape::rounded(rect, Radius::S),
                paint: self.palette.well.into(),
            });
        }
        if let Some(image) = symbols.get(name, self.palette.ink2) {
            self.items.push(Item::Image {
                rect: Rect::new(
                    rect.x + (rect.width - SYMBOL) / 2.0,
                    rect.y + (rect.height - SYMBOL) / 2.0,
                    SYMBOL,
                    SYMBOL,
                ),
                image,
            });
        }
        hits.push((rect, hit));
    }
}

impl Painter<'_> {
    fn password(
        &mut self,
        area: Rect,
        typed: usize,
        retry: bool,
        hover: Option<Hit>,
        hits: &mut Vec<(Rect, Hit)>,
    ) {
        let palette = self.palette;
        let width = FIELD_WIDTH.min(area.width);
        let left = area.x + (area.width - width) / 2.0;
        let top = area.y + area.height - 3.0 * BUTTON - PADDING;
        let field = Rect::new(left, top, width, BUTTON);
        self.items.push(Item::Fill {
            shape: Shape::rounded(field, Radius::S),
            paint: palette.field.into(),
        });
        self.items.push(Item::Stroke {
            shape: Shape::rounded(field, Radius::S),
            width: 2.0,
            paint: palette.focus.into(),
        });
        let (shown, color) = match typed {
            0 => (tr!("Password"), palette.ink3),
            count => ("\u{2022}".repeat(count), palette.ink),
        };
        let paragraph =
            self.text
                .fitted(&shown, TextStyle::role(TextRole::Body, color), width - 16.0);
        self.items.push(Item::Text {
            origin: Point::new(
                left + 8.0,
                top + ((BUTTON - paragraph.height()) / 2.0).round(),
            ),
            paragraph,
        });
        let label = self.paragraph(
            &tr!("Unlock"),
            TextStyle::role(TextRole::Body, palette.on_accent).weight(TextStyle::MEDIUM),
            None,
        );
        let button_width = (label.width() + 24.0).ceil();
        let button = Rect::new(
            area.x + (area.width - button_width) / 2.0,
            top + BUTTON + 8.0,
            button_width,
            BUTTON,
        );
        let fill = match hover == Some(Hit::Unlock) {
            true => palette.accent.mix(palette.ink, 0.1),
            false => palette.accent,
        };
        self.items.push(Item::Fill {
            shape: Shape::rounded(button, Radius::S),
            paint: fill.into(),
        });
        self.items.push(Item::Text {
            origin: Point::new(
                button.x + 12.0,
                button.y + ((BUTTON - label.height()) / 2.0).round(),
            ),
            paragraph: label,
        });
        hits.push((button, Hit::Unlock));
        if retry {
            let style = TextStyle::role(TextRole::Callout, palette.danger_text);
            self.centered(
                &tr!("The password is incorrect"),
                style,
                area,
                top - BUTTON + 4.0,
            );
        }
    }
}

fn summary_lines(subject: &Subject, shown: &Shown) -> Vec<String> {
    let entry = &subject.entry;
    let modified = entry
        .modified
        .map(|time| tr!("Modified {date}", date = format_date(time)))
        .unwrap_or_default();
    let size = match shown {
        Shown::Folder { items, bytes } => {
            let items = trn!("1 item", "{count} items", count = (*items));
            match bytes {
                Some(bytes) => tr!("{items}, {size}", items = items, size = format_size(*bytes)),
                None => items,
            }
        }
        Shown::Archive(contents) => {
            let unpacked = trn!(
                "1 item, {size} unpacked",
                "{count} items, {size} unpacked",
                count = contents.items,
                size = format_size(contents.bytes)
            );
            return vec![
                subject.kind.clone(),
                format_size(entry.size),
                unpacked,
                modified,
            ];
        }
        Shown::Pdf(pdf) if pdf.phase() == Phase::Failed => tr!("This document cannot be shown"),
        Shown::Pdf(pdf) if matches!(pdf.phase(), Phase::Locked { .. }) => {
            tr!("This document is locked")
        }
        _ if entry.is_folder() => String::new(),
        _ => format_size(entry.size),
    };
    vec![subject.kind.clone(), size, modified]
}

impl QuickLook {
    fn target(&self, bounds: Rect) -> Rect {
        if self.full_screen || self.windowed {
            return bounds;
        }
        let width = self
            .size
            .0
            .clamp(MIN_SIZE.0, (bounds.width - 2.0 * MARGIN).max(MIN_SIZE.0))
            .min(bounds.width);
        let height = self
            .size
            .1
            .clamp(MIN_SIZE.1, (bounds.height - 2.0 * MARGIN).max(MIN_SIZE.1))
            .min(bounds.height);
        let room = (
            ((bounds.width - width) / 2.0).max(0.0),
            ((bounds.height - height) / 2.0).max(0.0),
        );
        Rect::new(
            (bounds.x + room.0 + self.offset.x.clamp(-room.0, room.0)).round(),
            (bounds.y + room.1 + self.offset.y.clamp(-room.1, room.1)).round(),
            width.round(),
            height.round(),
        )
    }

    pub fn paint(
        &mut self,
        text: &mut TextSystem,
        palette: Palette,
        bounds: Rect,
        scale: f32,
        now: Instant,
    ) -> Vec<Item> {
        self.hits.clear();
        self.area = bounds;
        let target = self.target(bounds);
        self.frame = target;
        let presence = self.presence.value_at(now).0.clamp(0.0, 1.0);
        let mut painter = Painter {
            text,
            palette,
            scale,
            items: Vec::new(),
        };
        let shape = match self.full_screen {
            true => Shape::rect(target),
            false => Shape::rounded(target, Radius::XL),
        };
        if !self.full_screen && !self.windowed {
            painter.items.extend(
                Elevation::Overlay
                    .shadows(palette.scheme, true)
                    .into_iter()
                    .map(|shadow| Item::Shadow {
                        shape,
                        offset: Point::new(0.0, shadow.offset),
                        blur_radius: shadow.blur,
                        color: Color::BLACK.with_opacity(shadow.opacity),
                    }),
            );
        }
        painter.items.push(Item::Fill {
            shape,
            paint: palette.overlay.into(),
        });
        self.hits.push((target, Hit::Panel));
        self.hits.push((
            Rect::new(target.x, target.y, target.width, HEADER),
            Hit::Header,
        ));
        let mut content = std::mem::take(&mut painter.items);
        self.paint_header(&mut painter, target);
        let area = Rect::new(
            target.x + PADDING,
            target.y + HEADER,
            (target.width - 2.0 * PADDING).max(0.0),
            (target.height - HEADER - PADDING).max(0.0),
        );
        match self.index_sheet {
            true => self.paint_index_sheet(&mut painter, area),
            false => self.paint_content(&mut painter, area, now),
        }
        content.push(Item::Clip {
            shape,
            items: std::mem::take(&mut painter.items),
        });
        if !self.full_screen {
            self.resize_hits(target);
        }
        let origin = self
            .subject()
            .and_then(|subject| subject.origin)
            .filter(|_| !self.windowed);
        let shown = match origin {
            Some(origin) if !self.reduce_motion => Rect::new(
                lerp(origin.x, target.x, presence),
                lerp(origin.y, target.y, presence),
                lerp(origin.width, target.width, presence),
                lerp(origin.height, target.height, presence),
            ),
            _ if !self.reduce_motion => {
                let factor = lerp(START_SCALE, 1.0, presence);
                let (width, height) = (target.width * factor, target.height * factor);
                Rect::new(
                    target.x + (target.width - width) / 2.0,
                    target.y + (target.height - height) / 2.0,
                    width,
                    height,
                )
            }
            _ => target,
        };
        let mut items = content;
        if presence < 1.0 {
            let transform = Transform::translate(-target.x, -target.y)
                .then(Transform::scale(
                    shown.width / target.width.max(1.0),
                    shown.height / target.height.max(1.0),
                ))
                .then(Transform::translate(shown.x, shown.y));
            items = vec![Item::Opacity {
                alpha: presence.min(1.0),
                items: vec![Item::Transform { transform, items }],
            }];
        }
        items
    }

    fn resize_hits(&mut self, frame: Rect) {
        let frame = match self.windowed {
            true => frame.inset(GRIP),
            false => frame,
        };
        let (left, top) = (frame.x - GRIP, frame.y - GRIP);
        let (right, bottom) = (frame.x + frame.width - GRIP, frame.y + frame.height - GRIP);
        let (width, height) = (frame.width - 2.0 * GRIP, frame.height - 2.0 * GRIP);
        let grip = 2.0 * GRIP;
        self.hits.extend([
            (
                Rect::new(left + grip, top, width, grip),
                Hit::Resize(ResizeEdge::Top),
            ),
            (
                Rect::new(left + grip, bottom, width, grip),
                Hit::Resize(ResizeEdge::Bottom),
            ),
            (
                Rect::new(left, top + grip, grip, height),
                Hit::Resize(ResizeEdge::Left),
            ),
            (
                Rect::new(right, top + grip, grip, height),
                Hit::Resize(ResizeEdge::Right),
            ),
            (
                Rect::new(left, top, grip, grip),
                Hit::Resize(ResizeEdge::TopLeft),
            ),
            (
                Rect::new(right, top, grip, grip),
                Hit::Resize(ResizeEdge::TopRight),
            ),
            (
                Rect::new(left, bottom, grip, grip),
                Hit::Resize(ResizeEdge::BottomLeft),
            ),
            (
                Rect::new(right, bottom, grip, grip),
                Hit::Resize(ResizeEdge::BottomRight),
            ),
        ]);
    }

    fn paint_header(&mut self, painter: &mut Painter<'_>, panel: Rect) {
        let palette = painter.palette;
        let top = panel.y + ((HEADER - BUTTON) / 2.0).round();
        let mut left = panel.x + 10.0;
        let button = |painter: &mut Painter<'_>, x: f32, name, hit, this: &mut Self| {
            painter.symbol_button(
                &mut this.symbols,
                &mut this.hits,
                Rect::new(x, top, BUTTON, BUTTON),
                name,
                hit,
                this.hover,
            );
        };
        button(painter, left, "window-close-symbolic", Hit::Close, self);
        left += BUTTON + 4.0;
        let full_screen = match self.full_screen {
            true => "view-restore-symbolic",
            false => "view-fullscreen-symbolic",
        };
        button(painter, left, full_screen, Hit::FullScreen, self);
        left += BUTTON + 10.0;
        if let Some((at, count)) = self.position() {
            button(painter, left, "go-previous-symbolic", Hit::Previous, self);
            left += BUTTON;
            button(painter, left, "go-next-symbolic", Hit::Next, self);
            left += BUTTON + 6.0;
            button(painter, left, "view-grid-symbolic", Hit::IndexSheet, self);
            left += BUTTON + 8.0;
            let counter = painter.paragraph(
                &tr!("{at} of {count}", at = at, count = count),
                TextStyle::role(TextRole::Callout, palette.ink2).tabular(),
                None,
            );
            let width = counter.width();
            painter.items.push(Item::Text {
                origin: Point::new(left, panel.y + ((HEADER - counter.height()) / 2.0).round()),
                paragraph: counter,
            });
            left += width + 10.0;
        }
        let mut right = panel.x + panel.width - 10.0;
        if let Some(app) = self.subject().and_then(|subject| subject.app.clone()) {
            let label = painter.paragraph(
                &tr!("Open with {app}", app = app.name),
                TextStyle::role(TextRole::Body, palette.ink).weight(TextStyle::MEDIUM),
                None,
            );
            let width = (label.width() + 24.0).ceil();
            let rect = Rect::new(right - width, top, width, BUTTON);
            let fill = match self.hover == Some(Hit::Open) {
                true => palette.well.mix(palette.ink, 0.08),
                false => palette.well,
            };
            painter.items.push(Item::Fill {
                shape: Shape::rounded(rect, Radius::S),
                paint: fill.into(),
            });
            painter.items.push(Item::Text {
                origin: Point::new(
                    rect.x + 12.0,
                    top + ((BUTTON - label.height()) / 2.0).round(),
                ),
                paragraph: label,
            });
            self.hits.push((rect, Hit::Open));
            right = rect.x - 6.0;
        }
        right -= BUTTON;
        button(painter, right, "edit-copy-symbolic", Hit::Copy, self);
        if self.shown.is_picture() && !self.index_sheet {
            right -= BUTTON + 2.0;
            button(
                painter,
                right,
                "object-rotate-left-symbolic",
                Hit::Rotate,
                self,
            );
        }
        right -= 10.0;
        let Some(name) = self.subject().map(|subject| subject.entry.name.clone()) else {
            return;
        };
        let middle = panel.x + panel.width / 2.0;
        let room = (2.0 * (right - middle)).min(2.0 * (middle - left)).max(0.0);
        let title = painter.text.fitted(
            &name,
            TextStyle::role(TextRole::BodyStrong, palette.ink),
            room,
        );
        painter.items.push(Item::Text {
            origin: Point::new(
                (middle - title.width() / 2.0).round(),
                panel.y + ((HEADER - title.height()) / 2.0).round(),
            ),
            paragraph: title,
        });
    }

    fn paint_content(&mut self, painter: &mut Painter<'_>, area: Rect, now: Instant) {
        let Some(subject) = self.subjects.get(self.at).cloned() else {
            return;
        };
        let (rotation, zoom, scale) = (self.rotation, self.zoom, painter.scale);
        match &mut self.shown {
            Shown::Picture(picture) => {
                self.pan = paint_picture(painter, picture, area, rotation, zoom, self.pan, scale);
            }
            Shown::Pdf(pdf) if pdf.phase() == Phase::Open && !pdf.pages.is_empty() => {
                self.scroll = paint_pdf(painter, pdf, area, self.scroll, &mut self.hits);
            }
            Shown::Code(code) => self.scroll = paint_code(painter, code, area, self.scroll),
            Shown::Document(document) => {
                self.scroll = paint_document(painter, document, area, self.scroll);
            }
            Shown::Font(font) => self.scroll = paint_font(painter, font, area, self.scroll),
            Shown::Media(Some(stage)) => {
                stage.layout(painter.text, &painter.palette, area, now);
                stage.paint(&painter.palette, now, &mut painter.items);
            }
            shown => {
                let lines = summary_lines(&subject, shown);
                painter.summary(&subject, area, &lines);
                if let Shown::Pdf(pdf) = shown
                    && let Phase::Locked { retry } = pdf.phase()
                {
                    let password = self.password.chars().count();
                    let hover = self.hover;
                    painter.password(area, password, retry, hover, &mut self.hits);
                }
            }
        }
    }

    fn paint_index_sheet(&mut self, painter: &mut Painter<'_>, area: Rect) {
        let columns = ((area.width / TILE).floor() as usize).max(1);
        let tile_width = area.width / columns as f32;
        let tile_height = TILE;
        let rows = self.subjects.len().div_ceil(columns);
        let most = (rows as f32 * tile_height - area.height).max(0.0);
        self.scroll.y = self.scroll.y.clamp(0.0, most);
        let mut items = Vec::new();
        let style = TextStyle::role(TextRole::Callout, painter.palette.ink);
        for (index, subject) in self.subjects.iter().enumerate() {
            let (row, column) = (index / columns, index % columns);
            let tile = Rect::new(
                area.x + column as f32 * tile_width,
                area.y + row as f32 * tile_height - self.scroll.y,
                tile_width,
                tile_height,
            );
            if tile.y + tile.height < area.y || tile.y > area.y + area.height {
                continue;
            }
            if index == self.at || self.hover == Some(Hit::Tile(index)) {
                items.push(Item::Fill {
                    shape: Shape::rounded(tile.inset(4.0), Radius::M),
                    paint: painter.palette.well.into(),
                });
            }
            if let Some(icon) = &subject.icon {
                let room = Rect::new(tile.x + 12.0, tile.y + 10.0, tile.width - 24.0, 96.0);
                items.push(Item::Image {
                    rect: fitted(room, icon.width() as f32, icon.height() as f32, true),
                    image: icon.clone(),
                });
            }
            let name = painter
                .text
                .fitted(&subject.entry.name, style, tile.width - 16.0);
            items.push(Item::Text {
                origin: Point::new(tile.x + (tile.width - name.width()) / 2.0, tile.y + 114.0),
                paragraph: name,
            });
            self.hits.push((tile.inset(4.0), Hit::Tile(index)));
        }
        painter.items.push(Item::Clip {
            shape: Shape::rect(area),
            items,
        });
    }
}

fn paint_picture(
    painter: &mut Painter<'_>,
    picture: &mut Picture,
    area: Rect,
    rotation: u8,
    zoom: f32,
    pan: Point,
    scale: f32,
) -> Point {
    let (width, height) = (picture.width as f32, picture.height as f32);
    let turned = rotation % 2 == 1;
    let (shown_width, shown_height) = if turned {
        (height, width)
    } else {
        (width, height)
    };
    let fit = fitted(area, shown_width / scale, shown_height / scale, false);
    let (zoomed_width, zoomed_height) = (fit.width * zoom, fit.height * zoom);
    let room = (
        ((zoomed_width - area.width) / 2.0).max(0.0),
        ((zoomed_height - area.height) / 2.0).max(0.0),
    );
    let pan = Point::new(pan.x.clamp(-room.0, room.0), pan.y.clamp(-room.1, room.1));
    let center = Point::new(
        area.x + area.width / 2.0 + pan.x,
        area.y + area.height / 2.0 + pan.y,
    );
    let (drawn_width, drawn_height) = if turned {
        (zoomed_height, zoomed_width)
    } else {
        (zoomed_width, zoomed_height)
    };
    let Some(image) = picture.image(drawn_width * scale) else {
        return pan;
    };
    let rect = Rect::new(
        center.x - drawn_width / 2.0,
        center.y - drawn_height / 2.0,
        drawn_width,
        drawn_height,
    );
    let mut items = vec![Item::Image { rect, image }];
    if rotation != 0 {
        let transform = Transform::translate(-center.x, -center.y)
            .then(Transform::rotate(
                -f32::from(rotation) * std::f32::consts::FRAC_PI_2,
            ))
            .then(Transform::translate(center.x, center.y));
        items = vec![Item::Transform { transform, items }];
    }
    painter.items.push(Item::Clip {
        shape: Shape::rect(area),
        items,
    });
    pan
}

fn paint_pdf(
    painter: &mut Painter<'_>,
    pdf: &mut PdfView,
    area: Rect,
    scroll: Point,
    hits: &mut Vec<(Rect, crate::Hit)>,
) -> Point {
    let sidebar = pdf.pages.len() > 1 && area.width >= 3.0 * SIDEBAR;
    let pages_area = match sidebar {
        true => Rect::new(area.x + SIDEBAR, area.y, area.width - SIDEBAR, area.height),
        false => area,
    };
    let page_width = (pages_area.width - 2.0 * GAP).clamp(1.0, READABLE * 1.2);
    let total = pdf.lay_out(page_width);
    let scroll = Point::new(0.0, scroll.y.clamp(0.0, (total - area.height).max(0.0)));
    let left = pages_area.x + (pages_area.width - page_width) / 2.0;
    let mut items = Vec::new();
    let mut renders = Vec::new();
    for page in 0..pdf.pages.len() {
        let Some((top, height)) = pdf.page_rect(page) else {
            continue;
        };
        let y = area.y + top - scroll.y;
        if y + height < area.y - area.height || y > area.y + 2.0 * area.height {
            continue;
        }
        let rect = Rect::new(left, y, page_width, height);
        items.push(Item::Fill {
            shape: Shape::rect(rect),
            paint: Color::WHITE.into(),
        });
        if let Some((_, image)) = pdf.rendered.get(&page) {
            items.push(Item::Image {
                rect,
                image: image.clone(),
            });
        }
        let size = pdf.pages[page];
        renders.push(render(
            page,
            size,
            page_width / size.0.max(1.0) * painter.scale,
            Use::Page,
        ));
    }
    painter.items.push(Item::Clip {
        shape: Shape::rect(pages_area),
        items,
    });
    if sidebar {
        let current = pdf.page_at(scroll.y);
        let step = THUMBNAIL * 1.5 + 22.0;
        let offset = (current as f32 * step - area.height / 3.0)
            .clamp(0.0, (pdf.pages.len() as f32 * step - area.height).max(0.0));
        let mut items = Vec::new();
        for page in 0..pdf.pages.len() {
            let y = area.y + page as f32 * step - offset;
            if y + step < area.y || y > area.y + area.height {
                continue;
            }
            let size = pdf.pages[page];
            let room = Rect::new(
                area.x + (SIDEBAR - THUMBNAIL) / 2.0,
                y + 4.0,
                THUMBNAIL,
                THUMBNAIL * 1.5,
            );
            let rect = fitted(room, size.0, size.1, true);
            if page == current {
                items.push(Item::Fill {
                    shape: Shape::rounded(rect.inset(-4.0), Radius::S),
                    paint: painter.palette.accent_soft.into(),
                });
            }
            items.push(Item::Fill {
                shape: Shape::rect(rect),
                paint: Color::WHITE.into(),
            });
            if let Some(image) = pdf.thumbnails.get(&page) {
                items.push(Item::Image {
                    rect,
                    image: image.clone(),
                });
            }
            let number = painter.text.paragraph(
                &(page + 1).to_string(),
                TextStyle::role(TextRole::Label, painter.palette.ink2),
                Some(SIDEBAR),
                TextAlign::Center,
            );
            items.push(Item::Text {
                origin: Point::new(area.x, room.y + room.height + 2.0),
                paragraph: number,
            });
            hits.push((rect, crate::Hit::Thumbnail(page)));
            renders.push(render(
                page,
                size,
                rect.width / size.0.max(1.0) * painter.scale,
                Use::Thumbnail,
            ));
        }
        painter.items.push(Item::Clip {
            shape: Shape::rect(Rect::new(area.x, area.y, SIDEBAR, area.height)),
            items,
        });
    }
    pdf.want(renders);
    scroll
}

fn paint_code(painter: &mut Painter<'_>, code: &mut CodeView, area: Rect, scroll: Point) -> Point {
    let style = TextStyle::new(CODE_SIZE, painter.palette.ink).monospace();
    let sample = painter.paragraph("M", style, None);
    let (line, advance) = (sample.height().max(1.0), sample.width().max(1.0));
    let lines = code.starts.len();
    let scroll = Point::new(
        scroll
            .x
            .clamp(0.0, (code.widest as f32 * advance - area.width).max(0.0)),
        scroll
            .y
            .clamp(0.0, (lines as f32 * line - area.height).max(0.0)),
    );
    let first = (scroll.y / line).floor() as usize;
    let count = (area.height / line).ceil() as usize + 1;
    let last = (first + count).min(lines);
    let start = code
        .starts
        .get(first)
        .copied()
        .unwrap_or(code.code.text.len());
    let end = code
        .starts
        .get(last)
        .map_or(code.code.text.len(), |end| end.saturating_sub(1));
    let reuse = code
        .laid
        .as_ref()
        .is_some_and(|((from, to), _)| (*from, *to) == (first, last));
    if !reuse {
        let palette = painter.palette;
        let colors: Vec<(Range<usize>, Color)> = code
            .code
            .tones
            .iter()
            .filter(|(range, _)| range.end > start && range.start < end)
            .map(|(range, tone)| {
                (
                    range.start.max(start) - start..range.end.min(end) - start,
                    tone_color(*tone, &palette),
                )
            })
            .collect();
        let slice = code.code.text.get(start..end).unwrap_or_default();
        let paragraph = colored(painter.text, slice, style, None, &colors, None);
        code.laid = Some(((first, last), paragraph));
    }
    if let Some((_, paragraph)) = &code.laid {
        painter.items.push(Item::Clip {
            shape: Shape::rect(area),
            items: vec![Item::Text {
                origin: Point::new(area.x - scroll.x, area.y + first as f32 * line - scroll.y),
                paragraph: paragraph.clone(),
            }],
        });
    }
    scroll
}

fn block_style(block: &Block, palette: &Palette) -> TextStyle {
    match block {
        Block::Heading(level, _) => {
            let size = match level {
                1 => 26.0,
                2 => 21.0,
                3 => 17.0,
                _ => 15.0,
            };
            TextStyle::new(size, palette.ink).weight(TextStyle::BOLD)
        }
        Block::Quote(_) => TextStyle::new(14.0, palette.ink2),
        Block::Code(_) | Block::Plain(_) => TextStyle::new(CODE_SIZE, palette.ink).monospace(),
        Block::Paragraph(_) | Block::Bullet(_) | Block::Rule => TextStyle::new(14.0, palette.ink),
    }
}

fn paint_document(
    painter: &mut Painter<'_>,
    document: &mut DocumentView,
    area: Rect,
    scroll: Point,
) -> Point {
    let width = area.width.min(READABLE);
    let left = area.x + (area.width - width) / 2.0;
    if document
        .laid
        .as_ref()
        .is_none_or(|(laid, _, _)| (laid - width).abs() > 0.5)
    {
        let mut y = 0.0;
        let mut laid = Vec::new();
        for block in &document.blocks {
            let style = block_style(block, &painter.palette);
            let (text, indent) = match block {
                Block::Heading(_, text)
                | Block::Paragraph(text)
                | Block::Code(text)
                | Block::Plain(text) => (text.as_str(), 0.0),
                Block::Bullet(text) | Block::Quote(text) => (text.as_str(), 18.0),
                Block::Rule => ("", 0.0),
            };
            let inset = if matches!(block, Block::Code(_)) {
                20.0
            } else {
                indent
            };
            let paragraph = painter.paragraph(text, style, Some((width - inset).max(1.0)));
            let height = match block {
                Block::Rule => 1.0,
                Block::Code(_) => paragraph.height() + 16.0,
                _ => paragraph.height(),
            };
            if matches!(block, Block::Heading(..)) && y > 0.0 {
                y += 8.0;
            }
            laid.push((y, block.clone(), paragraph));
            y += height + 10.0;
        }
        document.laid = Some((width, laid, y));
    }
    let Some((_, laid, total)) = &document.laid else {
        return scroll;
    };
    let scroll = Point::new(0.0, scroll.y.clamp(0.0, (total - area.height).max(0.0)));
    let mut items = Vec::new();
    let palette = painter.palette;
    for (top, block, paragraph) in laid {
        let y = area.y + top - scroll.y;
        if y > area.y + area.height || y + paragraph.height() + 16.0 < area.y {
            continue;
        }
        match block {
            Block::Rule => items.push(Item::Fill {
                shape: Shape::rect(Rect::new(left, y, width, 1.0)),
                paint: palette.hairline.into(),
            }),
            Block::Code(_) => {
                items.push(Item::Fill {
                    shape: Shape::rounded(
                        Rect::new(left, y, width, paragraph.height() + 16.0),
                        Radius::S,
                    ),
                    paint: palette.well.into(),
                });
                items.push(Item::Text {
                    origin: Point::new(left + 10.0, y + 8.0),
                    paragraph: paragraph.clone(),
                });
            }
            Block::Bullet(_) => {
                let bullet = painter.paragraph("•", block_style(block, &palette), None);
                items.push(Item::Text {
                    origin: Point::new(left + 4.0, y),
                    paragraph: bullet,
                });
                items.push(Item::Text {
                    origin: Point::new(left + 18.0, y),
                    paragraph: paragraph.clone(),
                });
            }
            Block::Quote(_) => {
                items.push(Item::Fill {
                    shape: Shape::rect(Rect::new(left + 2.0, y, 3.0, paragraph.height())),
                    paint: palette.hairline.into(),
                });
                items.push(Item::Text {
                    origin: Point::new(left + 18.0, y),
                    paragraph: paragraph.clone(),
                });
            }
            Block::Heading(..) | Block::Paragraph(_) | Block::Plain(_) => items.push(Item::Text {
                origin: Point::new(left, y),
                paragraph: paragraph.clone(),
            }),
        }
    }
    painter.items.push(Item::Clip {
        shape: Shape::rect(area),
        items,
    });
    scroll
}

fn paint_font(painter: &mut Painter<'_>, font: &mut FontView, area: Rect, scroll: Point) -> Point {
    let Some(image) = font.image(painter.palette.ink) else {
        return scroll;
    };
    let natural = SAMPLE_WIDTH;
    let width = natural.min(area.width);
    let height = image.height() as f32 * width / image.width().max(1) as f32;
    let scroll = Point::new(0.0, scroll.y.clamp(0.0, (height - area.height).max(0.0)));
    let left = area.x + (area.width - width) / 2.0;
    painter.items.push(Item::Clip {
        shape: Shape::rect(area),
        items: vec![Item::Image {
            rect: Rect::new(left, area.y - scroll.y, width, height),
            image,
        }],
    });
    scroll
}
