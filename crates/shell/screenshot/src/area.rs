use unbound_toolkit::ui::{Event, Key};
use unbound_toolkit::{
    Color, Item, Paint, Point, Rect, Scene, ScreenImage, Shape, TextAlign, TextStyle, TextSystem,
    Typeface,
};

const SELECTION_FILL: Color = Color::hex(0x808080).with_alpha(0x40);
const SELECTION_EDGE: Color = Color::hex(0xffffff).with_alpha(0xc0);
const CROSSHAIR: Color = Color::hex(0xffffff);
const CROSSHAIR_OUTLINE: Color = Color::hex(0x000000).with_alpha(0x80);
const CROSSHAIR_ARM: f32 = 10.0;
const READOUT_OFFSET: f32 = 14.0;
const READOUT_SIZE: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaOutcome {
    Selecting,
    Cancelled,
    Chosen(Region),
}

#[derive(Debug, Default)]
pub struct AreaSelection {
    anchor: Option<Point>,
    pointer: Option<Point>,
}

fn pixel(coordinate: f32) -> u32 {
    coordinate.max(0.0).round() as u32
}

impl AreaSelection {
    pub fn handle(&mut self, event: &Event) -> AreaOutcome {
        match event {
            Event::Key {
                key: Key::Escape, ..
            } => AreaOutcome::Cancelled,
            Event::PointerDown(pointer) if pointer.is_primary() => {
                self.anchor = Some(pointer.position);
                self.pointer = Some(pointer.position);
                AreaOutcome::Selecting
            }
            Event::PointerMove(pointer) => {
                self.pointer = Some(pointer.position);
                AreaOutcome::Selecting
            }
            Event::PointerUp(pointer) if self.anchor.is_some() => {
                self.pointer = Some(pointer.position);
                match self.region() {
                    Some(region) if region.width > 0 && region.height > 0 => {
                        AreaOutcome::Chosen(region)
                    }
                    _ => AreaOutcome::Cancelled,
                }
            }
            _ => AreaOutcome::Selecting,
        }
    }

    pub fn region(&self) -> Option<Region> {
        let (anchor, pointer) = (self.anchor?, self.pointer?);
        let (left, right) = (
            pixel(anchor.x.min(pointer.x)),
            pixel(anchor.x.max(pointer.x)),
        );
        let (top, bottom) = (
            pixel(anchor.y.min(pointer.y)),
            pixel(anchor.y.max(pointer.y)),
        );
        Some(Region {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        })
    }

    pub fn scene(&self, width: u32, height: u32, text: &mut TextSystem) -> Scene {
        let mut items = Vec::new();
        if let Some(region) = self.region() {
            let frame = Rect::new(
                region.x as f32,
                region.y as f32,
                region.width as f32,
                region.height as f32,
            );
            items.push(Item::Fill {
                shape: Shape::rect(frame),
                paint: SELECTION_FILL.into(),
            });
            items.push(Item::Stroke {
                shape: Shape::rect(frame),
                width: 1.0,
                paint: SELECTION_EDGE.into(),
            });
        }
        if let Some(at) = self.pointer {
            crosshair(&mut items, at);
            let label = match self.region() {
                Some(region) => format!("{} × {}", region.width, region.height),
                None => format!("{} , {}", pixel(at.x), pixel(at.y)),
            };
            let style = TextStyle {
                size: READOUT_SIZE,
                weight: TextStyle::REGULAR,
                color: CROSSHAIR,
                typeface: Typeface::Interface,
                tabular: true,
            };
            items.push(Item::Text {
                origin: Point::new(at.x + READOUT_OFFSET, at.y + READOUT_OFFSET),
                paragraph: text.paragraph(&label, style, None, TextAlign::Start),
            });
        }
        Scene {
            width,
            height,
            background: Color::TRANSPARENT,
            items,
        }
    }
}

fn crosshair(items: &mut Vec<Item>, at: Point) {
    let arms = [
        Rect::new(at.x - CROSSHAIR_ARM, at.y - 0.5, CROSSHAIR_ARM * 2.0, 1.0),
        Rect::new(at.x - 0.5, at.y - CROSSHAIR_ARM, 1.0, CROSSHAIR_ARM * 2.0),
    ];
    for arm in arms {
        let outline = Rect::new(arm.x - 1.0, arm.y - 1.0, arm.width + 2.0, arm.height + 2.0);
        items.push(Item::Fill {
            shape: Shape::rect(outline),
            paint: Paint::Solid(CROSSHAIR_OUTLINE),
        });
    }
    for arm in arms {
        items.push(Item::Fill {
            shape: Shape::rect(arm),
            paint: Paint::Solid(CROSSHAIR),
        });
    }
}

