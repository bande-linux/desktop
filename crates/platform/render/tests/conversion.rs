use std::error::Error;
use std::fs::OpenOptions;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;

use bande_render::{Encode, OffscreenTarget, OutputColour, VulkanRenderer, vulkan_errors};
use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBuffer, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Fourcc, Modifier};
use smithay::backend::renderer::{Bind, Color32F, Frame, ImportDma, Offscreen, Renderer};
use smithay::utils::{Physical, Rectangle, Transform};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

const SIZE: (i32, i32) = (640, 360);
const TILE: (i32, i32) = (160, 180);
const GAMMA: f64 = 2.2;
const ENTRIES: usize = 4096;
const MATRIX: [[f32; 3]; 3] = [[0.7, 0.26, 0.04], [0.05, 0.92, 0.03], [0.02, 0.08, 0.9]];
const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

struct Gpu {
    renderer: VulkanRenderer,
    allocator: GbmAllocator<std::fs::File>,
}

fn hardware() -> Outcome<Option<Gpu>> {
    let nodes = std::fs::read_dir("/dev/dri")
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_else(|_| Vec::new());
    for node in nodes {
        if !node.to_string_lossy().contains("renderD") {
            continue;
        }
        let Ok(file) = OpenOptions::new().read(true).write(true).open(&node) else {
            continue;
        };
        let device = file.metadata()?.rdev();
        let Ok(renderer) = VulkanRenderer::new(Some(device)) else {
            continue;
        };
        let info = renderer.device_info();
        if info.software || info.render_device != Some(device) {
            continue;
        }
        let allocator = GbmAllocator::new(GbmDevice::new(file)?, GbmBufferFlags::RENDERING);
        return Ok(Some(Gpu {
            renderer,
            allocator,
        }));
    }
    Ok(None)
}

impl Gpu {
    fn buffer(&mut self, code: Fourcc, modifier: Modifier) -> Outcome<(GbmBuffer, Dmabuf)> {
        let (width, height) = (SIZE.0 as u32, SIZE.1 as u32);
        let buffer = self
            .allocator
            .create_buffer(width, height, code, &[modifier])?;
        let dmabuf = buffer.export()?;
        Ok((buffer, dmabuf))
    }

    fn pixels(&mut self, dmabuf: &Dmabuf) -> Outcome<Vec<u8>> {
        let whole = Rectangle::from_size(SIZE.into());
        let texture = self.renderer.import_dmabuf(dmabuf, None)?;
        let mut target: OffscreenTarget =
            self.renderer.create_buffer(Fourcc::Argb8888, SIZE.into())?;
        let mut framebuffer = self.renderer.bind(&mut target)?;
        let mut frame = self
            .renderer
            .render(&mut framebuffer, SIZE.into(), Transform::Normal)?;
        frame.clear(Color32F::TRANSPARENT, &[whole])?;
        frame.render_texture_from_to(
            &texture,
            Rectangle::from_size((f64::from(SIZE.0), f64::from(SIZE.1)).into()),
            whole,
            &[whole],
            &[],
            Transform::Normal,
            1.0,
        )?;
        frame.finish()?.wait()?;
        Ok(self.renderer.read_target(&target)?)
    }
}

fn tile(index: i32) -> Rectangle<i32, Physical> {
    Rectangle::new(
        ((index % 4) * TILE.0, (index / 4) * TILE.1).into(),
        TILE.into(),
    )
}

fn colour(index: i32, turn: i32) -> [u8; 3] {
    [37, 91, 53].map(|step| ((index * step + turn * 71 + 19) % 256) as u8)
}

fn paint(renderer: &mut VulkanRenderer, dmabuf: &mut Dmabuf, turn: i32, tiles: &[i32]) -> Outcome {
    let mut framebuffer = renderer.bind(dmabuf)?;
    let mut frame = renderer.render(&mut framebuffer, SIZE.into(), Transform::Normal)?;
    for &index in tiles {
        let [red, green, blue] = colour(index, turn).map(|value| f32::from(value) / 255.0);
        frame.clear(Color32F::new(red, green, blue, 1.0), &[tile(index)])?;
    }
    frame.finish()?.wait()?;
    Ok(())
}

