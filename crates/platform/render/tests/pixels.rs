use std::error::Error;
use std::fs::{File, OpenOptions, read_dir};
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;

use bande_render::{Blur, OffscreenTarget, RoundedRect, VulkanRenderer, vulkan_errors};
use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBuffer, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Fourcc, Modifier};
use smithay::backend::renderer::{Bind, Color32F, Frame, ImportDma, Offscreen, Renderer};
use smithay::utils::{Physical, Rectangle, Transform};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;
type Area = Rectangle<i32, Physical>;

const CELL: i32 = 32;
const CLIENT: (i32, i32) = (256, 192);
const TARGET: (i32, i32) = (1280, 720);
const PASSES: u32 = 3;
const OFFSET: f64 = 1.5;
const REACH: i32 = 24;

struct Gpu {
    renderer: VulkanRenderer,
    reader: VulkanRenderer,
    gbm: GbmDevice<Arc<File>>,
}

fn gpu() -> Option<Gpu> {
    let nodes = read_dir("/dev/dri").ok()?.flatten();
    nodes
        .map(|entry| entry.path())
        .filter(|path| path.to_string_lossy().contains("renderD"))
        .find_map(|node| {
            let file = OpenOptions::new().read(true).write(true).open(node).ok()?;
            let device = file.metadata().ok()?.rdev();
            let renderer = VulkanRenderer::new(Some(device)).ok()?;
            let info = renderer.device_info();
            (!info.software && info.render_device == Some(device)).then_some(())?;
            Some(Gpu {
                renderer,
                reader: VulkanRenderer::new(Some(device)).ok()?,
                gbm: GbmDevice::new(Arc::new(file)).ok()?,
            })
        })
}

impl Gpu {
    fn layouts(&self) -> Vec<Modifier> {
        let formats = self.renderer.render_formats();
        formats
            .iter()
            .filter(|format| format.code == Fourcc::Xrgb8888)
            .map(|format| format.modifier)
            .collect()
    }

    fn buffer(
        &self,
        size: (i32, i32),
        layout: Modifier,
        flags: GbmBufferFlags,
    ) -> Option<(GbmBuffer, Dmabuf)> {
        let buffer = GbmAllocator::new(self.gbm.clone(), flags)
            .create_buffer(
                size.0.unsigned_abs(),
                size.1.unsigned_abs(),
                Fourcc::Xrgb8888,
                &[layout],
            )
            .ok()?;
        let dmabuf = buffer.export().ok()?;
        Some((buffer, dmabuf))
    }

    fn targets(&self, layout: Modifier, count: usize) -> Option<Vec<(GbmBuffer, Dmabuf)>> {
        let flags = GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT;
        (0..count)
            .map(|_| self.buffer(TARGET, layout, flags))
            .collect()
    }

    fn read(&mut self, dmabuf: &Dmabuf) -> Outcome<Vec<u8>> {
        let size = TARGET;
        let mut target: OffscreenTarget =
            self.reader.create_buffer(Fourcc::Argb8888, size.into())?;
        let texture = self.reader.import_dmabuf(dmabuf, None)?;
        let whole: Area = Rectangle::from_size(size.into());
        let mut framebuffer = self.reader.bind(&mut target)?;
        let mut frame = self
            .reader
            .render(&mut framebuffer, size.into(), Transform::Normal)?;
        frame.render_texture_from_to(
            &texture,
            Rectangle::from_size((f64::from(size.0), f64::from(size.1)).into()),
            whole,
            &[whole],
            &[],
            Transform::Normal,
            1.0,
        )?;
        frame.finish()?.wait()?;
        Ok(self.reader.read_target(&target)?)
    }
}

fn cell_colour(seed: i32, x: i32, y: i32) -> [u8; 3] {
    let (column, row) = (x.div_euclid(CELL), y.div_euclid(CELL));
    [
        ((column * 73 + row * 151 + seed * 37) % 256) as u8,
        ((column * 29 + seed * 101 + 60) % 256) as u8,
        ((row * 83 + seed * 59 + column * 7) % 256) as u8,
    ]
}

fn cells(size: (i32, i32), seed: i32) -> impl Iterator<Item = (Area, [u8; 3])> {
    (0..size.1.div_euclid(CELL) + 1)
        .flat_map(move |row| (0..size.0.div_euclid(CELL) + 1).map(move |column| (column, row)))
        .filter_map(move |(column, row)| {
            let at = (column * CELL, row * CELL);
            let cell = Rectangle::new(at.into(), (CELL, CELL).into())
                .intersection(Rectangle::from_size(size.into()))?;
            Some((cell, cell_colour(seed, at.0, at.1)))
        })
}

