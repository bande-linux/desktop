use unbound_graphics::{Item, Paragraph, Point, Rect, Scene};

const SLACK_ACROSS: f32 = 1.0;
const SLACK_DOWN: f32 = 3.0;
const OVERLAP: f32 = 2.0;

#[derive(Debug, Clone, PartialEq)]
struct PlacedText {
    text: String,
    bounds: Rect,
    visible: Rect,
    clip: Rect,
}

#[derive(Clone, Copy)]
struct Placement {
    scale: f32,
    offset: Point,
    clip: Rect,
}

impl Placement {
    fn map(&self, rect: Rect) -> Rect {
        Rect::new(
            self.offset.x + rect.x * self.scale,
            self.offset.y + rect.y * self.scale,
            rect.width * self.scale,
            rect.height * self.scale,
        )
    }
}

fn intersection(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
}

fn ink(paragraph: &Paragraph, origin: Point) -> Rect {
    let mut left = f32::MAX;
    let mut right = f32::MIN;
    let mut top = f32::MAX;
    let mut bottom = f32::MIN;
    for line in paragraph.layout().lines() {
        let metrics = line.metrics();
        left = left.min(metrics.offset);
        right = right.max(metrics.offset + metrics.advance - metrics.hanging_advance);
        top = top.min(metrics.block_min_coord);
        bottom = bottom.max(metrics.block_max_coord);
    }
    if left > right {
        return Rect::new(origin.x, origin.y, 0.0, 0.0);
    }
    Rect::new(origin.x + left, origin.y + top, right - left, bottom - top)
}

fn collect(items: &[Item], placement: Placement, found: &mut Vec<PlacedText>) {
    for item in items {
        match item {
            Item::Text { origin, paragraph } => {
                let bounds = placement.map(ink(paragraph, *origin));
                if bounds.width > 0.0 && !paragraph.text().trim().is_empty() {
                    found.push(PlacedText {
                        text: paragraph.text().to_owned(),
                        bounds,
                        visible: intersection(bounds, placement.clip),
                        clip: placement.clip,
                    });
                }
            }
            Item::Clip { shape, items } => {
                let clip = intersection(placement.clip, placement.map(shape.rect));
                collect(items, Placement { clip, ..placement }, found);
            }
            Item::Translate { offset, items } => {
                let offset = Point::new(
                    placement.offset.x + offset.x * placement.scale,
                    placement.offset.y + offset.y * placement.scale,
                );
                collect(
                    items,
                    Placement {
                        offset,
                        ..placement
                    },
                    found,
                );
            }
            Item::Scale {
                origin,
                factor,
                items,
            } => {
                let offset = Point::new(
                    placement.offset.x + origin.x * (1.0 - factor) * placement.scale,
                    placement.offset.y + origin.y * (1.0 - factor) * placement.scale,
                );
                let scale = placement.scale * factor;
                collect(
                    items,
                    Placement {
                        scale,
                        offset,
                        ..placement
                    },
                    found,
                );
            }
            Item::Opacity { alpha, items } if *alpha > 0.0 => collect(items, placement, found),
            _ => {}
        }
    }
}

fn placed_texts(scene: &Scene) -> Vec<PlacedText> {
    let mut found = Vec::new();
    let placement = Placement {
        scale: 1.0,
        offset: Point::default(),
        clip: Rect::new(0.0, 0.0, scene.width as f32, scene.height as f32),
    };
    collect(&scene.items, placement, &mut found);
    found
}

pub(crate) fn clipped_texts(scene: &Scene) -> Vec<String> {
    placed_texts(scene)
        .into_iter()
        .filter(|placed| {
            let shown = placed.visible.width > 0.0 && placed.visible.height > 0.0;
            let cut_across = placed.bounds.width - placed.visible.width > SLACK_ACROSS;
            let tight = placed.clip.height < 2.0 * placed.bounds.height;
            let cut_down = tight && placed.bounds.height - placed.visible.height > SLACK_DOWN;
            shown && (cut_across || cut_down)
        })
        .map(|placed| placed.text)
        .collect()
}

pub(crate) fn overlapping_texts(scene: &Scene) -> Vec<(String, String)> {
    let placed = placed_texts(scene);
    let mut overlapping = Vec::new();
    for (index, first) in placed.iter().enumerate() {
        for second in &placed[index + 1..] {
            let shared = intersection(first.visible, second.visible);
            if shared.width > OVERLAP && shared.height > OVERLAP + SLACK_DOWN {
                overlapping.push((first.text.clone(), second.text.clone()));
            }
        }
    }
    overlapping
}

pub(crate) fn untranslated_texts(scene: &Scene, data: &[&str]) -> Vec<String> {
    placed_texts(scene)
        .into_iter()
        .map(|placed| placed.text)
        .filter(|text| {
            let rest = data
                .iter()
                .fold(text.clone(), |rest, known| rest.replace(known, ""));
            rest.split(|character: char| !character.is_ascii_alphabetic())
                .any(|word| word.len() > 1)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_graphics::{Color, Shape, TextAlign, TextStyle, TextSystem};

    fn text(system: &mut TextSystem, words: &str, at: Point) -> Item {
        Item::Text {
            origin: at,
            paragraph: system.paragraph(
                words,
                TextStyle::new(13.0, Color::BLACK),
                None,
                TextAlign::Start,
            ),
        }
    }

    fn scene(items: Vec<Item>) -> Scene {
        Scene {
            width: 200,
            height: 100,
            background: Color::WHITE,
            items,
        }
    }

    #[test]
    fn text_cut_by_its_clip_or_the_window_edge_is_reported_but_not_text_scrolled_away() {
        let mut system = TextSystem::new();
        let fits = text(&mut system, "Fits", Point::new(4.0, 4.0));
        let cut = Item::Clip {
            shape: Shape::rect(Rect::new(0.0, 40.0, 30.0, 30.0)),
            items: vec![text(&mut system, "Much too long", Point::new(4.0, 44.0))],
        };
        let shifted = Item::Translate {
            offset: Point::new(170.0, 0.0),
            items: vec![text(&mut system, "Past the edge", Point::new(0.0, 4.0))],
        };
        let scrolled = Item::Clip {
            shape: Shape::rect(Rect::new(0.0, 0.0, 200.0, 80.0)),
            items: vec![
                text(&mut system, "Below the fold", Point::new(4.0, 76.0)),
                text(&mut system, "Far below", Point::new(4.0, 300.0)),
            ],
        };
        assert_eq!(
            clipped_texts(&scene(vec![fits, cut, shifted, scrolled])),
            ["Much too long", "Past the edge"]
        );
    }

    #[test]
    fn texts_drawn_over_each_other_are_reported() {
        let mut system = TextSystem::new();
        let items = vec![
            text(&mut system, "Label", Point::new(4.0, 4.0)),
            text(&mut system, "Value", Point::new(20.0, 4.0)),
            text(&mut system, "Below", Point::new(4.0, 40.0)),
        ];
        assert_eq!(
            overlapping_texts(&scene(items)),
            [("Label".to_owned(), "Value".to_owned())]
        );
    }

    #[test]
    fn plain_english_outside_the_known_data_is_untranslated() {
        let mut system = TextSystem::new();
        let items = vec![
            text(&mut system, "Ŵīīƒīī: HomeNet", Point::new(4.0, 4.0)),
            text(&mut system, "Cancel", Point::new(4.0, 30.0)),
            text(&mut system, "12:30", Point::new(4.0, 60.0)),
        ];
        assert_eq!(untranslated_texts(&scene(items), &["HomeNet"]), ["Cancel"]);
    }
}
