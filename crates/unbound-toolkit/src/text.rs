use ab_glyph::{Font as _, FontVec, GlyphId, InvalidFont, PxScale, ScaleFont as _, point};

pub struct Font {
    inner: FontVec,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PlacedGlyph {
    pub id: GlyphId,
    pub x: i32,
    pub baseline: i32,
}

pub(crate) struct Coverage {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
}

impl Font {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, InvalidFont> {
        FontVec::try_from_vec(bytes).map(|inner| Self { inner })
    }

    pub(crate) fn layout(&self, text: &str, size: f32, x: i32, baseline: i32) -> Vec<PlacedGlyph> {
        let scaled = self.inner.as_scaled(PxScale::from(size));
        let mut pen = x as f32;
        let mut previous = None;
        text.chars()
            .map(|character| {
                let id = scaled.glyph_id(character);
                if let Some(previous) = previous {
                    pen += scaled.kern(previous, id);
                }
                previous = Some(id);
                let glyph = PlacedGlyph {
                    id,
                    x: pen.round() as i32,
                    baseline,
                };
                pen += scaled.h_advance(id);
                glyph
            })
            .collect()
    }

    pub(crate) fn coverage(&self, id: GlyphId, size: f32) -> Option<Coverage> {
        let glyph = id.with_scale_and_position(PxScale::from(size), point(0.0, 0.0));
        let outlined = self.inner.outline_glyph(glyph)?;
        let bounds = outlined.px_bounds();
        let width = bounds.width() as u32;
        let height = bounds.height() as u32;
        let mut alpha = vec![0u8; (width * height) as usize];
        outlined.draw(|x, y, coverage| {
            if let Some(texel) = alpha.get_mut((y * width + x) as usize) {
                *texel = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        });
        Some(Coverage {
            left: bounds.min.x as i32,
            top: bounds.min.y as i32,
            width,
            height,
            alpha,
        })
    }
}