fn colour([red, green, blue]: [u8; 3]) -> Color32F {
    let channel = |value: u8| f32::from(value) / 255.0;
    Color32F::new(channel(red), channel(green), channel(blue), 1.0)
}

fn fill_with_cells(
    frame: &mut impl Frame<Error = bande_render::RenderError>,
    size: (i32, i32),
    seed: i32,
) -> Outcome {
    for (cell, rgb) in cells(size, seed) {
        frame.draw_solid(cell, &[Rectangle::from_size(cell.size)], colour(rgb))?;
    }
    Ok(())
}

fn draw_cells(
    renderer: &mut VulkanRenderer,
    dmabuf: &mut Dmabuf,
    size: (i32, i32),
    seed: i32,
) -> Outcome {
    let mut framebuffer = renderer.bind(dmabuf)?;
    let mut frame = renderer.render(&mut framebuffer, size.into(), Transform::Normal)?;
    fill_with_cells(&mut frame, size, seed)?;
    frame.finish()?.wait()?;
    Ok(())
}

fn blur(x: i32) -> Blur {
    Blur {
        region: RoundedRect {
            x: x as f32,
            y: 150.0,
            width: 384.0,
            height: 256.0,
            radius: 12.0,
        },
        passes: PASSES,
        offset: OFFSET as f32,
        alpha: 1.0,
        tone: None,
    }
}

fn whole(blur: &Blur) -> Area {
    let bounds = blur.bounds();
    Rectangle::new(
        (bounds.x, bounds.y).into(),
        (bounds.width, bounds.height).into(),
    )
}

#[derive(Clone)]
struct Level {
    size: (usize, usize),
    pixels: Vec<[f64; 3]>,
}

impl Level {
    fn drawn(size: (usize, usize), pixel: impl Fn(f64, f64) -> [f64; 3]) -> Self {
        let pixels = (0..size.1)
            .flat_map(|y| (0..size.0).map(move |x| (x, y)))
            .map(|(x, y)| {
                let uv = (
                    (x as f64 + 0.5) / size.0 as f64,
                    (y as f64 + 0.5) / size.1 as f64,
                );
                pixel(uv.0, uv.1).map(|value| (value * 255.0).round() / 255.0)
            })
            .collect();
        Self { size, pixels }
    }

    fn sample(&self, u: f64, v: f64) -> [f64; 3] {
        let place = |at: f64, length: usize| {
            let at = at * length as f64 - 0.5;
            let last = length as f64 - 1.0;
            let from = at.floor();
            (
                from.clamp(0.0, last) as usize,
                (from + 1.0).clamp(0.0, last) as usize,
                at - from,
            )
        };
        let (left, right, across) = place(u, self.size.0);
        let (top, bottom, down) = place(v, self.size.1);
        let at = |x: usize, y: usize| self.pixels[y * self.size.0 + x];
        std::array::from_fn(|channel| {
            let row = |y| at(left, y)[channel] * (1.0 - across) + at(right, y)[channel] * across;
            row(top) * (1.0 - down) + row(bottom) * down
        })
    }

    fn weighted(&self, samples: &[(f64, f64, f64)]) -> [f64; 3] {
        let mut sum = [0.0; 3];
        for &(u, v, weight) in samples {
            let sample = self.sample(u, v);
            for channel in 0..3 {
                sum[channel] += sample[channel] * weight;
            }
        }
        sum
    }

    fn down(&self, size: (usize, usize)) -> Self {
        let (x, y) = (
            OFFSET * 0.5 / self.size.0 as f64,
            OFFSET * 0.5 / self.size.1 as f64,
        );
        Self::drawn(size, |u, v| {
            self.weighted(&[
                (u, v, 0.5),
                (u - x, v - y, 0.125),
                (u + x, v + y, 0.125),
                (u + x, v - y, 0.125),
                (u - x, v + y, 0.125),
            ])
        })
    }

    fn up(&self, size: (usize, usize)) -> Self {
        let (x, y) = (OFFSET * 0.5 / size.0 as f64, OFFSET * 0.5 / size.1 as f64);
        Self::drawn(size, |u, v| {
            self.weighted(&[
                (u - 2.0 * x, v, 1.0 / 12.0),
                (u + 2.0 * x, v, 1.0 / 12.0),
                (u, v - 2.0 * y, 1.0 / 12.0),
                (u, v + 2.0 * y, 1.0 / 12.0),
                (u - x, v + y, 1.0 / 6.0),
                (u + x, v + y, 1.0 / 6.0),
                (u + x, v - y, 1.0 / 6.0),
                (u - x, v - y, 1.0 / 6.0),
            ])
        })
    }
}

