use unbound_toolkit::ui::{Event, Key};

use crate::engine::Frame;
use unbound_toolkit::{
    Color, Item, Palette, Point, Radius, Rect, Scene, Scheme, ScreenImage, Shape, Spacing,
    TextAlign, TextRole, TextStyle, TextSystem,
};

const SELECTION_FILL: f32 = 0.16;
const WINDOW_TINT: f32 = 0.3;
const CROSSHAIR_OUTLINE: f32 = 0.5;
const CROSSHAIR_ARM: f32 = 10.0;
const READOUT_OFFSET: f32 = 14.0;
const READOUT_PADDING: f32 = Spacing::XS as f32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AreaOutcome {
    Selecting,
    Cancelled,
    Chosen(Region),
    ChosenWindow(String),
}

#[derive(Debug, Default)]
pub struct AreaSelection {
    anchor: Option<Point>,
    pointer: Option<Point>,
    windows: Vec<(String, Frame)>,
    window_mode: bool,
}

fn pixel(coordinate: f32) -> u32 {
    coordinate.max(0.0).round() as u32
}

impl AreaSelection {
    pub fn new(windows: Vec<(String, Frame)>, window_mode: bool) -> Self {
        Self {
            window_mode: window_mode && !windows.is_empty(),
            windows,
            ..Self::default()
        }
    }

    fn hovered(&self) -> Option<&(String, Frame)> {
        let at = self.pointer?;
        self.windows.iter().find(|(_, frame)| {
            at.x >= frame.x as f32
                && at.y >= frame.y as f32
                && at.x < (frame.x + frame.width) as f32
                && at.y < (frame.y + frame.height) as f32
        })
    }

