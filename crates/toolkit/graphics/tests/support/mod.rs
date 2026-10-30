#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(dead_code)]

use std::path::Path;

use ab_glyph::{Font as _, FontRef, GlyphId, OutlineCurve};
use image::{RgbImage, buffer::ConvertBuffer};
use unbound_graphics::{Color, Item, Paint, Rect, Scene, Shape, Snapshot};

pub const MIN_SSIM: f64 = 0.995;
pub const MAX_CHANNEL_DELTA: u8 = 2;

const SLICES: usize = 64;
const CURVE_STEPS: usize = 64;

struct Raster {
    width: usize,
    height: usize,
    accumulated: Vec<f64>,
}

impl Raster {
    fn new(width: u32, height: u32) -> Self {
        let (width, height) = (width as usize + 2, height as usize);
        Self {
            width,
            height,
            accumulated: vec![0.0; width * height + 1],
        }
    }

    fn line(&mut self, from: (f64, f64), to: (f64, f64)) {
        let shift = |point: (f64, f64)| (point.0 + 1.0, point.1);
        let (from, to) = (shift(from), shift(to));
        if from.1 == to.1 {
            return;
        }
        let (direction, top, bottom) = if from.1 < to.1 {
            (1.0, from, to)
        } else {
            (-1.0, to, from)
        };
        let slope = (bottom.0 - top.0) / (bottom.1 - top.1);
        let limit = (self.width - 1) as f64;
        let first_row = top.1.max(0.0).floor() as usize;
        let last_row = (bottom.1.ceil().max(0.0) as usize).min(self.height);
        for row in first_row..last_row {
            let row_top = (row as f64).max(top.1);
            let row_bottom = (row as f64 + 1.0).min(bottom.1);
            if row_bottom <= row_top {
                continue;
            }
            let x_top = (top.0 + (row_top - top.1) * slope).clamp(0.0, limit);
            let x_bottom = (top.0 + (row_bottom - top.1) * slope).clamp(0.0, limit);
            self.span(row, x_top, x_bottom, (row_bottom - row_top) * direction);
        }
    }

    fn span(&mut self, row: usize, x_top: f64, x_bottom: f64, height: f64) {
        let start = row * self.width;
        let (left, right) = (x_top.min(x_bottom), x_top.max(x_bottom));
        let first = left.floor() as usize;
        let last = (right.ceil() as usize).max(first + 1);
        let mut previous = 0.0;
        for cell in first..=last {
            let covered = height * right_of_edge(cell as f64, left, right);
            self.accumulated[start + cell] += covered - previous;
            previous = covered;
        }
    }

    fn coverage(&self) -> Vec<f64> {
        let mut sum = 0.0;
        let winding: Vec<f64> = self
            .accumulated
            .iter()
            .map(|delta| {
                sum += delta;
                sum
            })
            .collect();
        winding
            .chunks(self.width)
            .take(self.height)
            .flat_map(|row| {
                row[1..self.width - 1]
                    .iter()
                    .map(|value| value.abs().min(1.0))
            })
            .collect()
    }
}

fn right_of_edge(cell: f64, left: f64, right: f64) -> f64 {
    let fraction = |x: f64| (cell + 1.0 - x).clamp(0.0, 1.0);
    if right - left < 1e-12 {
        return fraction(left);
    }
    let antiderivative = |x: f64| {
        let into = (x - cell).clamp(0.0, 1.0);
        x.min(cell) + into - into * into / 2.0
    };
    (antiderivative(right) - antiderivative(left)) / (right - left)
}

pub struct Reference {
    width: u32,
    height: u32,
    premultiplied: Vec<[f64; 4]>,
}

fn unit(channel: u8) -> f64 {
    f64::from(channel) / 255.0
}

fn rgba(color: Color) -> [f64; 4] {
    [unit(color.r), unit(color.g), unit(color.b), unit(color.a)]
}

impl Reference {
    pub fn new(width: u32, height: u32, background: Color) -> Self {
        let [r, g, b, a] = rgba(background);
        Self {
            width,
            height,
            premultiplied: vec![[r * a, g * a, b * a, a]; (width * height) as usize],
        }
    }