fn blurred(area: Area, pixel: impl Fn(i32, i32) -> [u8; 3]) -> Level {
    let size = (area.size.w as usize, area.size.h as usize);
    let sizes: Vec<_> = (0..=PASSES)
        .map(|level| (size.0 >> level, size.1 >> level))
        .collect();
    let mut level = Level {
        size,
        pixels: (0..area.size.h)
            .flat_map(|y| (0..area.size.w).map(move |x| (x, y)))
            .map(|(x, y)| {
                pixel(area.loc.x + x, area.loc.y + y).map(|value| f64::from(value) / 255.0)
            })
            .collect(),
    };
    for &size in &sizes[1..] {
        level = level.down(size);
    }
    for &size in sizes[..PASSES as usize].iter().rev() {
        level = level.up(size);
    }
    level
}

fn farthest(found: &[u8], area: Area, expected: impl Fn(i32, i32) -> [f64; 3]) -> f64 {
    (area.loc.y..area.loc.y + area.size.h)
        .flat_map(|y| (area.loc.x..area.loc.x + area.size.w).map(move |x| (x, y)))
        .map(|(x, y)| {
            let at = ((y * TARGET.0 + x) * 4) as usize;
            let expected = expected(x, y);
            (0..3)
                .map(|channel| (f64::from(found[at + 2 - channel]) - expected[channel]).abs())
                .fold(0.0, f64::max)
        })
        .fold(0.0, f64::max)
}

fn blurred_matches(found: &[u8], blur: &Blur, level: &Level, case: &str) {
    let bounds = whole(blur);
    let inside = Rectangle::new(
        bounds.loc + smithay::utils::Point::from((REACH + 12, REACH + 12)),
        (
            bounds.size.w - 2 * (REACH + 12),
            bounds.size.h - 2 * (REACH + 12),
        )
            .into(),
    );
    let off = farthest(found, inside, |x, y| {
        let at = (y - bounds.loc.y) as usize * level.size.0 + (x - bounds.loc.x) as usize;
        level.pixels[at].map(|value| value * 255.0)
    });
    assert!(off <= 3.0, "{case}: a blurred pixel is off by {off}");
}

#[test]
fn every_texture_of_a_frame_shows_its_own_pixels() -> Outcome {
    let Some(mut gpu) = gpu() else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return Ok(());
    };
    let mut clients = Vec::new();
    for seed in 0..6 {
        let (buffer, mut dmabuf) = gpu
            .buffer(CLIENT, Modifier::Linear, GbmBufferFlags::RENDERING)
            .ok_or("no client buffer")?;
        draw_cells(&mut gpu.renderer, &mut dmabuf, CLIENT, seed)?;
        clients.push((buffer, dmabuf));
    }
    let place = |index: i32| -> Area {
        Rectangle::new(
            (index % 3 * 400 + 20, index / 3 * 330 + 20).into(),
            CLIENT.into(),
        )
    };
    for with_blur in [false, true, false] {
        for layout in gpu.layouts() {
            let Some(mut targets) = gpu.targets(layout, 3) else {
                println!("skipped: no buffer in layout {layout:?}");
                continue;
            };
            for step in 0..7 {
                let textures = clients
                    .iter()
                    .map(|(_, dmabuf)| gpu.renderer.import_dmabuf(dmabuf, None))
                    .collect::<Result<Vec<_>, _>>()?;
                let backdrop = gpu.renderer.backdrop(1, step);
                let mut framebuffer = gpu.renderer.bind(&mut targets[step as usize % 3].1)?;
                let mut frame =
                    gpu.renderer
                        .render(&mut framebuffer, TARGET.into(), Transform::Normal)?;
                frame.clear(
                    Color32F::new(0.1, 0.2, 0.3, 1.0),
                    &[Rectangle::from_size(TARGET.into())],
                )?;
                if with_blur {
                    let blur = blur(200);
                    frame.draw_blur(&blur, backdrop, &[Rectangle::from_size(whole(&blur).size)])?;
                }
                for (texture, index) in textures.iter().zip(0..) {
                    frame.render_texture_from_to(
                        texture,
                        Rectangle::from_size((f64::from(CLIENT.0), f64::from(CLIENT.1)).into()),
                        place(index),
                        &[Rectangle::from_size(CLIENT.into())],
                        &[],
                        Transform::Normal,
                        1.0,
                    )?;
                }
                frame.finish()?.wait()?;
            }
            let found = gpu.read(&targets[0].1)?;
            for index in 0..6 {
                let area = place(index);
                let off = farthest(&found, area, |x, y| {
                    cell_colour(index, x - area.loc.x, y - area.loc.y).map(f64::from)
                });
                assert!(
                    off <= 1.0,
                    "texture {index} in layout {layout:?} (blur: {with_blur}) is off by {off}"
                );
            }
        }
    }
    assert_eq!(vulkan_errors(), 0, "Vulkan reported errors");
    Ok(())
}

