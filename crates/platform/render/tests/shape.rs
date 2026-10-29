#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::{Bind, Color32F, Frame, ImportMem, Offscreen, Renderer};
use smithay::utils::{Physical, Rectangle, Size, Transform};
use unbound_render::{Blur, RoundedRect, Shadow, VulkanFrame, VulkanRenderer};

const SIZE: (i32, i32) = (80, 80);
const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];

fn render(paint: impl FnOnce(&mut VulkanRenderer) -> Vec<Step>) -> Vec<[u8; 4]> {
    let mut renderer = VulkanRenderer::new().expect("Vulkan renderer");
    let steps = paint(&mut renderer);
    let mut target = renderer
        .create_buffer(Fourcc::Abgr8888, Size::from(SIZE))
        .expect("target");
    {
        let mut framebuffer = renderer.bind(&mut target).expect("bind");
        let mut frame = renderer
            .render(&mut framebuffer, Size::from(SIZE), Transform::Normal)
            .expect("frame");
        for step in steps {
            step(&mut frame);
        }
        frame
            .finish()
            .expect("finished frame")
            .wait()
            .expect("signalled");
    }
    let rgba = renderer.read_target(&target).expect("read back");
    rgba.as_chunks::<4>().0.to_vec()
}

type Step = Box<dyn FnOnce(&mut VulkanFrame<'_, '_>)>;

fn at(pixels: &[[u8; 4]], x: i32, y: i32) -> [u8; 4] {
    pixels[(y * SIZE.0 + x) as usize]
}

fn whole() -> Rectangle<i32, Physical> {
    Rectangle::from_size(Size::from(SIZE))
}

fn clear(color: [f32; 4]) -> Step {
    Box::new(move |frame| {
        frame
            .clear(
                Color32F::new(color[0], color[1], color[2], color[3]),
                &[whole()],
            )
            .expect("clear");
    })
}

fn square(x: f32, y: f32, side: f32, radius: f32) -> RoundedRect {
    RoundedRect {
        x,
        y,
        width: side,
        height: side,
        radius,
    }
}

fn texture_step(
    renderer: &mut VulkanRenderer,
    bgra: [u8; 4],
    texels: (i32, i32),
    destination: Rectangle<i32, Physical>,
    clip: Option<RoundedRect>,
    pattern: Option<Vec<u8>>,
) -> Step {
    let bytes = pattern.unwrap_or_else(|| bgra.repeat((texels.0 * texels.1) as usize));
    let texture = renderer
        .import_memory(&bytes, Fourcc::Argb8888, Size::from(texels), false)
        .expect("texture");
    Box::new(move |frame| {
        frame.clip_to(clip);
        frame
            .render_texture_from_to(
                &texture,
                Rectangle::from_size(Size::from(texels)).to_f64(),
                destination,
                &[Rectangle::from_size(destination.size)],
                &[],
                Transform::Normal,
                1.0,
            )
            .expect("draw texture");
        frame.clip_to(None);
    })
}

#[test]
fn a_clipped_window_shows_what_is_behind_its_rounded_corners() {
    let pixels = render(|renderer| {
        let window = Rectangle::new((10, 10).into(), (40, 40).into());
        vec![
            clear([0.0, 0.0, 1.0, 1.0]),
            texture_step(
                renderer,
                [0, 0, 255, 255],
                (40, 40),
                window,
                Some(square(10.0, 10.0, 40.0, 8.0)),
                None,
            ),
        ]
    });

    assert_eq!(at(&pixels, 10, 10), BLUE);
    assert_eq!(at(&pixels, 49, 49), BLUE);
    assert_eq!(at(&pixels, 30, 10), RED);
    assert_eq!(at(&pixels, 10, 30), RED);
    assert_eq!(at(&pixels, 49, 30), RED);
    assert_eq!(at(&pixels, 30, 30), RED);
    let edge = at(&pixels, 12, 12);
    assert!(
        edge[0] > 0 && edge[0] < 255 && edge[2] > 0 && edge[2] < 255,
        "{edge:?}"
    );
}

#[test]
fn an_unclipped_texture_is_copied_exactly() {
    let pixels = render(|renderer| {
        vec![
            clear([0.0, 0.0, 1.0, 1.0]),
            texture_step(
                renderer,
                [0, 0, 255, 255],
                (40, 40),
                Rectangle::new((10, 10).into(), (40, 40).into()),
                None,
                None,
            ),
        ]
    });

    assert_eq!(at(&pixels, 10, 10), RED);
    assert_eq!(at(&pixels, 49, 49), RED);
    assert_eq!(at(&pixels, 9, 10), BLUE);
}

#[test]
fn a_scaled_texture_is_filtered_rather_than_repeated() {
    let pixels = render(|renderer| {
        let black_then_white = [[0, 0, 0, 255], [255, 255, 255, 255]].concat();
        vec![texture_step(
            renderer,
            [0; 4],
            (2, 1),
            Rectangle::new((0, 0).into(), (40, 10).into()),
            None,
            Some(black_then_white),
        )]
    });

    let row: Vec<u8> = (0..40).map(|x| at(&pixels, x, 5)[0]).collect();
    assert_eq!(row[0], 0);
    assert_eq!(row[39], 255);
    assert!(row.windows(2).all(|pair| pair[0] <= pair[1]), "{row:?}");
    assert!(
        row.iter()
            .filter(|value| **value != 0 && **value != 255)
            .count()
            > 10
    );
}

