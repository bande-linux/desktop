#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use unbound_graphics::{
    Color, CpuRenderer, GpuRenderer, Image, Item, Paint, Palette, Point, Rect, Scene, Scheme,
    Shape, Spacing, Stop, TextAlign, TextStyle, TextSystem, damage,
};

use support::{Reference, assert_all_match, compare};

fn shapes(palette: &Palette) -> Scene {
    Scene {
        width: 240,
        height: 160,
        background: palette.sheet,
        items: vec![
            Item::Fill {
                shape: Shape::rect(Rect::new(8.0, 8.0, 64.0, 40.0)),
                paint: palette.accent.into(),
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(80.25, 8.5, 63.5, 39.75)),
                paint: palette.ink.into(),
            },
            Item::Fill {
                shape: Shape::rounded(Rect::new(152.0, 8.0, 80.0, 40.0), 10.0),
                paint: palette.overlay.into(),
            },
            Item::Stroke {
                shape: Shape::rounded(Rect::new(152.5, 8.5, 79.0, 39.0), 9.5),
                width: 1.0,
                paint: palette.hairline.into(),
            },
            Item::Fill {
                shape: Shape::rounded(Rect::new(8.0, 64.0, 224.0, 32.0), 16.0),
                paint: Paint::LinearGradient {
                    start: Point::new(8.0, 0.0),
                    end: Point::new(232.0, 0.0),
                    stops: vec![
                        Stop {
                            offset: 0.0,
                            color: palette.accent,
                        },
                        Stop {
                            offset: 1.0,
                            color: palette.overlay,
                        },
                    ],
                },
            },
            Item::Fill {
                shape: Shape::rounded(Rect::new(20.3, 108.7, 30.0, 30.0), 15.0),
                paint: palette.ink2.into(),
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(64.0, 112.0, 160.0, 1.0)),
                paint: palette.hairline.into(),
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(64.0, 120.0, 160.0, 24.0)),
                paint: palette.accent.with_alpha(128).into(),
            },
        ],
    }
}

fn shadow(palette: &Palette) -> Scene {
    let card = Shape::rounded(Rect::new(40.0, 32.0, 160.0, 96.0), 12.0);
    Scene {
        width: 240,
        height: 180,
        background: palette.sheet,
        items: vec![
            Item::Shadow {
                shape: card,
                offset: Point::new(0.0, 8.0),
                blur_radius: 24.0,
                color: Color::hex(0x000000).with_alpha(64),
            },
            Item::Fill {
                shape: card,
                paint: palette.overlay.into(),
            },
        ],
    }
}

fn settings_card(palette: &Palette, text: &mut TextSystem) -> Scene {
    let inset = Spacing::L as f32;
    let content = inset + Spacing::L as f32;
    let card = Shape::rounded(
        Rect::new(inset, inset, 320.0 - 2.0 * inset, 200.0 - 2.0 * inset),
        10.0,
    );
    let title = text.paragraph(
        "Unbound Settings",
        TextStyle::new(20.0, palette.ink).weight(TextStyle::SEMIBOLD),
        None,
        TextAlign::Start,
    );
    let body = text.paragraph(
        "Displays, Sound & Network: AVA Wo",
        TextStyle::new(13.0, palette.ink2),
        Some(320.0 - 2.0 * content),
        TextAlign::Start,
    );
    let button = text.paragraph(
        "Apply",
        TextStyle::new(13.0, Color::hex(0xFFFFFF)),
        Some(96.0),
        TextAlign::Center,
    );
    Scene {
        width: 320,
        height: 200,
        background: palette.sheet,
        items: vec![
            Item::Shadow {
                shape: card,
                offset: Point::new(0.0, 2.0),
                blur_radius: 8.0,
                color: Color::hex(0x000000).with_alpha(40),
            },
            Item::Fill {
                shape: card,
                paint: palette.overlay.into(),
            },
            Item::Text {
                origin: Point::new(content, 30.0),
                paragraph: title,
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(inset, 68.0, 320.0 - 2.0 * inset, 1.0)),
                paint: palette.hairline.into(),
            },
            Item::Text {
                origin: Point::new(content + 0.4, 84.0),
                paragraph: body,
            },
            Item::Fill {
                shape: Shape::rounded(Rect::new(content, 128.0, 96.0, 32.0), 6.0),
                paint: palette.accent.into(),
            },
            Item::Text {
                origin: Point::new(content, 136.0),
                paragraph: button,
            },
        ],
    }
}

