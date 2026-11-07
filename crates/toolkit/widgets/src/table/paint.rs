use std::collections::HashMap;
use std::rc::Rc;

use bande_graphics::{
    Color, Cut, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle,
    TextSystem, shortened,
};

use super::model::{Cell, Line};
use super::{CELL_PADDING, Glide, ICON_SIZE, Table};
use crate::{Animated, Context, Spring};

const SELECTION_INSET: (f32, f32) = (4.0, 1.0);
const ICON_GAP: f32 = 6.0;
const TEXT_SIZE: f32 = 13.0;
const FLOATING_SHADOW: f32 = 0.18;
const FLOATING_BLUR: f32 = 8.0;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct TextKey {
    text: Rc<str>,
    column: usize,
    color: Color,
    width: u32,
    label: bool,
    tabular: bool,
    cut: Cut,
}

impl Table {
    pub(super) fn text(
        &self,
        text: &mut TextSystem,
        content: &str,
        style: TextStyle,
        width: f32,
        cut: Cut,
    ) -> Paragraph {
        let shown = shortened(content, cut, |candidate| {
            text.paragraph(candidate, style, None, TextAlign::Start)
                .width()
                <= width
        });
        text.paragraph(&shown, style, None, TextAlign::Start)
    }

    pub(super) fn text_key(
        &self,
        cx: &Context,
        line: &Line,
        column: usize,
        lit: bool,
        widths: &[f32],
    ) -> TextKey {
        let cell = &line.cells[column];
        let color = match (lit, cell.tint, cell.label, cell.muted) {
            (true, _, _, _) => cx.palette.on_accent,
            (false, Some(tint), _, _) => tint,
            (false, None, true, _) => cx.palette.ink3,
            (false, None, false, true) => cx.palette.ink2,
            (false, None, false, false) => cx.palette.ink,
        };
        let indent = if column == 0 { self.indent(line) } else { 0.0 };
        let width = widths[column] - 2.0 * CELL_PADDING - self.icon_space(column, cell) - indent;
        let width = width.max(0.0);
        TextKey {
            text: cell.text.clone(),
            column,
            color,
            width: width.to_bits(),
            label: cell.label,
            tabular: cell.tabular || self.columns[column].numeric,
            cut: cell.cut,
        }
    }

    pub fn text_bounds(&self, index: usize, column: usize) -> Option<Rect> {
        let bounds = self.cell_bounds(index, column)?;
        let line = self.line(index)?;
        let indent = if column == 0 { self.indent(line) } else { 0.0 };
        let icon = match line.cells.get(column) {
            Some(cell) => self.icon_space(column, cell),
            None if column == 0 && self.named_icons => ICON_SIZE + ICON_GAP,
            None => 0.0,
        };
        let x = bounds.x + CELL_PADDING + indent + icon;
        let width = (bounds.x + bounds.width - CELL_PADDING - x).max(0.0);
        Some(Rect::new(x, bounds.y, width, bounds.height))
    }

    pub(super) fn icon_space(&self, column: usize, cell: &Cell) -> f32 {
        match cell.icon.is_some() || (column == 0 && self.named_icons) {
            true => ICON_SIZE + ICON_GAP,
            false => 0.0,
        }
    }

    pub(super) fn prepare_texts(&self, cx: &mut Context, first: usize, last: usize) {
        let focused = cx.is_focused(self.id);
        let widths = self.widths();
        let mut old = std::mem::take(&mut *self.texts.borrow_mut());
        let mut texts = HashMap::with_capacity(old.len());
        for index in first..last {
            let lit = focused && !self.sidebar && self.lit(cx, index);
            let Some(line) = self.line(index) else {
                continue;
            };
            for column in 0..line.cells.len().min(self.columns.len()) {
                let key = self.text_key(cx, line, column, lit, &widths);
                if texts.contains_key(&key) {
                    continue;
                }
                let paragraph = match old.remove(&key) {
                    Some(paragraph) => paragraph,
                    None => {
                        let style = match key.label {
                            true => TextStyle::role(TextRole::Label, key.color),
                            false => TextStyle::new(TEXT_SIZE, key.color),
                        };
                        let style = match key.tabular {
                            true => style.tabular(),
                            false => style,
                        };
                        self.text(
                            &mut cx.text,
                            &key.text,
                            style,
                            f32::from_bits(key.width),
                            key.cut,
                        )
                    }
                };
                texts.insert(key, paragraph);
            }
        }
        *self.texts.borrow_mut() = texts;
    }

    pub(super) fn visible_rows(&self, cx: &Context) -> (usize, usize) {
        let offset = self.scroll.offset(cx).max(0.0);
        let first = self.first_row_below(offset).saturating_sub(1);
        let count = (self.body().height / self.row_height()).ceil() as usize + 2;
        (
            first.min(self.shown.len()),
            (first + count).min(self.shown.len()),
        )
    }

