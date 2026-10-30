use unbound_toolkit::ui::{Event, Key};
use unbound_toolkit::{
    Color, Item, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem, Typeface,
};

use crate::engine::Shot;

const LOUPE_CELLS: i32 = 9;
const LOUPE_CELL: f32 = 9.0;
const LOUPE_OFFSET: f32 = 24.0;
const LABEL_SIZE: f32 = 12.0;

pub struct ColorPicker {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    scale: f64,
    pointer: Option<Point>,
}

pub type Picked = Option<(f64, f64, f64)>;

impl ColorPicker {
    pub fn new(shot: Shot, scale: f64) -> Self {
        Self {
            width: shot.width,
            height: shot.height,
            rgba: shot.rgba(),
            scale,
            pointer: None,
        }
    }

    fn pixel(&self, x: i32, y: i32) -> Option<[u8; 3]> {
        let (x, y) = (u32::try_from(x).ok()?, u32::try_from(y).ok()?);
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = ((y * self.width + x) * 4) as usize;
        Some([self.rgba[at], self.rgba[at + 1], self.rgba[at + 2]])
    }

    fn under(&self, point: Point) -> (i32, i32) {
        (
            (f64::from(point.x) * self.scale).floor() as i32,
            (f64::from(point.y) * self.scale).floor() as i32,
        )
    }

    pub fn handle(&mut self, event: &Event) -> Option<Picked> {
        match event {
            Event::Key {
                key: Key::Escape, ..
            } => Some(None),
            Event::PointerMove(pointer) | Event::PointerEnter(pointer) => {
                self.pointer = Some(pointer.position);
                None
            }
            Event::PointerDown(pointer) if pointer.is_primary() => {
                let (x, y) = self.under(pointer.position);
                Some(self.pixel(x, y).map(|[r, g, b]| {
                    (
                        f64::from(r) / 255.0,
                        f64::from(g) / 255.0,
                        f64::from(b) / 255.0,
                    )
                }))
            }
            _ => None,
        }
    }

    pub fn scene(&self, width: u32, height: u32, text: &mut TextSystem) -> Scene {
        let mut items = Vec::new();
        if let Some(at) = self.pointer {
            let (x, y) = self.under(at);
            let side = LOUPE_CELL * LOUPE_CELLS as f32;
            let origin = Point::new(at.x + LOUPE_OFFSET, at.y + LOUPE_OFFSET);
            let loupe = Rect::new(origin.x, origin.y, side, side);
            let mut cells = Vec::new();
            for row in 0..LOUPE_CELLS {
                for column in 0..LOUPE_CELLS {
                    let [r, g, b] = self
                        .pixel(x + column - LOUPE_CELLS / 2, y + row - LOUPE_CELLS / 2)
                        .unwrap_or([0, 0, 0]);
                    cells.push(Item::Fill {
                        shape: Shape::rect(Rect::new(
                            origin.x + column as f32 * LOUPE_CELL,
                            origin.y + row as f32 * LOUPE_CELL,
                            LOUPE_CELL,
                            LOUPE_CELL,
                        )),
                        paint: Color { r, g, b, a: 0xff }.into(),
                    });
                }
            }
            items.push(Item::Shadow {
                shape: Shape::rounded(loupe, side / 2.0),
                offset: Point::new(0.0, 2.0),
                blur_radius: 8.0,
                color: Color::hex(0x000000).with_alpha(90),
            });
            items.push(Item::Clip {
                shape: Shape::rounded(loupe, side / 2.0),
                items: cells,
            });
            let middle = Rect::new(
                origin.x + (LOUPE_CELLS / 2) as f32 * LOUPE_CELL,
                origin.y + (LOUPE_CELLS / 2) as f32 * LOUPE_CELL,
                LOUPE_CELL,
                LOUPE_CELL,
            );
            items.push(Item::Stroke {
                shape: Shape::rect(middle),
                width: 1.0,
                paint: Color::hex(0xffffff).into(),
            });
            items.push(Item::Stroke {
                shape: Shape::rounded(loupe, side / 2.0),
                width: 2.0,
                paint: Color::hex(0xffffff).with_alpha(0xe0).into(),
            });
            if let Some([r, g, b]) = self.pixel(x, y) {
                let style = TextStyle {
                    size: LABEL_SIZE,
                    weight: TextStyle::MEDIUM,
                    color: Color::hex(0xffffff),
                    typeface: Typeface::Interface,
                    tabular: true,
                };
                let paragraph = text.paragraph(
                    &format!("#{r:02X}{g:02X}{b:02X}"),
                    style,
                    None,
                    TextAlign::Start,
                );
                let label = Rect::new(
                    origin.x + (side - paragraph.width()) / 2.0 - 6.0,
                    origin.y + side + 6.0,
                    paragraph.width() + 12.0,
                    paragraph.height() + 4.0,
                );
                items.push(Item::Fill {
                    shape: Shape::rounded(label, 4.0),
                    paint: Color::hex(0x000000).with_alpha(0xb0).into(),
                });
                items.push(Item::Text {
                    origin: Point::new(label.x + 6.0, label.y + 2.0),
                    paragraph,
                });
            }
        }
        Scene {
            width,
            height,
            background: Color::TRANSPARENT,
            items,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_toolkit::ui::Pointer;

    #[test]
    fn a_click_picks_the_color_under_the_pointer_at_the_screen_scale() {
        let bgra = [[0, 0, 0, 0xff], [0x00, 0x80, 0xff, 0xff]].repeat(2);
        let shot = Shot {
            width: 2,
            height: 2,
            bgra: bgra.concat(),
        };
        let mut picker = ColorPicker::new(shot, 2.0);

        let picked = picker.handle(&Event::PointerDown(Pointer::at(Point::new(0.6, 0.2))));

        assert_eq!(picked, Some(Some((1.0, 128.0 / 255.0, 0.0))));
    }

    #[test]
    fn escape_picks_nothing() {
        let shot = Shot {
            width: 1,
            height: 1,
            bgra: vec![0, 0, 0, 0xff],
        };
        let mut picker = ColorPicker::new(shot, 1.0);

        let picked = picker.handle(&Event::Key {
            key: Key::Escape,
            modifiers: Default::default(),
        });

        assert_eq!(picked, Some(None));
    }
}