    pub fn draw(scene: &Scene) -> RgbImage {
        let mut reference = Self::new(scene.width, scene.height, scene.background);
        reference.items(&scene.items, None);
        reference.image()
    }

    fn items(&mut self, items: &[Item], clip: Option<Shape>) {
        for item in items {
            match item {
                Item::Fill { shape, paint } => self.blend(
                    clip,
                    |x, y| area_coverage(shape, x, y),
                    |x, y| paint_at(paint, x, y),
                ),
                Item::Stroke {
                    shape,
                    width,
                    paint,
                } => {
                    let outer =
                        Shape::rounded(shape.rect.inset(-width / 2.0), shape.radius + width / 2.0);
                    let inner = Shape::rounded(
                        shape.rect.inset(width / 2.0),
                        (shape.radius - width / 2.0).max(0.0),
                    );
                    self.blend(
                        clip,
                        |x, y| area_coverage(&outer, x, y) - area_coverage(&inner, x, y),
                        |x, y| paint_at(paint, x, y),
                    )
                }
                Item::Shadow {
                    shape,
                    offset,
                    blur_radius,
                    color,
                } => {
                    let moved = Shape::rounded(
                        Rect::new(
                            shape.rect.x + offset.x,
                            shape.rect.y + offset.y,
                            shape.rect.width,
                            shape.rect.height,
                        ),
                        shape.radius,
                    );
                    let sigma = f64::from(*blur_radius) / 2.0;
                    self.blend(
                        clip,
                        |x, y| {
                            gaussian_coverage(&moved, sigma, f64::from(x) + 0.5, f64::from(y) + 0.5)
                        },
                        |_, _| rgba(*color),
                    )
                }
                Item::Text { origin, paragraph } => {
                    for run in paragraph.glyph_runs() {
                        let font =
                            FontRef::try_from_slice_and_index(run.font_bytes(), run.font_index())
                                .expect("the run's font parses");
                        assert!(
                            run.variation_coords().iter().all(|coord| *coord == 0),
                            "the reference rasterizer draws static instances only"
                        );
                        let scale = f64::from(run.size)
                            / f64::from(font.units_per_em().expect("units per em"));
                        for glyph in &run.glyphs {
                            let mut raster = Raster::new(self.width, self.height);
                            let id = GlyphId(u16::try_from(glyph.id).expect("16-bit glyph id"));
                            let Some(outline) = font.outline(id) else {
                                continue;
                            };
                            let x = f64::from(origin.x + glyph.x);
                            let y = f64::from(origin.y + glyph.y);
                            let place = |p: ab_glyph::Point| {
                                (x + f64::from(p.x) * scale, y - f64::from(p.y) * scale)
                            };
                            for curve in &outline.curves {
                                let points: Vec<(f64, f64)> = match curve {
                                    OutlineCurve::Line(a, b) => vec![place(*a), place(*b)],
                                    OutlineCurve::Quad(a, b, c) => {
                                        let (a, b, c) = (place(*a), place(*b), place(*c));
                                        (0..=CURVE_STEPS)
                                            .map(|step| {
                                                let t = step as f64 / CURVE_STEPS as f64;
                                                let u = 1.0 - t;
                                                (
                                                    u * u * a.0 + 2.0 * u * t * b.0 + t * t * c.0,
                                                    u * u * a.1 + 2.0 * u * t * b.1 + t * t * c.1,
                                                )
                                            })
                                            .collect()
                                    }
                                    OutlineCurve::Cubic(a, b, c, d) => {
                                        let (a, b, c, d) =
                                            (place(*a), place(*b), place(*c), place(*d));
                                        (0..=CURVE_STEPS)
                                            .map(|step| {
                                                let t = step as f64 / CURVE_STEPS as f64;
                                                let u = 1.0 - t;
                                                let weights = [
                                                    u * u * u,
                                                    3.0 * u * u * t,
                                                    3.0 * u * t * t,
                                                    t * t * t,
                                                ];
                                                (
                                                    weights[0] * a.0
                                                        + weights[1] * b.0
                                                        + weights[2] * c.0
                                                        + weights[3] * d.0,
                                                    weights[0] * a.1
                                                        + weights[1] * b.1
                                                        + weights[2] * c.1
                                                        + weights[3] * d.1,
                                                )
                                            })
                                            .collect()
                                    }
                                };
                                for segment in points.windows(2) {
                                    raster.line(segment[0], segment[1]);
                                }
                            }
                            let coverage = raster.coverage();
                            let width = self.width;
                            self.blend(
                                clip,
                                |x, y| coverage[(y * width + x) as usize],
                                |_, _| rgba(run.color),
                            );
                        }
                    }
                }
                Item::Clip { shape, items } => self.items(items, Some(*shape)),
                Item::Translate { .. }
                | Item::Opacity { .. }
                | Item::Scale { .. }
                | Item::Line { .. }
                | Item::FillPath { .. }
                | Item::StrokePath { .. } => {
                    panic!("the reference scenes do not use layers, lines or paths")
                }
                Item::Image { rect, image } => {
                    assert!(
                        rect.width as u32 == image.width()
                            && rect.height as u32 == image.height()
                            && rect.x.fract() == 0.0
                            && rect.y.fract() == 0.0,
                        "the reference draws images at their own size on whole pixels"
                    );
                    let pixels = image.premultiplied_rgba();
                    let (left, top) = (rect.x as i64, rect.y as i64);
                    let width = i64::from(image.width());
                    self.blend(
                        clip,
                        |x, y| area_coverage(&Shape::rect(*rect), x, y),
                        |x, y| {
                            let at = (((i64::from(y) - top) * width + i64::from(x) - left) * 4)
                                .max(0) as usize;
                            let [r, g, b, a] =
                                [0, 1, 2, 3].map(|channel| f64::from(pixels[at + channel]) / 255.0);
                            if a == 0.0 {
                                [0.0; 4]
                            } else {
                                [r / a, g / a, b / a, a]
                            }
                        },
                    );
                }
            }
        }
    }

