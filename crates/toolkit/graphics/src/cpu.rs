use vello_cpu::kurbo::{self, Affine, BezPath};
use vello_cpu::{
    Glyph, PaintType, PixmapMut, RasterizerSettings, RenderContext, Resources, TargetInit,
};

use crate::damage::Region;
use crate::draw::{self, Canvas, ImageIds};
use crate::outline::GlyphOutlines;
use crate::render::{DrawError, Snapshot};
use crate::scene::Scene;
use crate::text::GlyphRun;

pub struct CpuRenderer {
    context: RenderContext,
    resources: Resources,
    outlines: GlyphOutlines,
    image_ids: ImageIds,
}

impl Default for CpuRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuRenderer {
    pub fn new() -> Self {
        Self {
            context: RenderContext::new(1, 1),
            resources: Resources::new(),
            outlines: GlyphOutlines::default(),
            image_ids: ImageIds::default(),
        }
    }

    pub fn render(&mut self, scene: &Scene) -> Result<Snapshot, DrawError> {
        let mut rgba = vec![0; scene.width as usize * scene.height as usize * 4];
        self.render_into(scene, 1.0, &mut rgba)?;
        Ok(Snapshot {
            width: scene.width,
            height: scene.height,
            rgba,
        })
    }

    pub fn render_into(
        &mut self,
        scene: &Scene,
        scale: f64,
        rgba: &mut [u8],
    ) -> Result<(), DrawError> {
        let (width, height) = crate::render::extent(scene, scale)?;
        let whole = Region {
            x: 0,
            y: 0,
            width: u32::from(width),
            height: u32::from(height),
        };
        self.render_region(scene, scale, whole, rgba)
    }

    pub fn render_region(
        &mut self,
        scene: &Scene,
        scale: f64,
        region: Region,
        rgba: &mut [u8],
    ) -> Result<(), DrawError> {
        crate::render::extent(scene, scale)?;
        let too_large = || DrawError::TooLarge {
            width: region.width,
            height: region.height,
        };
        let width = u16::try_from(region.width).map_err(|_| too_large())?;
        let height = u16::try_from(region.height).map_err(|_| too_large())?;
        self.context.reset_and_resize(width, height);
        let mut images = Vec::new();
        draw::images(&scene.items, &mut images);
        for id in self.image_ids.take_dropped() {
            self.resources.destroy_image(id);
        }
        for image in images {
            if self.image_ids.get(&image).is_none()
                && let Some(pixmap) = image.pixmap()
            {
                let id = self.resources.register_image(pixmap);
                self.image_ids.insert(&image, id);
            }
        }
        draw::encode(
            &mut CpuCanvas {
                context: &mut self.context,
                resources: &mut self.resources,
            },
            &mut self.outlines,
            &self.image_ids,
            &scene.items,
            scale,
            (region.x, region.y),
        );
        self.outlines.end_frame();
        self.context.flush();
        let target = PixmapMut::new(width, height, rgba).ok_or(DrawError::BufferSize)?;
        self.context.render_with(
            target,
            &mut self.resources,
            RasterizerSettings {
                target_init: TargetInit::Clear(draw::color(scene.background)),
                ..RasterizerSettings::default()
            },
        );
        Ok(())
    }
}

struct CpuCanvas<'a> {
    context: &'a mut RenderContext,
    resources: &'a mut Resources,
}

impl Canvas for CpuCanvas<'_> {
    fn set_paint(&mut self, paint: PaintType) {
        self.context.set_paint(paint);
    }

    fn set_transform(&mut self, transform: Affine) {
        self.context.set_transform(transform);
    }

    fn set_paint_transform(&mut self, transform: Affine) {
        self.context.set_paint_transform(transform);
    }

    fn reset_paint_transform(&mut self) {
        self.context.reset_paint_transform();
    }

    fn fill_path(&mut self, path: &BezPath) {
        self.context.fill_path(path);
    }

    fn fill_blurred_rounded_rect(&mut self, rect: &kurbo::Rect, radius: f32, std_dev: f32) {
        self.context
            .fill_blurred_rounded_rect(rect, radius, std_dev, false);
    }

    fn push_clip_layer(&mut self, path: &BezPath) {
        self.context.push_clip_layer(path);
    }

    fn push_opacity_layer(&mut self, alpha: f32) {
        self.context.push_opacity_layer(alpha);
    }

    fn pop_layer(&mut self) {
        self.context.pop_layer();
    }

    fn fill_glyphs(&mut self, run: &GlyphRun, glyphs: &[Glyph]) {
        let _ = self
            .context
            .glyph_run(self.resources, &run.font)
            .font_size(run.size)
            .hint(false)
            .normalized_coords(&run.coords)
            .fill_glyphs(glyphs.iter().copied());
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::CpuRenderer;
    use crate::scene::{Image, Item, Rect, Scene};
    use bande_design::Color;

    fn scene(items: Vec<Item>) -> Scene {
        Scene {
            width: 8,
            height: 8,
            background: Color::hex(0xFFFFFF),
            items,
        }
    }

    #[test]
    fn an_image_nobody_holds_any_more_leaves_the_renderer() {
        let mut renderer = CpuRenderer::new();
        let image = Image::from_rgba(4, 4, vec![255; 64]).expect("image");
        let shown = Item::Image {
            rect: Rect::new(0.0, 0.0, 4.0, 4.0),
            image: image.clone(),
        };
        renderer.render(&scene(vec![shown])).expect("frame");
        let id = renderer.image_ids.get(&image).expect("uploaded");
        renderer.render(&scene(Vec::new())).expect("frame");
        assert!(renderer.resources.resolve_image(id).is_some(), "still held");
        drop(image);
        renderer.render(&scene(Vec::new())).expect("frame");
        assert!(renderer.resources.resolve_image(id).is_none());
    }
}
