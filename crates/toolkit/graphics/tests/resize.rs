use bande_graphics::{Point, Radius, Rect, ResizeEdge, resize_zone};

const WINDOW: Rect = Rect::new(100.0, 100.0, 200.0, 150.0);
const OUTSIDE: f32 = 8.0;

fn zone(x: f32, y: f32) -> Option<ResizeEdge> {
    resize_zone(WINDOW, Radius::XL, OUTSIDE, Point::new(x, y))
}

#[test]
fn the_wedge_and_the_curve_of_a_rounded_corner_resize_from_that_corner() {
    assert_eq!(zone(299.0, 249.0), Some(ResizeEdge::BottomRight));
    assert_eq!(zone(294.5, 244.5), Some(ResizeEdge::BottomRight));
}

#[test]
fn the_band_reaches_eight_outside_and_three_inside_a_straight_edge() {
    assert_eq!(zone(298.0, 175.0), Some(ResizeEdge::Right));
    assert_eq!(zone(296.0, 175.0), None);
    assert_eq!(zone(307.0, 175.0), Some(ResizeEdge::Right));
    assert_eq!(zone(309.0, 175.0), None);
}

#[test]
fn a_corner_reaches_twenty_four_along_each_edge() {
    assert_eq!(zone(280.0, 251.0), Some(ResizeEdge::BottomRight));
    assert_eq!(zone(270.0, 251.0), Some(ResizeEdge::Bottom));
}

#[test]
fn a_tiled_window_resizes_only_from_inside() {
    let tiled = |x| resize_zone(WINDOW, 0.0, 0.0, Point::new(x, 175.0));
    assert_eq!(tiled(298.0), Some(ResizeEdge::Right));
    assert_eq!(tiled(301.0), None);
}