    fn blend(
        &mut self,
        clip: Option<Shape>,
        coverage: impl Fn(u32, u32) -> f64,
        color: impl Fn(u32, u32) -> [f64; 4],
    ) {
        for y in 0..self.height {
            for x in 0..self.width {
                let clip = clip.map_or(1.0, |shape| area_coverage(&shape, x, y));
                let cover = coverage(x, y).clamp(0.0, 1.0) * clip;
                if cover == 0.0 {
                    continue;
                }
                let [r, g, b, a] = color(x, y);
                let alpha = a * cover;
                let pixel = &mut self.premultiplied[(y * self.width + x) as usize];
                for (channel, source) in
                    pixel
                        .iter_mut()
                        .zip([r * alpha, g * alpha, b * alpha, alpha])
                {
                    *channel = source + *channel * (1.0 - alpha);
                }
            }
        }
    }

    pub fn image(&self) -> RgbImage {
        RgbImage::from_fn(self.width, self.height, |x, y| {
            let pixel = self.premultiplied[(y * self.width + x) as usize];
            image::Rgb([0, 1, 2].map(|channel| (pixel[channel] * 255.0).round() as u8))
        })
    }
}

fn paint_at(paint: &Paint, x: u32, y: u32) -> [f64; 4] {
    match paint {
        Paint::Solid(color) => rgba(*color),
        Paint::LinearGradient { start, end, stops } => {
            let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let (dx, dy) = (f64::from(end.x - start.x), f64::from(end.y - start.y));
            let t = (((px - f64::from(start.x)) * dx + (py - f64::from(start.y)) * dy)
                / (dx * dx + dy * dy))
                .clamp(0.0, 1.0);
            let after = stops
                .iter()
                .position(|stop| f64::from(stop.offset) >= t)
                .unwrap_or(stops.len() - 1);
            let before = after.saturating_sub(1);
            let (a, b) = (stops[before], stops[after]);
            let span = f64::from(b.offset - a.offset);
            let mix = if span > 0.0 {
                (t - f64::from(a.offset)) / span
            } else {
                0.0
            };
            let (a, b) = (rgba(a.color), rgba(b.color));
            [0, 1, 2, 3].map(|channel| a[channel] + (b[channel] - a[channel]) * mix)
        }
    }
}