fn icon(palette: &Palette) -> Scene {
    let (width, height) = (48u32, 32u32);
    let rgba = (0..height)
        .flat_map(|y| {
            (0..width).flat_map(move |x| {
                let alpha = if (x / 8 + y / 8) % 2 == 0 { 255 } else { 96 };
                [(x * 5) as u8, (y * 7) as u8, 200, alpha]
            })
        })
        .collect();
    Scene {
        width: 96,
        height: 64,
        background: palette.sheet,
        items: vec![Item::Image {
            rect: Rect::new(20.0, 12.0, width as f32, height as f32),
            image: Image::from_rgba(width, height, rgba).expect("icon pixels"),
        }],
    }
}

fn scenes(text: &mut TextSystem) -> Vec<(String, Scene)> {
    let mut scenes = Vec::new();
    for (name, scheme) in [("light", Scheme::Light), ("dark", Scheme::Dark)] {
        let palette = Palette::for_scheme(scheme);
        scenes.push((format!("shapes-{name}"), shapes(&palette)));
        scenes.push((format!("shadow-{name}"), shadow(&palette)));
        scenes.push((format!("icon-{name}"), icon(&palette)));
        scenes.push((
            format!("settings-card-{name}"),
            settings_card(&palette, text),
        ));
    }
    scenes
}

#[test]
fn cpu_frames_match_references_computed_from_the_scene() {
    let mut text = TextSystem::new();
    let mut renderer = CpuRenderer::new();
    let results: Vec<_> = scenes(&mut text)
        .into_iter()
        .map(|(name, scene)| {
            let name = format!("cpu-{name}");
            let snapshot = renderer.render(&scene).expect("CPU frame");
            let comparison = compare(&name, &snapshot, &Reference::draw(&scene));
            (name, comparison)
        })
        .collect();
    assert_all_match(&results);
}

#[test]
fn gpu_frames_match_the_same_references() {
    let mut text = TextSystem::new();
    let mut renderer = GpuRenderer::new().expect("Vulkan device");
    let info = renderer.adapter_info();
    println!(
        "adapter: {} ({:?}, {})",
        info.name, info.device_type, info.driver
    );
    let results: Vec<_> = scenes(&mut text)
        .into_iter()
        .map(|(name, scene)| {
            let name = format!("gpu-{name}");
            let snapshot = renderer.render(&scene).expect("GPU frame");
            let comparison = compare(&name, &snapshot, &Reference::draw(&scene));
            (name, comparison)
        })
        .collect();
    assert_all_match(&results);
}

fn solid_image_scene(rgba: [u8; 4]) -> Scene {
    let image = Image::from_rgba(32, 32, rgba.repeat(32 * 32)).expect("image");
    Scene {
        width: 64,
        height: 64,
        background: Color::hex(0x000000),
        items: vec![Item::Image {
            rect: Rect::new(16.0, 16.0, 32.0, 32.0),
            image,
        }],
    }
}

#[test]
fn an_image_replacing_a_dropped_one_is_drawn_whole_on_the_gpu() {
    let mut renderer = GpuRenderer::new().expect("Vulkan device");
    renderer
        .render(&solid_image_scene([255, 0, 0, 255]))
        .expect("first frame");
    let snapshot = renderer
        .render(&solid_image_scene([0, 0, 255, 255]))
        .expect("second frame");
    let centre = ((32 * snapshot.width + 32) * 4) as usize;
    assert_eq!(&snapshot.rgba[centre..centre + 4], [0, 0, 255, 255]);
}

fn highlighted(scene: &Scene) -> Scene {
    let mut highlighted = scene.clone();
    highlighted.items.insert(
        1,
        Item::Fill {
            shape: Shape::rounded(Rect::new(20.5, 10.25, 40.0, 24.0), 8.0),
            paint: Color::hex(0x3478F6).with_alpha(100).into(),
        },
    );
    highlighted
}

#[test]
fn repainting_only_the_damaged_regions_gives_the_full_frame() {
    let mut text = TextSystem::new();
    let mut renderer = CpuRenderer::new();
    for (name, before) in scenes(&mut text) {
        let after = highlighted(&before);
        let regions = damage(&before, &after, 1.0).expect("a partial change");
        assert!(!regions.is_empty(), "{name}: the highlight is damaged");
        let mut patched = renderer.render(&before).expect("first frame").rgba;
        for region in regions {
            let row_bytes = region.width as usize * 4;
            let mut pixels = vec![0; row_bytes * region.height as usize];
            renderer
                .render_region(&after, 1.0, region, &mut pixels)
                .expect("region");
            for (row, source) in pixels.chunks_exact(row_bytes).enumerate() {
                let start =
                    ((region.y as usize + row) * after.width as usize + region.x as usize) * 4;
                patched[start..start + row_bytes].copy_from_slice(source);
            }
        }
        let whole = renderer.render(&after).expect("full frame").rgba;
        let delta = patched
            .iter()
            .zip(&whole)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        assert!(
            delta <= 1,
            "{name}: partial and full frames differ by {delta}"
        );
    }
}