fn shadow_under(window: RoundedRect) -> Shadow {
    Shadow {
        caster: RoundedRect {
            y: window.y + 6.0,
            ..window
        },
        window,
        blur: 5.0,
        color: [0.0, 0.0, 0.0, 0.5],
    }
}

fn shadow_step(shadow: Shadow) -> Step {
    Box::new(move |frame| {
        let bounds = shadow.bounds();
        frame
            .draw_shadow(
                &shadow,
                &[Rectangle::from_size((bounds.width, bounds.height).into())],
            )
            .expect("draw shadow");
    })
}

#[test]
fn a_shadow_falls_off_below_the_window_and_leaves_it_uncovered() {
    let window = square(20.0, 20.0, 30.0, 6.0);
    let pixels = render(|_| vec![clear([1.0; 4]), shadow_step(shadow_under(window))]);

    assert_eq!(at(&pixels, 35, 35), WHITE);
    assert_eq!(at(&pixels, 27, 20), WHITE);
    assert_eq!(at(&pixels, 35, 79), WHITE);
    assert_eq!(at(&pixels, 2, 35), WHITE);
    let below: Vec<u8> = (50..72).map(|y| at(&pixels, 35, y)[0]).collect();
    assert!(below[0] < 200, "{below:?}");
    assert!(below.windows(2).all(|pair| pair[0] <= pair[1]), "{below:?}");
    assert!(at(&pixels, 35, 52)[0] < at(&pixels, 35, 17)[0]);
    assert!(at(&pixels, 35, 17)[0] < 255);
}

#[test]
fn a_shadow_reaches_only_its_bounds() {
    let shadow = shadow_under(square(20.0, 20.0, 30.0, 6.0));
    let bounds = shadow.bounds();

    assert_eq!(
        (bounds.x, bounds.y, bounds.width, bounds.height),
        (5, 11, 60, 60)
    );
}

#[test]
fn a_clipped_solid_fill_is_rounded_even_when_opaque() {
    let pixels = render(|_| {
        vec![
            clear([0.0, 0.0, 1.0, 1.0]),
            Box::new(|frame: &mut VulkanFrame<'_, '_>| {
                frame.clip_to(Some(square(10.0, 10.0, 40.0, 8.0)));
                frame
                    .draw_solid(
                        Rectangle::new((10, 10).into(), (40, 40).into()),
                        &[Rectangle::from_size((40, 40).into())],
                        Color32F::new(1.0, 0.0, 0.0, 1.0),
                    )
                    .expect("fill");
                frame.clip_to(None);
            }),
        ]
    });

    assert_eq!(at(&pixels, 10, 10), BLUE);
    assert_eq!(at(&pixels, 30, 10), RED);
    assert_eq!(at(&pixels, 30, 30), RED);
}

fn halves_then_blur(blur: Blur) -> Vec<[u8; 4]> {
    render(move |_| {
        vec![
            clear([1.0; 4]),
            Box::new(|frame: &mut VulkanFrame<'_, '_>| {
                frame
                    .clear(
                        Color32F::new(0.0, 0.0, 0.0, 1.0),
                        &[Rectangle::from_size((40, 80).into())],
                    )
                    .expect("left half");
            }),
            Box::new(move |frame: &mut VulkanFrame<'_, '_>| {
                let bounds = blur.bounds();
                frame
                    .draw_blur(
                        &blur,
                        &[Rectangle::from_size((bounds.width, bounds.height).into())],
                    )
                    .expect("blur");
            }),
        ]
    })
}

#[test]
fn a_blurred_region_softens_an_edge_behind_it_and_leaves_the_rest_alone() {
    let pixels = halves_then_blur(Blur {
        region: square(20.0, 20.0, 40.0, 0.0),
        passes: 3,
        offset: 1.5,
        alpha: 1.0,
    });

    let row: Vec<u8> = (20..60).map(|x| at(&pixels, x, 40)[0]).collect();
    assert!(row.windows(2).all(|pair| pair[0] <= pair[1]), "{row:?}");
    assert!(
        row.iter()
            .filter(|value| **value > 10 && **value < 245)
            .count()
            >= 8,
        "{row:?}"
    );
    assert_eq!(at(&pixels, 10, 40), [0, 0, 0, 255]);
    assert_eq!(at(&pixels, 70, 40), WHITE);
    assert_eq!(at(&pixels, 39, 10), [0, 0, 0, 255]);
    assert_eq!(at(&pixels, 40, 70), WHITE);
}

#[test]
fn a_blur_keeps_a_uniform_background_uniform() {
    let pixels = halves_then_blur(Blur {
        region: square(60.0, 20.0, 15.0, 4.0),
        passes: 2,
        offset: 1.0,
        alpha: 1.0,
    });

    assert!((60..75).all(|x| at(&pixels, x, 28)[0] >= 254));
}

#[test]
fn a_blurred_region_is_clipped_to_its_rounded_shape() {
    let pixels = halves_then_blur(Blur {
        region: square(30.0, 20.0, 20.0, 8.0),
        passes: 3,
        offset: 1.5,
        alpha: 1.0,
    });

    assert_eq!(at(&pixels, 30, 20), [0, 0, 0, 255]);
    assert!(at(&pixels, 30, 30)[0] > 0);
}