pub fn crop(image: &ScreenImage, region: Region) -> Option<ScreenImage> {
    let right = region.x.checked_add(region.width)?;
    let bottom = region.y.checked_add(region.height)?;
    if region.width == 0 || region.height == 0 || right > image.width || bottom > image.height {
        return None;
    }
    let row = image.width as usize * 4;
    let rgba = (region.y..bottom)
        .flat_map(|y| {
            let start = y as usize * row + region.x as usize * 4;
            image.rgba[start..start + region.width as usize * 4]
                .iter()
                .copied()
        })
        .collect();
    Some(ScreenImage {
        width: region.width,
        height: region.height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_toolkit::ui::{Modifiers, Pointer};

    fn drag(from: (f32, f32), to: (f32, f32)) -> (AreaSelection, AreaOutcome) {
        let mut selection = AreaSelection::default();
        selection.handle(&Event::PointerDown(Pointer::at(Point::new(from.0, from.1))));
        selection.handle(&Event::PointerMove(Pointer::at(Point::new(to.0, to.1))));
        let outcome = selection.handle(&Event::PointerUp(Pointer::at(Point::new(to.0, to.1))));
        (selection, outcome)
    }

    #[test]
    fn dragging_chooses_the_rectangle_between_press_and_release() {
        let (_, outcome) = drag((10.0, 20.0), (110.0, 70.0));

        assert_eq!(
            outcome,
            AreaOutcome::Chosen(Region {
                x: 10,
                y: 20,
                width: 100,
                height: 50
            })
        );
    }

    #[test]
    fn dragging_up_and_left_chooses_the_same_rectangle() {
        let (_, outcome) = drag((110.0, 70.0), (10.0, 20.0));

        assert_eq!(
            outcome,
            AreaOutcome::Chosen(Region {
                x: 10,
                y: 20,
                width: 100,
                height: 50
            })
        );
    }

    #[test]
    fn a_click_without_a_drag_takes_no_picture() {
        let (_, outcome) = drag((40.0, 40.0), (40.0, 40.0));

        assert_eq!(outcome, AreaOutcome::Cancelled);
    }

    #[test]
    fn escape_cancels_as_on_a_mac() {
        let mut selection = AreaSelection::default();
        selection.handle(&Event::PointerDown(Pointer::at(Point::new(5.0, 5.0))));
        selection.handle(&Event::PointerMove(Pointer::at(Point::new(50.0, 50.0))));

        let outcome = selection.handle(&Event::Key {
            key: Key::Escape,
            modifiers: Modifiers::default(),
        });

        assert_eq!(outcome, AreaOutcome::Cancelled);
    }

    #[test]
    fn a_release_without_a_press_keeps_selecting() {
        let mut selection = AreaSelection::default();

        let outcome = selection.handle(&Event::PointerUp(Pointer::at(Point::new(5.0, 5.0))));

        assert_eq!(outcome, AreaOutcome::Selecting);
    }

    #[test]
    fn cropping_keeps_the_chosen_pixels() {
        let pixel = |x: u8, y: u8| [x, y, 0, 0xff];
        let rgba = (0..3u8)
            .flat_map(|y| (0..4u8).flat_map(move |x| pixel(x, y)))
            .collect();
        let image = ScreenImage {
            width: 4,
            height: 3,
            rgba,
        };

        let cropped = crop(
            &image,
            Region {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            },
        );

        let expected = [pixel(1, 1), pixel(2, 1), pixel(1, 2), pixel(2, 2)].concat();
        assert_eq!(
            cropped,
            Some(ScreenImage {
                width: 2,
                height: 2,
                rgba: expected
            })
        );
    }

    #[test]
    fn a_region_outside_the_screen_cannot_be_cropped() {
        let image = ScreenImage {
            width: 4,
            height: 3,
            rgba: vec![0; 48],
        };

        let outside = Region {
            x: 3,
            y: 0,
            width: 2,
            height: 1,
        };

        assert_eq!(crop(&image, outside), None);
    }
}