fn vertical_extent(shape: &Shape, x: f64) -> Option<(f64, f64)> {
    let rect = shape.rect;
    let (x0, y0) = (f64::from(rect.x), f64::from(rect.y));
    let (x1, y1) = (x0 + f64::from(rect.width), y0 + f64::from(rect.height));
    if x < x0 || x > x1 {
        return None;
    }
    let radius = f64::from(shape.radius)
        .min(f64::from(rect.width) / 2.0)
        .min(f64::from(rect.height) / 2.0);
    let into_corner = (x0 + radius - x).max(x - (x1 - radius)).max(0.0);
    let inset = radius
        - (radius * radius - into_corner * into_corner)
            .max(0.0)
            .sqrt();
    Some((y0 + inset, y1 - inset))
}

pub fn area_coverage(shape: &Shape, x: u32, y: u32) -> f64 {
    let rect = shape.rect;
    let (x0, x1) = (f64::from(rect.x), f64::from(rect.x + rect.width));
    let radius = f64::from(shape.radius);
    let (left, right) = (f64::from(x), f64::from(x) + 1.0);
    let (top, bottom) = (f64::from(y), f64::from(y) + 1.0);
    let mut breaks = vec![left, right];
    breaks.extend(
        [x0, x1, x0 + radius, x1 - radius]
            .into_iter()
            .filter(|edge| *edge > left && *edge < right),
    );
    breaks.sort_by(f64::total_cmp);
    breaks
        .windows(2)
        .map(|piece| {
            let step = (piece[1] - piece[0]) / SLICES as f64;
            (0..SLICES)
                .map(|slice| {
                    let sample = piece[0] + (slice as f64 + 0.5) * step;
                    vertical_extent(shape, sample).map_or(0.0, |(from, to)| {
                        (to.min(bottom) - from.max(top)).max(0.0) * step
                    })
                })
                .sum::<f64>()
        })
        .sum()
}

fn normal_cdf(z: f64) -> f64 {
    0.5 * (1.0 + erf(z / std::f64::consts::SQRT_2))
}

fn erf(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.5 * x.abs());
    let tau = t
        * (-x * x - 1.265_512_23
            + t * (1.000_023_68
                + t * (0.374_091_96
                    + t * (0.096_784_18
                        + t * (-0.186_288_06
                            + t * (0.278_868_07
                                + t * (-1.135_203_98
                                    + t * (1.488_515_87
                                        + t * (-0.822_152_23 + t * 0.170_872_77)))))))))
            .exp();
    if x >= 0.0 { 1.0 - tau } else { tau - 1.0 }
}

pub fn gaussian_coverage(shape: &Shape, sigma: f64, px: f64, py: f64) -> f64 {
    let rect = shape.rect;
    let (y0, y1) = (f64::from(rect.y), f64::from(rect.y + rect.height));
    let from = y0.max(py - 6.0 * sigma);
    let to = y1.min(py + 6.0 * sigma);
    if from >= to {
        return 0.0;
    }
    let steps = 512;
    let step = (to - from) / f64::from(steps);
    (0..steps)
        .map(|index| {
            let y = from + (f64::from(index) + 0.5) * step;
            let (left, right) = horizontal_extent(shape, y);
            let row = normal_cdf((right - px) / sigma) - normal_cdf((left - px) / sigma);
            let weight = (-(py - y).powi(2) / (2.0 * sigma * sigma)).exp()
                / (sigma * (2.0 * std::f64::consts::PI).sqrt());
            row * weight * step
        })
        .sum()
}

fn horizontal_extent(shape: &Shape, y: f64) -> (f64, f64) {
    let rect = shape.rect;
    let (x0, y0) = (f64::from(rect.x), f64::from(rect.y));
    let (x1, y1) = (x0 + f64::from(rect.width), y0 + f64::from(rect.height));
    let radius = f64::from(shape.radius)
        .min(f64::from(rect.width) / 2.0)
        .min(f64::from(rect.height) / 2.0);
    let into_corner = (y0 + radius - y).max(y - (y1 - radius)).max(0.0);
    let inset = radius
        - (radius * radius - into_corner * into_corner)
            .max(0.0)
            .sqrt();
    (x0 + inset, x1 - inset)
}

const SSIM_WINDOW: u32 = 8;

