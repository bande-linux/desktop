use std::error::Error;

use bande_render::{Blur, OffscreenTarget, RoundedRect, Shadow, VulkanRenderer, vulkan_errors};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::{Bind, Color32F, Frame, Offscreen, Renderer};
use smithay::utils::{Physical, Rectangle, Transform};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

const SIZE: (i32, i32) = (320, 240);
const GROUND: f64 = 200.0;
const OPACITY: f64 = 0.6;
const FALLOFF_SLACK: f64 = 2.0;

fn gaussian(x: f64, sigma: f64) -> f64 {
    (-(x * x) / (2.0 * sigma * sigma)).exp() / (2.5066283 * sigma)
}

fn erf(x: f64) -> f64 {
    let size = x.abs();
    let polynomial =
        1.0 + (0.278393 + (0.230389 + 0.000972 * size + 0.078108 * size * size) * size) * size;
    x.signum() - x.signum() / polynomial.powi(4)
}

fn distance(point: (f64, f64), rect: RoundedRect) -> f64 {
    let half = (f64::from(rect.width) / 2.0, f64::from(rect.height) / 2.0);
    let corner = f64::from(rect.radius).min(half.0.min(half.1));
    let inset = (
        (point.0 - f64::from(rect.x) - half.0).abs() - half.0 + corner,
        (point.1 - f64::from(rect.y) - half.1).abs() - half.1 + corner,
    );
    inset.0.max(0.0).hypot(inset.1.max(0.0)) + inset.0.max(inset.1).min(0.0) - corner
}

fn falloff(point: (f64, f64), rect: RoundedRect, sigma: f64) -> f64 {
    let half = (f64::from(rect.width) / 2.0, f64::from(rect.height) / 2.0);
    let corner = f64::from(rect.radius).min(half.0.min(half.1));
    let local = (
        point.0 - f64::from(rect.x) - half.0,
        point.1 - f64::from(rect.y) - half.1,
    );
    let (low, high) = (local.1 - half.1, local.1 + half.1);
    let start = (-3.0 * sigma).clamp(low, high);
    let end = (3.0 * sigma).clamp(low, high);
    let step = (end - start) / 4.0;
    (0..4)
        .map(|sample| {
            let y = start + step * (f64::from(sample) + 0.5);
            let delta = (half.1 - corner - (local.1 - y).abs()).min(0.0);
            let curved = half.0 - corner + (corner * corner - delta * delta).max(0.0).sqrt();
            let scale = std::f64::consts::FRAC_1_SQRT_2 / sigma;
            let row = 0.5 * (erf((local.0 + curved) * scale) - erf((local.0 - curved) * scale));
            row * gaussian(y, sigma) * step
        })
        .sum()
}

fn shade(point: (f64, f64), shadow: &Shadow) -> f64 {
    let uncovered = 1.0 - (0.5 - distance(point, shadow.window)).clamp(0.0, 1.0);
    GROUND * (1.0 - OPACITY * uncovered * falloff(point, shadow.caster, f64::from(shadow.blur)))
}

fn read(renderer: &mut VulkanRenderer, target: &OffscreenTarget) -> Outcome<Vec<u8>> {
    Ok(renderer.read_target(target)?)
}

