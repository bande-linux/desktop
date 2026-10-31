use std::collections::HashMap;
use std::rc::Rc;

use unbound_graphics::{
    Color, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle, TextSystem,
};

use super::model::{Cell, Line};
use super::{CELL_PADDING, Glide, ICON_SIZE, Table};
use crate::{Animated, Context, Spring};

const SELECTION_INSET: (f32, f32) = (4.0, 1.0);
const ICON_GAP: f32 = 6.0;
const TEXT_SIZE: f32 = 13.0;
const ELLIPSIS: char = '…';

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct TextKey {
    text: Rc<str>,
    column: usize,
    color: Color,
    width: u32,
    label: bool,
    tabular: bool,
}

impl Table {
    pub(super) fn text(
        &self,
        text: &mut TextSystem,
        content: &str,
        style: TextStyle,
        width: f32,
    ) -> Paragraph {
        let paragraph = text.paragraph(content, style, None, TextAlign::Start);
        if paragraph.width() <= width {
            return paragraph;
        }
        let characters = content.chars().collect::<Vec<_>>();
        let (mut low, mut high) = (0, characters.len());
        while low < high {
            let middle = (low + high).div_ceil(2);
            let candidate = characters[..middle].iter().collect::<String>() + &ELLIPSIS.to_string();
            if text
                .paragraph(&candidate, style, None, TextAlign::Start)
                .width()
                <= width
            {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        let shortened = characters[..low]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_owned()
            + &ELLIPSIS.to_string();
        text.paragraph(&shortened, style, None, TextAlign::Start)
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
        let color = match (lit, cell.tint, cell.muted) {
            (true, _, _) => cx.palette.on_accent,
            (false, Some(tint), _) => tint,
            (false, None, true) => cx.palette.ink2,
            (false, None, false) => cx.palette.ink,
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
        }
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
            let lit = focused && self.lit(cx, index);
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
                        self.text(&mut cx.text, &key.text, style, f32::from_bits(key.width))
                    }
                };
                texts.insert(key, paragraph);
            }
        }
        *self.texts.borrow_mut() = texts;
    }

    pub(super) fn visible_rows(&self, cx: &Context) -> (usize, usize) {
        let offset = self.scroll.offset(cx).max(0.0);
        let first = (offset / self.row_height()).floor() as usize;
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
        for (column, ((x, width), paragraph)) in self
            .column_edges()
            .into_iter()
            .zip(&self.titles)
            .enumerate()
        {
            let sorted = self.sort.filter(|sort| sort.column == column);
            let paragraph = paragraph.clone();
            let text_x = match self.columns[column].numeric {
                true => x + width - CELL_PADDING - paragraph.width(),
                false => x + CELL_PADDING,
            };
            let text_y = self.frame.y + ((self.header_bar() - paragraph.height()) / 2.0).round();
            if let Some(sort) = sorted {
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

    fn runs(&self) -> Vec<(usize, usize)> {
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for (index, selected) in self.selected.iter().enumerate() {
            match runs.last_mut() {
                Some(run) if *selected && run.1 == index => run.1 = index + 1,
                _ if *selected => runs.push((index, index + 1)),
                _ => {}
            }
        }
        runs
    }

    pub(super) fn lit(&self, cx: &Context, index: usize) -> bool {
        match &self.glide {
            Some(glide) => {
                let centre = (index as f32 + 0.5) * self.row_height();
                glide.top.value(cx) <= centre && centre < glide.bottom.value(cx)
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
            run.0 as f32 * self.row_height(),
            run.1 as f32 * self.row_height(),
        );
        match &mut self.glide {
            Some(glide) if glide.run == run => {}
            Some(glide) => {
                glide.run = run;
                glide.top.animate_to(cx, top, Spring::QUICK);
                glide.bottom.animate_to(cx, bottom, Spring::QUICK);
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
        let color = match focused {
            true => cx.palette.accent,
            false => cx.palette.well,
        };
        let spans: Vec<(f32, f32)> = match &self.glide {
            Some(glide) => vec![(glide.top.value(cx), glide.bottom.value(cx))],
            None => self
                .runs()
                .into_iter()
                .map(|(first, last)| {
                    (
                        first as f32 * self.row_height(),
                        last as f32 * self.row_height(),
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
}
