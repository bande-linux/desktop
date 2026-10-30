use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::{Logical, Size, Transform};
use unbound_graphics::{
    Color, CpuRenderer, Item, Paint, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

const TEXT_SIZE: f32 = 12.0;
const PADDING: (f32, f32) = (10.0, 4.0);
const TEXT: Color = Color::hex(0xffffff);
const BACKGROUND: Color = Color::hex(0x1c1c1e).with_alpha(0xd0);

pub(crate) struct Label {
    pub(crate) text: String,
    pub(crate) scale: f64,
    pub(crate) size: Size<f64, Logical>,
    pub(crate) buffer: MemoryRenderBuffer,
}

#[derive(Default)]
pub(crate) struct Labels {
    text: TextSystem,
    renderer: CpuRenderer,
}

impl Labels {
    pub(crate) fn render(&mut self, text: &str, max_width: f64, scale: f64) -> Option<Label> {
        let style = TextStyle::new(TEXT_SIZE, TEXT).weight(TextStyle::SEMIBOLD);
        let room = (max_width as f32 - 2.0 * PADDING.0).max(TEXT_SIZE);
        let paragraph = self
            .text
            .paragraph(text, style, Some(room), TextAlign::Start);
        let width = (paragraph.width().min(room) + 2.0 * PADDING.0).ceil();
        let height = (paragraph.height().min(TEXT_SIZE * 1.6) + 2.0 * PADDING.1).ceil();
        let scene = Scene {
            width: width as u32,
            height: height as u32,
            background: Color::TRANSPARENT,
            items: vec![
                Item::Fill {
                    shape: Shape::rounded(Rect::new(0.0, 0.0, width, height), height / 2.0),
                    paint: Paint::Solid(BACKGROUND),
                },
                Item::Text {
                    origin: Point::new(PADDING.0, PADDING.1),
                    paragraph,
                },
            ],
        };
        let pixels = (
            unbound_graphics::pixels(scene.width, scale),
            unbound_graphics::pixels(scene.height, scale),
        );
        let mut rgba = vec![0; pixels.0 as usize * pixels.1 as usize * 4];
        if let Err(error) = self.renderer.render_into(&scene, scale, &mut rgba) {
            tracing::warn!("cannot draw a window title: {error}");
            return None;
        }
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        Some(Label {
            text: text.to_owned(),
            scale,
            size: (f64::from(width), f64::from(height)).into(),
            buffer: MemoryRenderBuffer::from_slice(
                &rgba,
                Fourcc::Argb8888,
                (pixels.0 as i32, pixels.1 as i32),
                1,
                Transform::Normal,
                None,
            ),
        })
    }
}
