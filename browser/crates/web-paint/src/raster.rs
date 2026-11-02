use std::sync::Arc;
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{self, Affine, Shape};

use vello_cpu::peniko::ImageSampler;
use vello_cpu::{Image, ImageSource, PaintType, PixelMetadata, Pixmap, RenderContext, Resources};
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
    let mut registered: Vec<Option<vello_cpu::ImageId>> = vec![None; list.images.len()];
    let shape = |rect: &crate::Rect| {
        kurbo::Rect::new(
            f64::from(rect.x),
            f64::from(rect.y),
            f64::from(rect.x + rect.width),
            f64::from(rect.y + rect.height),
        )
    };
    for item in &list.items {
        match item {
            Item::PushClip { rect } => {
                context.push_clip_layer(&shape(rect).to_path(0.1));
            }
            Item::PushOpacity { alpha } => context.push_opacity_layer(*alpha),
            Item::Pop => context.pop_layer(),
            Item::Image { rect, clip, image } => {
                if rect.y > visible.1 || rect.y + rect.height < visible.0 {
                    continue;
                }
                let Some(data) = list.images.get(*image as usize) else {
                    continue;
                };
                let id = match registered.get(*image as usize).copied().flatten() {
                    Some(id) => id,
                    None => {
                        let Some(pixmap) = pixmap(data) else {
                            continue;
                        };
                        let id = resources.register_image(Arc::new(pixmap));
                        if let Some(slot) = registered.get_mut(*image as usize) {
                            *slot = Some(id);
                        }
                        id
                    }
                };
                context.set_paint(PaintType::Image(Image {
                    image: ImageSource::opaque_id(id),
                    sampler: ImageSampler::default(),
                }));
                context.set_paint_transform(
                    Affine::translate((f64::from(rect.x), f64::from(rect.y)))
                        * Affine::scale_non_uniform(
                            f64::from(rect.width) / f64::from(data.width.max(1)),
                            f64::from(rect.height) / f64::from(data.height.max(1)),
                        ),
                );
                context.fill_rect(&shape(clip).intersect(shape(rect)));
                context.reset_paint_transform();
            }
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
                baseline,
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
                let glyphs = list.glyphs(glyphs).iter().map(|glyph| {
                    let (id, x, y) = glyph.translated(0.0, *baseline);
                    vello_cpu::Glyph { id, x, y }
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

fn pixmap(image: &web_image::Image) -> Option<Pixmap> {
    let width = u16::try_from(image.width).ok()?;
    let height = u16::try_from(image.height).ok()?;
    let mut rgba = image.rgba.to_vec();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    Some(Pixmap::from_parts(
        rgba,
        width,
        height,
        PixelMetadata::default(),
    ))
}