#[test]
fn a_shadow_shades_the_ring_around_its_window_and_nothing_under_it() -> Outcome {
    let mut renderer = VulkanRenderer::new(None)?;
    let whole: Rectangle<i32, Physical> = Rectangle::from_size(SIZE.into());
    let ground = GROUND as f32 / 255.0;
    for (radius, turn) in [
        (0.0, Transform::Normal),
        (18.0, Transform::Normal),
        (18.0, Transform::_90),
        (200.0, Transform::Normal),
    ] {
        let mut window = RoundedRect {
            x: 60.5,
            y: 50.0,
            width: 130.0,
            height: 120.25,
            radius,
        };
        let mut caster = RoundedRect {
            y: window.y + 6.0,
            ..window
        };
        let shadow = Shadow {
            caster,
            window,
            blur: 9.0,
            color: [0.0, 0.0, 0.0, OPACITY as f32],
        };
        let mut target: OffscreenTarget = renderer.create_buffer(Fourcc::Argb8888, SIZE.into())?;
        renderer.take_counters();
        let mut framebuffer = renderer.bind(&mut target)?;
        let mut frame = renderer.render(&mut framebuffer, SIZE.into(), turn)?;
        let area = turn.transform_rect_in(whole, &SIZE.into());
        frame.clear(
            Color32F::new(ground, ground, ground, 1.0),
            &[Rectangle::from_size(area.size)],
        )?;
        let bounds = shadow.bounds();
        frame.draw_shadow(
            &shadow,
            &[Rectangle::from_size((bounds.width, bounds.height).into())],
        )?;
        frame.finish()?.wait()?;
        let shaded = renderer.take_counters().shaded_pixels;
        if turn == Transform::Normal && radius < 100.0 {
            let saved = (window.width - 4.0) * (window.height - 2.0 * radius - 4.0);
            let most = whole.size.w as u64 * whole.size.h as u64 + bounds.area();
            assert!(
                shaded + saved as u64 <= most,
                "{shaded} pixels of {most} shaded with radius {radius}"
            );
        }
        let pixels = read(&mut renderer, &target)?;
        if turn != Transform::Normal {
            let turned = |rect: RoundedRect| {
                let placed = turn.transform_rect_in(
                    Rectangle::<f64, Physical>::new(
                        (f64::from(rect.x), f64::from(rect.y)).into(),
                        (f64::from(rect.width), f64::from(rect.height)).into(),
                    ),
                    &(f64::from(SIZE.1), f64::from(SIZE.0)).into(),
                );
                RoundedRect {
                    x: placed.loc.x as f32,
                    y: placed.loc.y as f32,
                    width: placed.size.w as f32,
                    height: placed.size.h as f32,
                    radius: rect.radius,
                }
            };
            window = turned(window);
            caster = turned(caster);
        }
        let expected = Shadow {
            caster,
            window,
            ..shadow
        };
        let mut drift = 0.0;
        for y in 0..SIZE.1 {
            for x in 0..SIZE.0 {
                let wanted = shade((f64::from(x) + 0.5, f64::from(y) + 0.5), &expected);
                let found = f64::from(pixels[((y * SIZE.0 + x) * 4) as usize]);
                assert!(
                    (found - wanted).abs() <= FALLOFF_SLACK,
                    "{found} at {x},{y}, not {wanted:.2}, radius {radius} {turn:?}"
                );
                drift += found - wanted;
            }
        }
        let drift = drift / f64::from(SIZE.0 * SIZE.1);
        assert!(drift.abs() < 0.2, "shadows average {drift} off");
    }
    assert_eq!(vulkan_errors(), 0, "Vulkan reported errors");
    Ok(())
}

#[test]
fn a_toned_blur_of_one_colour_keeps_it_on_average() -> Outcome {
    let mut renderer = VulkanRenderer::new(None)?;
    let whole: Rectangle<i32, Physical> = Rectangle::from_size(SIZE.into());
    let mut target: OffscreenTarget = renderer.create_buffer(Fourcc::Argb8888, SIZE.into())?;
    let level = 100.0 / 255.0;
    let blur = Blur {
        region: RoundedRect {
            x: 80.0,
            y: 60.0,
            width: 160.0,
            height: 120.0,
            radius: 0.0,
        },
        passes: 3,
        offset: 1.5,
        alpha: 1.0,
        tone: Some([1.0, 0.0, 1.0, 0.0]),
    };
    let backdrop = renderer.backdrop((1, &blur), 0, (Vec::new(), &[]), |_, _| Vec::new());
    let mut framebuffer = renderer.bind(&mut target)?;
    let mut frame = renderer.render(&mut framebuffer, SIZE.into(), Transform::Normal)?;
    frame.clear(Color32F::new(level, level, level, 1.0), &[whole])?;
    let bounds = blur.bounds();
    frame.draw_blur(
        &blur,
        &backdrop,
        &[Rectangle::from_size((bounds.width, bounds.height).into())],
    )?;
    frame.finish()?.wait()?;
    let pixels = read(&mut renderer, &target)?;
    let mut total = 0.0;
    for y in 60..180 {
        for x in 80..240 {
            let found = f64::from(pixels[(y * SIZE.0 as usize + x) * 4]);
            assert!((found - 100.0).abs() <= 1.0, "{found} at {x},{y}");
            total += found;
        }
    }
    let mean = total / (120.0 * 160.0);
    assert!((mean - 100.0).abs() < 0.3, "{mean} on average");
    assert_eq!(vulkan_errors(), 0, "Vulkan reported errors");
    Ok(())
}
