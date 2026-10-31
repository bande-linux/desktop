use bande_graphics::{
    Color, CpuRenderer, Elevation, Item, Palette, Point, Radius, Rect, Scene, Scheme, Shape,
    TextAlign, TextRole, TextStyle, TextSystem,
};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::{Logical, Size, Transform};

pub(crate) const TITLE: LabelStyle = LabelStyle {
    role: TextRole::Callout,
    padding: (8.0, 4.0),
    look: Look::Tip,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Look {
    Tip,
    Card,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LabelStyle {
    pub(crate) role: TextRole,
    pub(crate) padding: (f32, f32),
    pub(crate) look: Look,
}

const CARD_MARGIN: f32 = 24.0;

pub(crate) struct Label {
    pub(crate) text: String,
    pub(crate) scale: f64,
    pub(crate) scheme: Scheme,
    pub(crate) size: Size<f64, Logical>,
    pub(crate) buffer: MemoryRenderBuffer,
}

#[derive(Default)]
pub(crate) struct Labels {
    text: TextSystem,
    renderer: CpuRenderer,
}

impl Labels {
    pub(crate) fn render(
        &mut self,
        text: &str,
        LabelStyle {
            role,
            padding,
            look,
        }: LabelStyle,
        max_width: f64,
        scheme: Scheme,
        scale: f64,
    ) -> Option<Label> {
        let palette = Palette::for_scheme(scheme);
        let (ink, fill, radius, margin) = match look {
            Look::Tip => (palette.chassis, palette.ink, Radius::S, 0.0),
            Look::Card => (palette.ink, palette.overlay, Radius::XL, CARD_MARGIN),
        };
        let style = match look {
            Look::Tip => TextStyle::role(role, ink).weight(TextStyle::MEDIUM),
            Look::Card => TextStyle::role(role, ink),
        };
        let spec = role.spec();
        let room = (max_width as f32 - 2.0 * (padding.0 + margin)).max(spec.size);
        let paragraph = self
            .text
            .paragraph(text, style, Some(room), TextAlign::Start);
        let width = (paragraph.width().min(room) + 2.0 * padding.0).ceil();
        let height = (paragraph.height().min(spec.line_height) + 2.0 * padding.1).ceil();
        let shape = Shape::rounded(Rect::new(margin, margin, width, height), radius);
        let mut items = Vec::new();
        if look == Look::Card {
            items.extend(Item::shadows(shape, Elevation::Overlay, scheme, true));
        }
        items.push(Item::Fill {
            shape,
            paint: fill.into(),
        });
        if look == Look::Card && scheme == Scheme::Dark {
            items.push(Item::Stroke {
                shape: Shape::rounded(shape.rect.inset(0.5), radius - 0.5),
                width: 1.0,
                paint: palette.hairline.into(),
            });
        }
        items.push(Item::Text {
            origin: Point::new(margin + padding.0, margin + padding.1),
            paragraph,
        });
        let (outer_width, outer_height) = (width + 2.0 * margin, height + 2.0 * margin);
        let scene = Scene {
            width: outer_width as u32,
            height: outer_height as u32,
            background: Color::TRANSPARENT,
            items,
        };
        let buffer = self.paint(&scene, scale)?;
        Some(Label {
            text: text.to_owned(),
            scale,
            scheme,
            size: (f64::from(outer_width), f64::from(outer_height)).into(),
            buffer,
        })
    }

    pub(crate) fn text(&mut self) -> &mut TextSystem {
        &mut self.text
    }

    pub(crate) fn rasterise(&mut self, scene: &Scene, scale: f64) -> Option<(Vec<u8>, (i32, i32))> {
        let pixels = (
            bande_graphics::pixels(scene.width, scale),
            bande_graphics::pixels(scene.height, scale),
        );
        let mut rgba = vec![0; pixels.0 as usize * pixels.1 as usize * 4];
        if let Err(error) = self.renderer.render_into(scene, scale, &mut rgba) {
            tracing::warn!("cannot draw a compositor overlay: {error}");
            return None;
        }
        Some((rgba, (pixels.0 as i32, pixels.1 as i32)))
    }

    pub(crate) fn paint(&mut self, scene: &Scene, scale: f64) -> Option<MemoryRenderBuffer> {
        let (rgba, size) = self.rasterise(scene, scale)?;
        Some(MemoryRenderBuffer::from_slice(
            &rgba,
            Fourcc::Abgr8888,
            size,
            1,
            Transform::Normal,
            None,
        ))
    }
}