#[test]
fn equal_blurs_show_what_lies_beneath_each_in_every_frame() -> Outcome {
    let Some(mut gpu) = gpu() else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return Ok(());
    };
    let blurs = [blur(100), blur(700)];
    let expected = blurs.map(|blur| blurred(whole(&blur), |x, y| cell_colour(0, x, y)));
    for layout in gpu.layouts() {
        let Some(mut targets) = gpu.targets(layout, 3) else {
            println!("skipped: no buffer in layout {layout:?}");
            continue;
        };
        for step in 0..5 {
            let backdrops = [1, 2].map(|owner| gpu.renderer.backdrop(owner, step));
            let target = &mut targets[step as usize % 3].1;
            {
                let mut framebuffer = gpu.renderer.bind(target)?;
                let mut frame =
                    gpu.renderer
                        .render(&mut framebuffer, TARGET.into(), Transform::Normal)?;
                fill_with_cells(&mut frame, TARGET, 0)?;
                for (blur, backdrop) in blurs.iter().zip(backdrops) {
                    frame.draw_blur(blur, backdrop, &[Rectangle::from_size(whole(blur).size)])?;
                }
                frame.finish()?.wait()?;
            }
            let found = gpu.read(&targets[step as usize % 3].1)?;
            for (blur, level) in blurs.iter().zip(&expected) {
                blurred_matches(
                    &found,
                    blur,
                    level,
                    &format!("frame {step} in layout {layout:?}"),
                );
            }
        }
    }
    assert_eq!(vulkan_errors(), 0, "Vulkan reported errors");
    Ok(())
}

#[test]
fn a_part_of_a_blur_redrawn_shows_the_whole_backdrop() -> Outcome {
    let Some(mut gpu) = gpu() else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return Ok(());
    };
    let blur = blur(300);
    let bounds = whole(&blur);
    let panel = Rectangle::new((300, 150).into(), (384, 256).into());
    let part: Area = Rectangle::new((400, 240).into(), (60, 40).into());
    let expected = blurred(bounds, |x, y| cell_colour(0, x, y));
    let mixed = Level {
        size: expected.size,
        pixels: expected
            .pixels
            .iter()
            .map(|pixel| pixel.map(|value| value * 0.6 + 0.4))
            .collect(),
    };
    for layout in gpu.layouts() {
        let Some(mut targets) = gpu.targets(layout, 1) else {
            println!("skipped: no buffer in layout {layout:?}");
            continue;
        };
        for (step, damage) in [Rectangle::from_size(TARGET.into()), part, part]
            .into_iter()
            .enumerate()
        {
            let backdrop = gpu.renderer.backdrop(1, 1);
            {
                let mut framebuffer = gpu.renderer.bind(&mut targets[0].1)?;
                let mut frame =
                    gpu.renderer
                        .render(&mut framebuffer, TARGET.into(), Transform::Normal)?;
                let within = |area: Area| {
                    damage.intersection(area).map(|mut damaged| {
                        damaged.loc -= area.loc;
                        damaged
                    })
                };
                for (cell, rgb) in cells(TARGET, 0) {
                    if let Some(damaged) = within(cell) {
                        frame.draw_solid(cell, &[damaged], colour(rgb))?;
                    }
                }
                if let Some(damaged) = within(bounds) {
                    frame.draw_blur(&blur, backdrop, &[damaged])?;
                }
                if let Some(damaged) = within(panel) {
                    frame.draw_solid(panel, &[damaged], Color32F::new(0.4, 0.4, 0.4, 0.4))?;
                }
                frame.finish()?.wait()?;
            }
            let found = gpu.read(&targets[0].1)?;
            blurred_matches(
                &found,
                &blur,
                &mixed,
                &format!("draw {step} in layout {layout:?}"),
            );
        }
        let next = gpu.renderer.backdrop(1, 2);
        assert_ne!(
            next.drawing,
            gpu.renderer.backdrop(1, 1).drawing,
            "a changed backdrop must be redrawn whole"
        );
    }
    assert_eq!(vulkan_errors(), 0, "Vulkan reported errors");
    Ok(())
}
