use vello_cpu::kurbo::{self, Affine, BezPath};
use vello_cpu::{
    Glyph, PaintType, PixmapMut, RasterizerSettings, RenderContext, Resources, TargetInit,
};

use crate::draw::{self, Canvas, ImageIds};
use crate::outline::GlyphOutlines;
use crate::render::{RenderError, Snapshot};
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
            image_ids: ImageIds::new(),
        }
    }

    pub fn render(&mut self, scene: &Scene) -> Result<Snapshot, RenderError> {
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
    ) -> Result<(), RenderError> {
        let (width, height) = crate::render::extent(scene, scale)?;
        self.context.reset_and_resize(width, height);
        let mut images = Vec::new();
        draw::images(&scene.items, &mut images);
        for image in images {
            if !self.image_ids.contains_key(&image.id()) {
                let id = self.resources.register_image(image.pixmap.clone());
                self.image_ids.insert(image.id(), id);
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
        );
        self.outlines.end_frame();
        self.context.flush();
        let target = PixmapMut::new(width, height, rgba).ok_or(RenderError::BufferSize)?;
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