    pub(super) fn paint_header(&self, cx: &Context, items: &mut Vec<Item>) {
        let header = Rect::new(
            self.frame.x,
            self.frame.y,
            self.frame.width,
            self.header_bar(),
        );
        items.push(Item::Fill {
            shape: Shape::rect(header),
            paint: cx.palette.sheet.into(),
        });
        let shifts = self.column_shifts(cx);
        let floating = self.floating_column(cx);
        for (column, (x, width)) in self.column_edges().into_iter().enumerate() {
            if Some(column) != floating {
                self.paint_title(cx, column, x + shifts[column], width, items);
            }
        }
        items.push(Item::Line {
            from: Point::new(self.frame.x, self.frame.y + self.header_bar() - 0.5),
            to: Point::new(
                self.frame.x + self.frame.width,
                self.frame.y + self.header_bar() - 0.5,
            ),
            width: 1.0,
            paint: cx.palette.hairline.into(),
        });
    }

    fn paint_title(&self, cx: &Context, column: usize, x: f32, width: f32, items: &mut Vec<Item>) {
        let Some(paragraph) = self.titles.get(column).cloned() else {
            return;
        };
        let text_x = match self.columns[column].numeric {
            true => x + width - CELL_PADDING - paragraph.width(),
            false => x + CELL_PADDING,
        };
        let text_y = self.frame.y + ((self.header_bar() - paragraph.height()) / 2.0).round();
        if let Some(sort) = self.sort.filter(|sort| sort.column == column) {
            let arrow_x = match self.columns[column].numeric {
                true => text_x - 10.0,
                false => text_x + paragraph.width() + 6.0,
            };
            let middle = self.frame.y + self.header_bar() / 2.0;
            let (tip, base) = if sort.ascending {
                (-2.0, 2.0)
            } else {
                (2.0, -2.0)
            };
            for side in [-1.0, 1.0] {
                items.push(Item::Line {
                    from: Point::new(arrow_x + 2.5, middle + tip),
                    to: Point::new(arrow_x + 2.5 + side * 3.0, middle + base),
                    width: 1.5,
                    paint: cx.palette.ink2.into(),
                });
            }
        }
        items.push(Item::Text {
            origin: Point::new(text_x, text_y),
            paragraph,
        });
    }

    pub(super) fn paint_cell(
        &self,
        cx: &Context,
        shared: (&HashMap<TextKey, Paragraph>, &[f32]),
        line: &Line,
        column: usize,
        (x, y, lit): (f32, f32, bool),
        items: &mut Vec<Item>,
    ) {
        let (texts, widths) = shared;
        let cell = &line.cells[column];
        let mut text_x = x + CELL_PADDING;
        if column == 0 {
            text_x += self.indent(line);
        }
        if let Some(icon) = &cell.icon {
            items.push(Item::Image {
                rect: Rect::new(
                    text_x,
                    y + (self.row_height() - ICON_SIZE) / 2.0,
                    ICON_SIZE,
                    ICON_SIZE,
                ),
                image: icon.clone(),
            });
        }
        text_x += self.icon_space(column, cell);
        let key = self.text_key(cx, line, column, lit, widths);
        let Some(paragraph) = texts.get(&key) else {
            return;
        };
        if self.columns[column].numeric {
            text_x = x + widths[column] - CELL_PADDING - paragraph.width();
        }
        items.push(Item::Text {
            origin: Point::new(
                text_x,
                y + ((self.row_height() - paragraph.height()) / 2.0).round(),
            ),
            paragraph: paragraph.clone(),
        });
    }

    pub(super) fn paint_floating_column(
        &self,
        cx: &Context,
        column: usize,
        (x, width): (f32, f32),
        shift: f32,
        items: &mut Vec<Item>,
    ) {
        let x = x + shift;
        let rise = cx.sample(&self.rise);
        let body = self.body();
        let whole = Rect::new(x, self.frame.y, width, self.frame.height);
        let underline = self.frame.y + self.header_bar() - 0.5;
        let mut floating = vec![
            Item::Shadow {
                shape: Shape::rect(whole),
                offset: Point::new(0.0, 2.0),
                blur_radius: FLOATING_BLUR,
                color: Color::BLACK.with_opacity(FLOATING_SHADOW * rise),
            },
            Item::Fill {
                shape: Shape::rect(whole),
                paint: cx.palette.sheet.into(),
            },
        ];
        self.paint_title(cx, column, x, width, &mut floating);
        floating.push(Item::Line {
            from: Point::new(x, underline),
            to: Point::new(x + width, underline),
            width: 1.0,
            paint: cx.palette.hairline.into(),
        });
        let focused = cx.is_focused(self.id);
        let offset = self.scroll.offset(cx);
        let (first, last) = self.visible_rows(cx);
        let widths = self.widths();
        let texts = self.texts.borrow();
        let mut rows = Vec::new();
        self.paint_selection(cx, body, offset, focused, &mut rows);
        self.paint_headings(cx, body, offset, first, last, &mut rows);
        for index in first..last {
            let Some(line) = self.line(index).filter(|line| column < line.cells.len()) else {
                continue;
            };
            let y = body.y + self.row_top(index) - offset;
            let lit = focused && !self.sidebar && self.lit(cx, index);
            self.paint_cell(cx, (&texts, &widths), line, column, (x, y, lit), &mut rows);
        }
        floating.push(Item::Clip {
            shape: Shape::rect(Rect::new(x, body.y, width, body.height)),
            items: rows,
        });
        for edge in [x + 0.5, x + width - 0.5] {
            floating.push(Item::Line {
                from: Point::new(edge, self.frame.y),
                to: Point::new(edge, self.frame.y + self.frame.height),
                width: 1.0,
                paint: cx.palette.hairline.with_opacity(rise).into(),
            });
        }
        items.push(Item::Clip {
            shape: Shape::rect(self.frame),
            items: floating,
        });
    }