fn profile(matrix: [[f32; 3]; 3]) -> OutputColour {
    let encode: Vec<[u16; 3]> = (0..ENTRIES)
        .map(|entry| {
            let root = entry as f64 / (ENTRIES - 1) as f64;
            [((root * root).powf(1.0 / GAMMA) * 65535.0).round() as u16; 3]
        })
        .collect();
    OutputColour {
        matrix,
        encode: Encode::Table(Arc::from(encode)),
        extended: false,
    }
}

fn shown(colour: [u8; 3], profile: Option<&OutputColour>) -> [f64; 3] {
    let Some(profile) = profile else {
        return colour.map(f64::from);
    };
    let linear = colour.map(|value| {
        let encoded = f64::from(value) / 255.0;
        match encoded <= 0.04045 {
            true => encoded / 12.92,
            false => ((encoded + 0.055) / 1.055).powf(2.4),
        }
    });
    profile.matrix.map(|row| {
        let value: f64 = row
            .iter()
            .zip(linear)
            .map(|(factor, value)| f64::from(*factor) * value)
            .sum();
        value.clamp(0.0, 1.0).powf(1.0 / GAMMA) * 255.0
    })
}

fn wrong_tiles(pixels: &[u8], wanted: impl Fn(i32) -> [f64; 3]) -> Vec<i32> {
    (0..8)
        .filter(|&index| {
            let place = tile(index);
            let [red, green, blue] = wanted(index);
            let matches = |x: i32, y: i32| {
                let at = ((y * SIZE.0 + x) * 4) as usize;
                [blue, green, red]
                    .iter()
                    .zip(&pixels[at..at + 3])
                    .all(|(wanted, found)| (wanted - f64::from(*found)).abs() <= 2.0)
            };
            !(place.loc.y..place.loc.y + place.size.h)
                .all(|y| (place.loc.x..place.loc.x + place.size.w).all(|x| matches(x, y)))
        })
        .collect()
}

#[test]
fn only_the_frames_of_a_display_are_converted_and_each_pixel_once() -> Outcome {
    let Some(mut gpu) = hardware()? else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return Ok(());
    };
    let modifiers: Vec<Modifier> = gpu
        .renderer
        .render_formats()
        .iter()
        .filter(|format| format.code == Fourcc::Argb8888)
        .map(|format| format.modifier)
        .collect();
    let all: Vec<i32> = (0..8).collect();
    for modifier in modifiers {
        let (_shown, mut display) = gpu.buffer(Fourcc::Argb8888, modifier)?;
        let (_captured, mut capture) = gpu.buffer(Fourcc::Argb8888, modifier)?;
        for matrix in [MATRIX, IDENTITY] {
            let profile = profile(matrix);
            let mut shows = [0; 8];
            for (turn, tiles) in [(0, &all[..]), (1, &all[2..4]), (2, &all[3..6])] {
                gpu.renderer.converting(Some(&profile), |renderer| {
                    paint(renderer, &mut display, turn, tiles)
                })??;
                tiles.iter().for_each(|&index| shows[index as usize] = turn);
                let found = gpu.pixels(&display)?;
                let wrong = wrong_tiles(&found, |index| {
                    shown(colour(index, shows[index as usize]), Some(&profile))
                });
                assert!(
                    wrong.is_empty(),
                    "tiles {wrong:?} are wrong after turn {turn} in {modifier:?}"
                );
            }
            paint(&mut gpu.renderer, &mut capture, 0, &all)?;
            let found = gpu.pixels(&capture)?;
            let wrong = wrong_tiles(&found, |index| shown(colour(index, 0), None));
            assert!(
                wrong.is_empty(),
                "a capture is converted in tiles {wrong:?}"
            );
        }
        let mut framebuffer = gpu.renderer.bind(&mut capture)?;
        let mut frame = gpu
            .renderer
            .render(&mut framebuffer, SIZE.into(), Transform::Normal)?;
        frame.clear(Color32F::TRANSPARENT, &[Rectangle::from_size(SIZE.into())])?;
        frame.finish()?.wait()?;
        assert!(
            gpu.pixels(&capture)?.iter().all(|byte| *byte == 0),
            "a transparent capture became opaque in {modifier:?}"
        );
    }
    assert_eq!(vulkan_errors(), 0, "Vulkan reported errors");
    Ok(())
}