    pub fn handle(&mut self, event: &Event) -> AreaOutcome {
        match event {
            Event::Key {
                key: Key::Escape, ..
            } => AreaOutcome::Cancelled,
            Event::Key {
                key: Key::Space, ..
            } if self.anchor.is_none() && !self.windows.is_empty() => {
                self.window_mode = !self.window_mode;
                AreaOutcome::Selecting
            }
            Event::PointerDown(pointer) if pointer.is_primary() && self.window_mode => {
                self.pointer = Some(pointer.position);
                match self.hovered() {
                    Some((identifier, _)) => AreaOutcome::ChosenWindow(identifier.clone()),
                    None => AreaOutcome::Selecting,
                }
            }
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

    pub fn scene(
        &self,
        width: u32,
        height: u32,
        palette: &Palette,
        text: &mut TextSystem,
    ) -> Scene {
        let mut items = Vec::new();
        if self.window_mode {
            if let Some((_, frame)) = self.hovered() {
                items.push(Item::Fill {
                    shape: Shape::rounded(
                        Rect::new(
                            frame.x as f32,
                            frame.y as f32,
                            frame.width as f32,
                            frame.height as f32,
                        ),
                        Radius::XL,
                    ),
                    paint: palette.accent.with_opacity(WINDOW_TINT).into(),
                });
            }
            return Scene {
                width,
                height,
                background: Color::TRANSPARENT,
                items,
            };
        }
        if let Some(region) = self.region() {
            let frame = Rect::new(
                region.x as f32,
                region.y as f32,
                region.width as f32,
                region.height as f32,
            );
            items.push(Item::Fill {
                shape: Shape::rect(frame),
                paint: palette.accent.with_opacity(SELECTION_FILL).into(),
            });
            items.push(Item::Stroke {
                shape: Shape::rect(frame.inset(0.5)),
                width: 1.0,
                paint: palette.accent.into(),
            });
        }
        if let Some(at) = self.pointer {
            crosshair(&mut items, at);
            let label = match self.region() {
                Some(region) => format!("{} × {}", region.width, region.height),
                None => format!("{} , {}", pixel(at.x), pixel(at.y)),
            };
            let style = TextStyle::role(TextRole::Callout, palette.chassis).tabular();
            let paragraph = text.paragraph(&label, style, None, TextAlign::Start);
            let chip = Rect::new(
                at.x + READOUT_OFFSET,
                at.y + READOUT_OFFSET,
                (paragraph.width() + 2.0 * Spacing::S as f32).ceil(),
                (paragraph.height() + 2.0 * READOUT_PADDING).ceil(),
            );
            items.push(Item::Fill {
                shape: Shape::rounded(chip, Radius::S),
                paint: palette.ink.into(),
            });
            items.push(Item::Text {
                origin: Point::new(chip.x + Spacing::S as f32, chip.y + READOUT_PADDING),
                paragraph,
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
    let light = Palette::for_scheme(Scheme::Dark).ink;
    let dark = Palette::for_scheme(Scheme::Light)
        .ink
        .with_opacity(CROSSHAIR_OUTLINE);
    let arms = [
        Rect::new(at.x - CROSSHAIR_ARM, at.y - 0.5, CROSSHAIR_ARM * 2.0, 1.0),
        Rect::new(at.x - 0.5, at.y - CROSSHAIR_ARM, 1.0, CROSSHAIR_ARM * 2.0),
    ];
    for arm in arms {
        let outline = Rect::new(arm.x - 1.0, arm.y - 1.0, arm.width + 2.0, arm.height + 2.0);
        items.push(Item::Fill {
            shape: Shape::rect(outline),
            paint: dark.into(),
        });
    }
    for arm in arms {
        items.push(Item::Fill {
            shape: Shape::rect(arm),
            paint: light.into(),
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

    #[test]
    fn a_drag_either_way_chooses_the_rectangle_between_press_and_release_and_a_click_cancels() {
        let chosen = AreaOutcome::Chosen(Region {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        });
        for (from, to, expected) in [
            ((10.0, 20.0), (110.0, 70.0), chosen.clone()),
            ((110.0, 70.0), (10.0, 20.0), chosen),
            ((40.0, 40.0), (40.0, 40.0), AreaOutcome::Cancelled),
        ] {
            let mut selection = AreaSelection::default();
            selection.handle(&Event::PointerDown(Pointer::at(Point::new(from.0, from.1))));
            selection.handle(&Event::PointerMove(Pointer::at(Point::new(to.0, to.1))));
            let outcome = selection.handle(&Event::PointerUp(Pointer::at(Point::new(to.0, to.1))));
            assert_eq!(outcome, expected, "{from:?} to {to:?}");
        }
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

    fn windows() -> Vec<(String, Frame)> {
        let frame = |x, y| Frame {
            x,
            y,
            width: 100,
            height: 80,
        };
        vec![
            ("front".to_owned(), frame(50, 50)),
            ("back".to_owned(), frame(0, 0)),
        ]
    }

    fn space() -> Event {
        Event::Key {
            key: Key::Space,
            modifiers: Modifiers::default(),
        }
    }

    #[test]
    fn space_switches_to_choosing_the_frontmost_window_under_the_pointer() {
        let mut selection = AreaSelection::new(windows(), false);
        selection.handle(&space());

        let outcome = selection.handle(&Event::PointerDown(Pointer::at(Point::new(60.0, 60.0))));

        assert_eq!(outcome, AreaOutcome::ChosenWindow("front".to_owned()));
    }

    #[test]
    fn space_again_goes_back_to_dragging_out_an_area() {
        let mut selection = AreaSelection::new(windows(), true);
        selection.handle(&space());

        selection.handle(&Event::PointerDown(Pointer::at(Point::new(10.0, 10.0))));
        let outcome = selection.handle(&Event::PointerUp(Pointer::at(Point::new(30.0, 40.0))));

        assert_eq!(
            outcome,
            AreaOutcome::Chosen(Region {
                x: 10,
                y: 10,
                width: 20,
                height: 30
            })
        );
    }

    #[test]
    fn a_click_beside_every_window_chooses_nothing() {
        let mut selection = AreaSelection::new(windows(), true);

        let outcome = selection.handle(&Event::PointerDown(Pointer::at(Point::new(400.0, 400.0))));

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
