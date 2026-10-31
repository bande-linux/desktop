use smithay::utils::{Logical, Point, Rectangle, Size};

use super::{Frame, GAP, MARGIN, NAME_ROW, PAD, THUMBNAIL, THUMBNAIL_INSET};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Layout {
    pub(super) card: Frame,
    pub(super) tiles: Vec<Frame>,
    pub(super) contents: Vec<Frame>,
    pub(super) name: Frame,
}

pub(super) fn lay_out(tiles: &[Size<f64, Logical>], inset: f64, room: Frame) -> Layout {
    let count = tiles.len() as f64;
    let widths: f64 = tiles.iter().map(|tile| tile.w).sum();
    let tallest = tiles.iter().map(|tile| tile.h).fold(0.0, f64::max);
    let spacing = 2.0 * PAD + (count - 1.0).max(0.0) * GAP;
    let fit = ((room.size.w - 2.0 * MARGIN - spacing) / widths.max(1.0)).clamp(0.0, 1.0);
    let row = tallest * fit;
    let size = Size::from((
        (widths * fit + spacing).round(),
        (PAD + row + NAME_ROW).round(),
    ));
    let loc = Point::from((
        (room.loc.x + (room.size.w - size.w) / 2.0).round(),
        (room.loc.y + (room.size.h - size.h) / 2.0).round(),
    ));
    let card = Rectangle::new(loc, size);
    let mut x = loc.x + PAD;
    let mut placed = Vec::new();
    let mut contents = Vec::new();
    for tile in tiles {
        let tile = tile.upscale(fit);
        let rect = Rectangle::new((x, loc.y + PAD + (row - tile.h) / 2.0).into(), tile);
        let inner = inset * fit;
        contents.push(Rectangle::new(
            rect.loc + Point::from((inner, inner)),
            (tile.w - 2.0 * inner, tile.h - 2.0 * inner).into(),
        ));
        placed.push(rect);
        x += tile.w + GAP;
    }
    Layout {
        card,
        tiles: placed,
        contents,
        name: Rectangle::new(
            (loc.x + PAD, loc.y + PAD + row).into(),
            (size.w - 2.0 * PAD, NAME_ROW).into(),
        ),
    }
}

pub(super) fn thumbnail_tile(window: Size<f64, Logical>) -> Size<f64, Logical> {
    let fit = (THUMBNAIL.0 / window.w.max(1.0))
        .min(THUMBNAIL.1 / window.h.max(1.0))
        .min(1.0);
    Size::from((
        window.w * fit + 2.0 * THUMBNAIL_INSET,
        window.h * fit + 2.0 * THUMBNAIL_INSET,
    ))
}

pub(super) fn fitted(rect: Frame, layout: &Frame, shown: &Frame) -> Frame {
    let factor = (shown.size.w / layout.size.w.max(1.0)).min(shown.size.h / layout.size.h.max(1.0));
    let center = |frame: &Frame| frame.loc + frame.size.downscale(2.0).to_point();
    Rectangle::new(
        center(shown) + (rect.loc - center(layout)).upscale(factor),
        rect.size.upscale(factor),
    )
}

pub(super) fn step(index: usize, count: usize, backwards: bool) -> usize {
    if count == 0 {
        0
    } else if backwards {
        (index + count - 1) % count
    } else {
        (index + 1) % count
    }
}