    fn runs(&self) -> Vec<(usize, usize)> {
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for (index, selected) in self.selected.iter().enumerate() {
            match runs.last_mut() {
                Some(run) if *selected && run.1 == index && !self.heads_group(index) => {
                    run.1 = index + 1;
                }
                _ if *selected => runs.push((index, index + 1)),
                _ => {}
            }
        }
        runs
    }

    pub(super) fn lit(&self, cx: &Context, index: usize) -> bool {
        match &self.glide {
            Some(glide) => {
                let centre = self.row_top(index) + self.row_height() / 2.0;
                cx.sample(&glide.top) <= centre && centre < cx.sample(&glide.bottom)
            }
            None => self.selected.get(index).copied().unwrap_or(false),
        }
    }

    pub(super) fn follow_selection(&mut self, cx: &Context) {
        let runs = self.runs();
        let [run] = runs[..] else {
            self.glide = None;
            return;
        };
        let (top, bottom) = (
            self.row_top(run.0),
            self.row_top(run.1 - 1) + self.row_height(),
        );
        match &mut self.glide {
            Some(glide) if glide.run == run => {}
            Some(glide) => {
                glide.run = run;
                cx.animate(&mut glide.top, top, Spring::QUICK);
                cx.animate(&mut glide.bottom, bottom, Spring::QUICK);
            }
            None => {
                self.glide = Some(Glide {
                    run,
                    top: Animated::points(top),
                    bottom: Animated::points(bottom),
                });
            }
        }
    }

    pub(super) fn paint_selection(
        &self,
        cx: &Context,
        body: Rect,
        offset: f32,
        focused: bool,
        items: &mut Vec<Item>,
    ) {
        let color = match (self.sidebar, focused) {
            (true, _) => cx.palette.sidebar_selection(cx.window_active, focused),
            (false, true) => cx.palette.accent,
            (false, false) => cx.palette.well,
        };
        let spans: Vec<(f32, f32)> = match &self.glide {
            Some(glide) => vec![(cx.sample(&glide.top), cx.sample(&glide.bottom))],
            None => self
                .runs()
                .into_iter()
                .map(|(first, last)| {
                    (
                        self.row_top(first),
                        self.row_top(last - 1) + self.row_height(),
                    )
                })
                .collect(),
        };
        for (top, bottom) in spans {
            let rect = Rect::new(
                body.x + SELECTION_INSET.0,
                body.y + top - offset + SELECTION_INSET.1,
                body.width - 2.0 * SELECTION_INSET.0,
                bottom - top - 2.0 * SELECTION_INSET.1,
            );
            items.push(Item::Fill {
                shape: Shape::rounded(rect, Radius::S),
                paint: color.into(),
            });
        }
    }

    pub(super) fn paint_headings(
        &self,
        cx: &Context,
        body: Rect,
        offset: f32,
        first: usize,
        last: usize,
        items: &mut Vec<Item>,
    ) {
        let Some(from) = self.shown.get(first) else {
            return;
        };
        let to = self.shown.get(last).copied().unwrap_or(usize::MAX);
        for ((start, _), paragraph) in self.headings.iter().zip(&self.heading_texts) {
            if *start < *from || *start > to {
                continue;
            }
            let Ok(index) = self.shown.binary_search(start) else {
                continue;
            };
            let top = body.y + self.row_top(index) - self.row_height() - offset;
            let bottom = top + self.row_height();
            items.push(Item::Text {
                origin: Point::new(
                    body.x + CELL_PADDING,
                    (bottom - paragraph.height() - 5.0).round(),
                ),
                paragraph: paragraph.clone(),
            });
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    body.x + CELL_PADDING,
                    bottom - 1.0,
                    body.width - 2.0 * CELL_PADDING,
                    1.0,
                )),
                paint: cx.palette.hairline.into(),
            });
        }
    }
}