fn ssim(first: &RgbImage, second: &RgbImage) -> f64 {
    let c1 = (0.01 * 255.0_f64).powi(2);
    let c2 = (0.03 * 255.0_f64).powi(2);
    (0..3)
        .map(|channel| {
            let mut total = 0.0;
            let mut windows = 0.0;
            for top in (0..first.height()).step_by(SSIM_WINDOW as usize) {
                for left in (0..first.width()).step_by(SSIM_WINDOW as usize) {
                    let pixels: Vec<(f64, f64)> = (top..(top + SSIM_WINDOW).min(first.height()))
                        .flat_map(|y| {
                            (left..(left + SSIM_WINDOW).min(first.width())).map(move |x| (x, y))
                        })
                        .map(|(x, y)| {
                            (
                                f64::from(first.get_pixel(x, y).0[channel]),
                                f64::from(second.get_pixel(x, y).0[channel]),
                            )
                        })
                        .collect();
                    let count = pixels.len() as f64;
                    let mean_x = pixels.iter().map(|p| p.0).sum::<f64>() / count;
                    let mean_y = pixels.iter().map(|p| p.1).sum::<f64>() / count;
                    let moment =
                        |f: &dyn Fn(&(f64, f64)) -> f64| pixels.iter().map(f).sum::<f64>() / count;
                    let variance_x = moment(&|p| (p.0 - mean_x).powi(2));
                    let variance_y = moment(&|p| (p.1 - mean_y).powi(2));
                    let covariance = moment(&|p| (p.0 - mean_x) * (p.1 - mean_y));
                    total += ((2.0 * mean_x * mean_y + c1) * (2.0 * covariance + c2))
                        / ((mean_x.powi(2) + mean_y.powi(2) + c1) * (variance_x + variance_y + c2));
                    windows += 1.0;
                }
            }
            total / windows
        })
        .fold(1.0, f64::min)
}

pub struct Comparison {
    pub ssim: f64,
    pub max_delta: u8,
}

pub fn compare(name: &str, snapshot: &Snapshot, expected: &RgbImage) -> Comparison {
    let actual: RgbImage =
        image::RgbaImage::from_raw(snapshot.width, snapshot.height, snapshot.rgba.clone())
            .expect("snapshot dimensions")
            .convert();
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"));
    actual
        .save(directory.join(format!("{name}-actual.png")))
        .expect("save rendered frame");
    expected
        .save(directory.join(format!("{name}-expected.png")))
        .expect("save reference");
    let ssim = ssim(&actual, expected);
    let max_delta = actual
        .as_raw()
        .iter()
        .zip(expected.as_raw())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    println!("{name}: SSIM {ssim:.5}, max channel delta {max_delta}");
    if max_delta > MAX_CHANNEL_DELTA {
        let mut worst: Vec<(u8, u32, u32)> = actual
            .enumerate_pixels()
            .map(|(x, y, pixel)| {
                let delta = pixel
                    .0
                    .iter()
                    .zip(expected.get_pixel(x, y).0)
                    .map(|(a, b)| a.abs_diff(b))
                    .max()
                    .unwrap_or(0);
                (delta, x, y)
            })
            .filter(|(delta, _, _)| *delta > MAX_CHANNEL_DELTA)
            .collect();
        worst.sort_unstable_by(|a, b| b.cmp(a));
        println!(
            "{name}: {} pixels differ by more than {MAX_CHANNEL_DELTA}",
            worst.len()
        );
        for (delta, x, y) in worst.into_iter().take(12) {
            println!(
                "  ({x}, {y}) delta {delta}: actual {:?}, expected {:?}",
                actual.get_pixel(x, y).0,
                expected.get_pixel(x, y).0
            );
        }
    }
    Comparison { ssim, max_delta }
}

pub fn assert_all_match(results: &[(String, Comparison)]) {
    let failures: Vec<String> = results
        .iter()
        .filter(|(_, comparison)| {
            comparison.ssim < MIN_SSIM || comparison.max_delta > MAX_CHANNEL_DELTA
        })
        .map(|(name, Comparison { ssim, max_delta })| {
            format!("{name}: SSIM {ssim:.5} (min {MIN_SSIM}), max channel delta {max_delta} (max {MAX_CHANNEL_DELTA})")
        })
        .collect();
    assert!(
        failures.is_empty(),
        "frames differ from their references (see {}):\n{}",
        env!("CARGO_TARGET_TMPDIR"),
        failures.join("\n")
    );
}
