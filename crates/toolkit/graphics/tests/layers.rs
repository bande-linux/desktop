#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_graphics::{Color, CpuRenderer, Item, Path, Point, Rect, Scene, Shape, Snapshot};

const WHITE: Color = Color::hex(0xFFFFFF);
const BLACK: Color = Color::hex(0x000000);

fn render(items: Vec<Item>, scale: f64) -> Snapshot {
    let scene = Scene {
        width: 40,
        height: 20,
        background: WHITE,
        items,
    };
    let mut renderer = CpuRenderer::new();
    let mut rgba = vec![0; (40.0 * scale * 20.0 * scale * 4.0) as usize];
    renderer
        .render_into(&scene, scale, &mut rgba)
        .expect("frame");
    Snapshot {
        width: (40.0 * scale) as u32,
        height: (20.0 * scale) as u32,
        rgba,
    }
}

fn red(snapshot: &Snapshot, x: u32, y: u32) -> u8 {
    snapshot.rgba[((y * snapshot.width + x) * 4) as usize]
}

fn square() -> Item {
    Item::Fill {
        shape: Shape::rect(Rect::new(4.0, 4.0, 8.0, 8.0)),
        paint: BLACK.into(),
    }
}

#[test]
fn translated_content_lands_on_whole_device_pixels_at_fractional_scales() {
    let scale = 1.5;
    let moved = render(
        vec![Item::Translate {
            offset: Point::new(10.0, 3.0),
            items: vec![square()],
        }],
        scale,
    );
    let left = (14.0_f64 * scale).round() as u32;
    let top = (7.0_f64 * scale).round() as u32;
    for y in top..top + 12 {
        for x in left..left + 12 {
            assert_eq!(red(&moved, x, y), 0, "covered at {x},{y}");
        }
    }
    assert_eq!(red(&moved, left - 1, top + 4), 255);
    assert_eq!(red(&moved, left + 4, top - 1), 255);
}

#[test]
fn an_opacity_layer_blends_its_content_as_a_whole() {
    let faded = render(
        vec![Item::Opacity {
            alpha: 0.5,
            items: vec![square(), square()],
        }],
        1.0,
    );
    let value = red(&faded, 8, 8);
    assert!(
        (126..=129).contains(&value),
        "half black over white, got {value}"
    );
}

#[test]
fn a_line_is_stroked_with_round_caps_between_its_ends() {
    let line = render(
        vec![Item::Line {
            from: Point::new(10.0, 10.5),
            to: Point::new(30.0, 10.5),
            width: 1.0,
            paint: BLACK.into(),
        }],
        1.0,
    );
    for x in 10..30 {
        assert_eq!(red(&line, x, 10), 0, "covered at {x}");
    }
    assert!(
        red(&line, 9, 10) < 255,
        "the round cap reaches past the start"
    );
    assert_eq!(red(&line, 20, 9), 255);
    assert_eq!(red(&line, 20, 11), 255);
}

#[test]
fn a_closed_path_fills_the_area_it_encloses() {
    let triangle = Path::through(&[
        Point::new(0.0, 0.0),
        Point::new(20.0, 0.0),
        Point::new(0.0, 20.0),
    ])
    .close();
    let frame = render(
        vec![Item::FillPath {
            path: triangle,
            paint: BLACK.into(),
        }],
        1.0,
    );
    assert_eq!(red(&frame, 3, 3), 0, "inside the triangle");
    assert_eq!(red(&frame, 16, 16), 255, "beyond its slanted side");
}

#[test]
fn a_stroked_polyline_follows_its_points() {
    let graph = Path::through(&[
        Point::new(2.0, 18.0),
        Point::new(20.0, 2.0),
        Point::new(38.0, 18.0),
    ]);
    let frame = render(
        vec![Item::StrokePath {
            path: graph,
            width: 2.0,
            paint: BLACK.into(),
        }],
        1.0,
    );
    assert!(red(&frame, 20, 2) < 128, "on the peak");
    assert!(red(&frame, 29, 10) < 128, "on the falling edge");
    assert_eq!(red(&frame, 20, 15), 255, "under the peak");
}

#[test]
fn a_scale_layer_grows_its_content_around_the_origin_given() {
    let frame = render(
        vec![Item::Scale {
            origin: Point::new(20.0, 10.0),
            factor: 2.0,
            items: vec![Item::Fill {
                shape: Shape::rect(Rect::new(15.0, 5.0, 10.0, 10.0)),
                paint: BLACK.into(),
            }],
        }],
        1.0,
    );
    assert_eq!(red(&frame, 11, 10), 0, "the left edge moved out to x 10");
    assert_eq!(red(&frame, 9, 10), 255);
    assert_eq!(red(&frame, 28, 1), 0, "and the top edge up to y 0");
}
