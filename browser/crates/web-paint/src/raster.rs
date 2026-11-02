use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{self, Affine};
use vello_cpu::{Pixmap, RenderContext, Resources};
use web_style::Color;

use crate::{DisplayList, Item};

fn color(color: Color) -> AlphaColor<Srgb> {
    AlphaColor::from_rgba8(color.r, color.g, color.b, color.a)
}

pub(crate) fn rasterize(list: &DisplayList, scroll_y: f32, width: u16, height: u16) -> Vec<u8> {
    let mut context = RenderContext::new(width, height);
    let mut resources = Resources::new();
    context.set_paint(color(list.background));
    context.fill_rect(&kurbo::Rect::new(
        0.0,
        0.0,
        f64::from(width),
        f64::from(height),
    ));
    context.set_transform(Affine::translate((0.0, -f64::from(scroll_y))));
    let visible = (scroll_y, scroll_y + f32::from(height));
    for item in &list.items {
        match item {
            Item::Rect { rect, color: fill } => {
                if rect.y > visible.1 || rect.y + rect.height < visible.0 {
                    continue;
                }
                context.set_paint(color(*fill));
                context.fill_rect(&kurbo::Rect::new(
                    f64::from(rect.x),
                    f64::from(rect.y),
                    f64::from(rect.x + rect.width),
                    f64::from(rect.y + rect.height),
                ));
            }
            Item::Glyphs {
                rect,
                font,
                size,
                color: fill,
                glyphs,
                ..
            } => {
                if rect.y > visible.1 || rect.y + rect.height < visible.0 {
                    continue;
                }
                let Some(font) = list.fonts.get(*font as usize) else {
                    continue;
                };
                context.set_paint(color(*fill));
                let glyphs = list.glyphs(glyphs).iter().map(|glyph| vello_cpu::Glyph {
                    id: glyph.id,
                    x: glyph.x,
                    y: glyph.y,
                });
                let _ = context
                    .glyph_run(&mut resources, font)
                    .font_size(*size)
                    .fill_glyphs(glyphs);
            }
        }
    }
    context.flush();
    let mut pixmap = Pixmap::new(width, height);
    context.render(&mut pixmap, &mut resources);
    pixmap.data_as_u8_slice().to_vec()
}
