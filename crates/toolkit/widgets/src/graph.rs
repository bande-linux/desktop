use std::ops::Range;

use accesskit::{Node, Role};
use bande_graphics::{
    Color, Item, Palette, Paragraph, Path, Point, Radius, Rect, Scheme, Shape, TextAlign, TextRole,
    TextStyle,
};

use super::{Context, Size, Widget, WidgetId};

const CAPTION_HEIGHT: f32 = 20.0;
const WELL_HEIGHT: f32 = 28.0;
const LABEL_PAD: f32 = 6.0;
const STROKE_WIDTH: f32 = 1.5;
const AREA_OPACITY: f32 = 0.3;
const BAND_HEIGHT: f32 = 3.0;
const GRID_ROWS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphKind {
    Line,
    Area,
    Stacked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesColor {
    Primary,
    Secondary,
    Success,
    Warning,
    Danger,
}

impl SeriesColor {
    pub fn color(self, palette: &Palette) -> Color {
        let (lightness, chroma) = match palette.scheme {
            Scheme::Light => (0.56, 0.18),
            Scheme::Dark => (0.72, 0.14),
        };
        match self {
            Self::Primary => Color::oklch(lightness, chroma, 255.0),
            Self::Secondary => Color::oklch(lightness, chroma + 0.02, 25.0),
            Self::Success => palette.success,
            Self::Warning => palette.warning,
            Self::Danger => palette.danger_text,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub color: SeriesColor,
    pub values: Vec<Option<f32>>,
}

impl Series {
    pub fn new(color: SeriesColor, values: impl IntoIterator<Item = f32>) -> Self {
        Self {
            color,
            values: values.into_iter().map(Some).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Band {
    pub color: SeriesColor,
    pub along: Range<f32>,
}

struct Text {
    clip: Rect,
    origin: Point,
    paragraph: Paragraph,
}

impl Text {
    fn centred(
        cx: &mut Context,
        text: &str,
        style: TextStyle,
        clip: Rect,
        align: TextAlign,
    ) -> Self {
        let paragraph = cx.text.paragraph(text, style, Some(clip.width), align);
        let y = clip.y + ((clip.height - paragraph.height()) / 2.0).round();
        Self {
            clip,
            origin: Point::new(clip.x, y),
            paragraph,
        }
    }

    fn paint(&self, items: &mut Vec<Item>) {
        items.push(Item::Clip {
            shape: Shape::rect(self.clip),
            items: vec![Item::Text {
                origin: self.origin,
                paragraph: self.paragraph.clone(),
            }],
        });
    }
}

pub struct Graph {
    id: WidgetId,
    label: String,
    kind: GraphKind,
    span: usize,
    caption: Option<(String, String)>,
    note: Option<String>,
    series: Vec<Series>,
    bands: Vec<Band>,
    texts: Vec<Text>,
    note_text: Option<Text>,
    frame: Rect,
}

impl Graph {
    pub fn new(label: &str, kind: GraphKind, span: usize) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            kind,
            span: span.max(2),
            caption: None,
            note: None,
            series: Vec::new(),
            bands: Vec::new(),
            texts: Vec::new(),
            note_text: None,
            frame: Rect::default(),
        }
    }

    pub fn caption(self, title: &str, span: &str) -> Self {
        Self {
            caption: Some((title.to_owned(), span.to_owned())),
            ..self
        }
    }

    pub fn set_series(&mut self, series: Vec<Series>) {
        self.series = series;
    }

    pub fn set_note(&mut self, note: Option<&str>) {
        self.note = note.map(str::to_owned);
    }

    pub fn set_bands(&mut self, bands: Vec<Band>) {
        self.bands = bands;
    }

    fn well(&self) -> Rect {
        match self.caption {
            Some(_) => Rect::new(
                self.frame.x,
                self.frame.y + CAPTION_HEIGHT,
                self.frame.width,
                (self.frame.height - CAPTION_HEIGHT).max(1.0),
            ),
            None => self.frame,
        }
    }

    fn padded(&self) -> Vec<Vec<Option<f32>>> {
        self.series
            .iter()
            .map(|series| {
                let shown = &series.values[series.values.len().saturating_sub(self.span)..];
                std::iter::repeat_n(None, self.span - shown.len())
                    .chain(shown.iter().copied())
                    .collect()
            })
            .collect()
    }

    fn layers(&self) -> Vec<(SeriesColor, Vec<Option<f32>>)> {
        let padded = self.padded();
        let colors = self.series.iter().map(|series| series.color);
        match self.kind {
            GraphKind::Line | GraphKind::Area => colors.zip(padded).collect(),
            GraphKind::Stacked => {
                let mut running = vec![0.0; self.span];
                let mut layers = colors
                    .zip(padded)
                    .map(|(color, values)| {
                        let tops = values
                            .iter()
                            .zip(&mut running)
                            .map(|(value, total)| {
                                value.map(|value| {
                                    *total += value;
                                    *total
                                })
                            })
                            .collect();
                        (color, tops)
                    })
                    .collect::<Vec<_>>();
                layers.reverse();
                layers
            }
        }
    }

    fn paint_layer(&self, values: &[Option<f32>], plot: Rect, color: Color, items: &mut Vec<Item>) {
        let step = plot.width / (self.span - 1) as f32;
        let bottom = plot.y + plot.height;
        let point = |index: usize, value: f32| {
            Point::new(
                plot.x + index as f32 * step,
                bottom - value.clamp(0.0, 1.0) * plot.height,
            )
        };
        let indexed = values.iter().copied().enumerate().collect::<Vec<_>>();
        for run in indexed.split(|(_, value)| value.is_none()) {
            let run = run
                .iter()
                .filter_map(|(index, value)| value.map(|value| point(*index, value)))
                .collect::<Vec<_>>();
            let run = match run.as_slice() {
                [only] => vec![Point::new(only.x - 1.0, only.y), *only],
                _ => run,
            };
            let (Some(first), Some(last)) = (run.first().copied(), run.last().copied()) else {
                continue;
            };
            let line = Path::through(&run);
            if self.kind != GraphKind::Line {
                items.push(Item::FillPath {
                    path: line
                        .clone()
                        .line_to(Point::new(last.x, bottom))
                        .line_to(Point::new(first.x, bottom))
                        .close(),
                    paint: color.with_opacity(AREA_OPACITY).into(),
                });
            }
            items.push(Item::StrokePath {
                path: line,
                width: STROKE_WIDTH,
                paint: color.into(),
            });
        }
    }
}

fn label_style(cx: &Context) -> TextStyle {
    TextStyle::role(TextRole::Label, cx.palette.ink3)
}

impl Widget for Graph {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        let caption = self.caption.as_ref().map_or(0.0, |_| CAPTION_HEIGHT);
        Size::new(available.width, caption + WELL_HEIGHT)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let label = label_style(cx);
        let caption = Rect::new(frame.x, frame.y, frame.width, CAPTION_HEIGHT);
        self.texts = self
            .caption
            .iter()
            .flat_map(|(title, span)| [(title, TextAlign::Center), (span, TextAlign::End)])
            .map(|(text, align)| Text::centred(cx, text, label, caption, align))
            .collect();
        let well = self.well();
        self.note_text = self.note.as_deref().map(|note| {
            let style = TextStyle {
                caps: false,
                ..label
            }
            .tabular();
            let clip = Rect::new(
                well.x + LABEL_PAD,
                well.y + LABEL_PAD / 2.0,
                well.width - 2.0 * LABEL_PAD,
                CAPTION_HEIGHT - LABEL_PAD,
            );
            Text::centred(cx, note, style, clip, TextAlign::Start)
        });
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let palette = &cx.palette;
        for text in &self.texts {
            text.paint(items);
        }
        let well = self.well();
        let shape = Shape::rounded(well, Radius::S);
        let background = match palette.scheme {
            Scheme::Light => palette.well.mix(palette.sheet, 0.55),
            Scheme::Dark => palette.well.mix(palette.chassis, 0.5),
        };
        let mut inside = vec![Item::Fill {
            shape,
            paint: background.into(),
        }];
        for row in 1..GRID_ROWS {
            let y = (well.y + well.height * row as f32 / GRID_ROWS as f32).round();
            inside.push(Item::Fill {
                shape: Shape::rect(Rect::new(well.x, y, well.width, 1.0)),
                paint: palette.hairline.with_opacity(0.6).into(),
            });
        }
        let plot = Rect::new(
            well.x,
            well.y + STROKE_WIDTH,
            well.width,
            (well.height - STROKE_WIDTH).max(1.0),
        );
        for (color, values) in self.layers() {
            self.paint_layer(&values, plot, color.color(palette), &mut inside);
        }
        let bottom = plot.y + plot.height;
        for band in &self.bands {
            inside.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    plot.x + band.along.start * plot.width,
                    bottom - BAND_HEIGHT,
                    (band.along.end - band.along.start) * plot.width,
                    BAND_HEIGHT,
                )),
                paint: band.color.color(palette).into(),
            });
        }
        if let Some(text) = &self.note_text {
            text.paint(&mut inside);
        }
        items.push(Item::Clip {
            shape,
            items: inside,
        });
        if self.caption.is_some() {
            items.push(Item::Stroke {
                shape: Shape::rounded(well.inset(0.5), Radius::S - 0.5),
                width: 1.0,
                paint: palette.hairline.into(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Image);
        node.set_label(self.label.clone());
        node
    }
}
