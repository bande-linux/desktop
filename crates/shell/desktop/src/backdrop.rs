use std::path::PathBuf;

use unbound_spring::{Animation, Spring, State};
use unbound_toolkit::{Color, Image, Item, Rect};
use unbound_wallpaper::{Fit, Picture};

use crate::model::Bounds;

const LUMA_CELL: u32 = 16;
const LIGHT_LUMA: u32 = 150;
const FADE_FREQUENCY: f64 = 14.0;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub path: PathBuf,
    pub fit: Fit,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Luma {
    columns: u32,
    rows: u32,
    cells: Vec<u8>,
}

impl Luma {
    pub fn of(picture: &Picture) -> Self {
        let columns = picture.width.div_ceil(LUMA_CELL).max(1);
        let rows = picture.height.div_ceil(LUMA_CELL).max(1);
        let mut sums = vec![(0u64, 0u64); (columns * rows) as usize];
        for (index, pixel) in picture.rgba.chunks_exact(4).enumerate() {
            let index = index as u32;
            let (x, y) = (index % picture.width, index / picture.width);
            let cell = ((y / LUMA_CELL) * columns + x / LUMA_CELL) as usize;
            let luma =
                2126 * u64::from(pixel[0]) + 7152 * u64::from(pixel[1]) + 722 * u64::from(pixel[2]);
            if let Some(sum) = sums.get_mut(cell) {
                sum.0 += luma / 10_000;
                sum.1 += 1;
            }
        }
        Self {
            columns,
            rows,
            cells: sums
                .into_iter()
                .map(|(sum, count)| (sum / count.max(1)) as u8)
                .collect(),
        }
    }

    pub fn is_light(&self, area: Bounds, scale: f32) -> bool {
        let cell = |value: f32, count: u32| {
            ((value * scale / LUMA_CELL as f32).floor().max(0.0) as u32).min(count - 1)
        };
        let (left, right) = (
            cell(area.x, self.columns),
            cell(area.x + area.width, self.columns),
        );
        let (top, bottom) = (
            cell(area.y, self.rows),
            cell(area.y + area.height, self.rows),
        );
        let mut sum = 0u32;
        let mut count = 0u32;
        for row in top..=bottom {
            for column in left..=right {
                sum += u32::from(self.cells[(row * self.columns + column) as usize]);
                count += 1;
            }
        }
        sum / count.max(1) >= LIGHT_LUMA
    }
}

#[derive(Debug, Clone)]
pub struct Layer {
    pub key: Key,
    pub image: Image,
    pub luma: Luma,
}

pub struct Backdrop {
    current: Option<Layer>,
    previous: Option<Layer>,
    fade: Animation,
}

fn fade_spring() -> Spring {
    Spring::new(FADE_FREQUENCY, 1.0).unwrap_or_else(|_| Spring::default())
}

impl Default for Backdrop {
    fn default() -> Self {
        Self {
            current: None,
            previous: None,
            fade: Animation::new(fade_spring(), State::default(), 1.0),
        }
    }
}

impl Backdrop {
    pub fn key(&self) -> Option<&Key> {
        self.current.as_ref().map(|layer| &layer.key)
    }

    pub fn show(&mut self, layer: Layer, animated: bool) {
        if self.key() == Some(&layer.key) {
            return;
        }
        self.previous = self.current.replace(layer);
        let start = if animated && self.previous.is_some() || self.previous.is_none() && animated {
            0.0
        } else {
            1.0
        };
        self.fade = Animation::new(
            fade_spring(),
            State {
                position: start,
                velocity: 0.0,
            },
            1.0,
        );
    }

    pub fn advance(&mut self, seconds: f64) {
        self.fade.advance(seconds);
        if self.is_settled() {
            self.previous = None;
        }
    }

    pub fn is_settled(&self) -> bool {
        self.fade.is_settled(0.002, 0.01)
    }

    pub fn is_light(&self, area: Bounds, scale: f32) -> Option<bool> {
        self.current
            .as_ref()
            .map(|layer| layer.luma.is_light(area, scale))
    }

    pub fn paint(&self, rect: Rect, items: &mut Vec<Item>) {
        if let Some(previous) = &self.previous {
            items.push(Item::Image {
                rect,
                image: previous.image.clone(),
            });
        }
        if let Some(current) = &self.current {
            let alpha = self.fade.state().position.clamp(0.0, 1.0) as f32;
            let image = Item::Image {
                rect,
                image: current.image.clone(),
            };
            items.push(if alpha >= 1.0 {
                image
            } else {
                Item::Opacity {
                    alpha,
                    items: vec![image],
                }
            });
        }
    }
}

pub fn placeholder(dark: bool) -> Color {
    if dark {
        Color::hex(0x141826)
    } else {
        Color::hex(0xE8ECF3)
    }
}
