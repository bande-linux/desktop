use std::path::Path;

use ab_glyph::{Font as _, FontVec, PxScale, ScaleFont as _, point};
use image::{RgbImage, buffer::ConvertBuffer};
use image_compare::Algorithm;
use unbound_toolkit::{
    Color, Font, Gpu, Item, Palette, Rect, Renderer, Scene, Scheme, Snapshot, Spacing,
};

const FONT_PATH: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";
const MIN_SSIM: f64 = 0.995;
const MAX_CHANNEL_DELTA: u8 = 2;

fn settings_card(palette: &Palette) -> Scene {
    let inset = Spacing::L as i32;
    let content = inset + Spacing::L as i32;
    Scene {
        width: 320,
        height: 200,
        background: palette.surface_background,
        items: vec![
            Item::Fill {
                rect: Rect {
                    x: inset,
                    y: inset,
                    width: 320 - 2 * Spacing::L,
                    height: 200 - 2 * Spacing::L,
                },
                color: palette.surface_elevated,
            },
            Item::Text {
                x: content,
                baseline: 52,
                size: 20.0,
                color: palette.label_primary,
                text: "Unbound Settings".to_owned(),
            },
            Item::Fill {
                rect: Rect {
                    x: inset,
                    y: 68,
                    width: 320 - 2 * Spacing::L,
                    height: 1,
                },
                color: palette.separator_color,
            },
            Item::Text {
                x: content,
                baseline: 100,
                size: 14.0,
                color: palette.label_muted,
                text: "Displays, Sound & Network: AVA Wo".to_owned(),
            },
            Item::Fill {
                rect: Rect {
                    x: content,
                    y: 128,
                    width: 96,
                    height: 32,
                },
                color: palette.accent_color,
            },
        ],
    }
}

fn reference(scene: &Scene, font: &FontVec) -> RgbImage {
    let rgb = |color: Color| image::Rgb([color.r, color.g, color.b]);
    let mut image = RgbImage::from_pixel(scene.width, scene.height, rgb(scene.background));
    for item in &scene.items {
        match item {
            Item::Fill { rect, color } => {
                for y in rect.y..rect.y + rect.height as i32 {
                    for x in rect.x..rect.x + rect.width as i32 {
                        image.put_pixel(x as u32, y as u32, rgb(*color));
                    }
                }
            }
            Item::Text {
                x,
                baseline,
                size,
                color,
                text,
            } => {
                let scale = PxScale::from(*size);
                let scaled = font.as_scaled(scale);
                let mut pen = *x as f32;
                let mut previous = None;
                for character in text.chars() {
                    let id = scaled.glyph_id(character);
                    if let Some(previous) = previous {
                        pen += scaled.kern(previous, id);
                    }
                    previous = Some(id);
                    let origin = point(pen.round(), *baseline as f32);
                    pen += scaled.h_advance(id);
                    let Some(outlined) =
                        font.outline_glyph(id.with_scale_and_position(scale, origin))
                    else {
                        continue;
                    };
                    let bounds = outlined.px_bounds();
                    outlined.draw(|gx, gy, coverage| {
                        let px = bounds.min.x as i32 + gx as i32;
                        let py = bounds.min.y as i32 + gy as i32;
                        if px < 0 || py < 0 || px >= scene.width as i32 || py >= scene.height as i32
                        {
                            return;
                        }
                        let alpha =
                            f64::from((coverage.clamp(0.0, 1.0) * 255.0).round() as u8) / 255.0;
                        let pixel = image.get_pixel_mut(px as u32, py as u32);
                        for (channel, ink) in pixel.0.iter_mut().zip([color.r, color.g, color.b]) {
                            let blended =
                                f64::from(ink) * alpha + f64::from(*channel) * (1.0 - alpha);
                            *channel = blended.round() as u8;
                        }
                    });
                }
            }
        }
    }
    image
}

fn compare(name: &str, snapshot: Snapshot, expected: &RgbImage) {
    let actual: RgbImage =
        image::RgbaImage::from_raw(snapshot.width, snapshot.height, snapshot.rgba)
            .expect("snapshot dimensions")
            .convert();
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let actual_path = directory.join(format!("{name}-actual.png"));
    actual.save(&actual_path).expect("save rendered frame");
    expected
        .save(directory.join(format!("{name}-expected.png")))
        .expect("save reference");

    let ssim = image_compare::rgb_similarity_structure(&Algorithm::MSSIMSimple, &actual, expected)
        .expect("SSIM")
        .score;
    let max_delta = actual
        .as_raw()
        .iter()
        .zip(expected.as_raw())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(
        ssim >= MIN_SSIM && max_delta <= MAX_CHANNEL_DELTA,
        "{name}: SSIM {ssim:.5} (min {MIN_SSIM}), max channel delta {max_delta} (max {MAX_CHANNEL_DELTA}); see {}",
        actual_path.display()
    );
}

#[test]
fn settings_card_matches_a_cpu_reference_in_both_schemes() {
    let bytes = std::fs::read(FONT_PATH).expect("DejaVu Sans (fonts-dejavu-core)");
    let font = Font::from_bytes(bytes.clone()).expect("font");
    let reference_font = FontVec::try_from_vec(bytes).expect("font");
    let renderer = Renderer::new(Gpu::new().expect("Vulkan device"));
    let info = renderer.adapter_info();
    println!(
        "adapter: {} ({:?}, {})",
        info.name, info.device_type, info.driver
    );

    for (name, scheme) in [("light", Scheme::Light), ("dark", Scheme::Dark)] {
        let scene = settings_card(&Palette::for_scheme(scheme));
        let snapshot = renderer.render(&scene, &font).expect("rendered frame");
        compare(
            &format!("settings-card-{name}"),
            snapshot,
            &reference(&scene, &reference_font),
        );
    }
}
