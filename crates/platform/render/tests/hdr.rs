use std::error::Error;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;

use bande_render::{
    Encode, HdrContent, OffscreenTarget, OutputColour, VulkanRenderer, vulkan_errors,
};
use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBuffer, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Buffer, Fourcc, Modifier};
use smithay::backend::renderer::{Bind, Color32F, Frame, ImportDma, Offscreen, Renderer};
use smithay::utils::{Physical, Rectangle, Transform};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

const CELL: i32 = 16;
const COLUMNS: i32 = 16;
const SIZE: (i32, i32) = (CELL * COLUMNS, CELL * 4);
const WHITE: f64 = 203.0;
const PEAK: f64 = 600.0;
const MASTERED: f64 = 1000.0;
const TO_2020: [[f64; 3]; 3] = [
    [0.6274039, 0.3292830, 0.0433131],
    [0.0690973, 0.9195404, 0.0113623],
    [0.0163914, 0.0880133, 0.8955953],
];
const FROM_2020: [[f64; 3]; 3] = [
    [1.660491, -0.5876411, -0.0728499],
    [-0.1245505, 1.1328999, -0.0083494],
    [-0.0181508, -0.1005789, 1.1187297],
];

fn pq(nits: f64) -> f64 {
    let power = (nits / 10000.0).clamp(0.0, 1.0).powf(0.1593017578125);
    ((0.8359375 + 18.8515625 * power) / (1.0 + 18.6875 * power)).powf(78.84375)
}

fn nits(code: f64) -> f64 {
    let power = code.powf(1.0 / 78.84375);
    let ratio = (power - 0.8359375).max(0.0) / (18.8515625 - 18.6875 * power);
    10000.0 * ratio.powf(1.0 / 0.1593017578125)
}

fn decoded(value: f64) -> f64 {
    match value <= 0.04045 {
        true => value / 12.92,
        false => ((value + 0.055) / 1.055).powf(2.4),
    }
}

fn encoded(value: f64) -> f64 {
    match value <= 0.0031308 {
        true => value * 12.92,
        false => 1.055 * value.powf(1.0 / 2.4) - 0.055,
    }
}

fn through(matrix: [[f64; 3]; 3], colour: [f64; 3]) -> [f64; 3] {
    matrix.map(|row| row.iter().zip(colour).map(|(a, b)| a * b).sum())
}

fn mapped(brightest: f64, peak: f64) -> f64 {
    let source = pq(MASTERED);
    let target = pq(peak) / source;
    let value = (pq(brightest) / source).min(1.0);
    let knee = (1.5 * target - 0.5).max(0.0);
    if value <= knee {
        return brightest;
    }
    let t = (value - knee) / (1.0 - knee);
    let curved = (2.0 * t.powi(3) - 3.0 * t * t + 1.0) * knee
        + (t.powi(3) - 2.0 * t * t + t) * (1.0 - knee)
        + (3.0 * t * t - 2.0 * t.powi(3)) * target;
    nits(curved * source)
}

fn toned(content: [f64; 3], peak: f64) -> [f64; 3] {
    let light = content.map(nits);
    let brightest = light.into_iter().fold(0.0001, f64::max);
    light.map(|value| value * mapped(brightest, peak) / brightest)
}

fn cell(index: i32) -> Rectangle<i32, Physical> {
    Rectangle::new(
        ((index % COLUMNS) * CELL, (index / COLUMNS) * CELL).into(),
        (CELL, CELL).into(),
    )
}

fn content(index: i32) -> [f64; 3] {
    let level = pq(f64::from(index % COLUMNS + 1) / f64::from(COLUMNS) * MASTERED);
    match index / COLUMNS {
        0 => [level; 3],
        1 => [level, level * 0.6, level * 0.7],
        2 => [level * 0.7, level, level * 0.75],
        _ => [level * 0.8, level * 0.7, level],
    }
}

struct Gpu {
    renderer: VulkanRenderer,
    gbm: GbmDevice<Arc<File>>,
}

fn gpu() -> Option<Gpu> {
    std::fs::read_dir("/dev/dri")
        .ok()?
        .flatten()
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
                gbm: GbmDevice::new(Arc::new(file)).ok()?,
            })
        })
}

impl Gpu {
    fn buffer(&self, modifier: Modifier) -> Option<(GbmBuffer, Dmabuf)> {
        let buffer = GbmAllocator::new(self.gbm.clone(), GbmBufferFlags::RENDERING)
            .create_buffer(
                SIZE.0 as u32,
                SIZE.1 as u32,
                Fourcc::Xrgb2101010,
                &[modifier],
            )
            .ok()?;
        let dmabuf = buffer.export().ok()?;
        Some((buffer, dmabuf))
    }

    fn codes(&self, dmabuf: &Dmabuf) -> Outcome<Vec<[f64; 3]>> {
        let mut fds = [None; 4];
        let (mut strides, mut offsets) = ([0; 4], [0; 4]);
        let planes = dmabuf.handles().zip(dmabuf.offsets()).zip(dmabuf.strides());
        for (index, ((fd, offset), stride)) in planes.enumerate() {
            fds[index] = Some(fd);
            offsets[index] = i32::try_from(offset)?;
            strides[index] = i32::try_from(stride)?;
        }
        let imported = self
            .gbm
            .import_buffer_object_from_dma_buf_with_modifiers::<()>(
                dmabuf.num_planes() as u32,
                fds,
                SIZE.0 as u32,
                SIZE.1 as u32,
                dmabuf.format().code,
                GbmBufferFlags::RENDERING,
                strides,
                offsets,
                dmabuf.format().modifier,
            )?;
        Ok(imported.map(0, 0, SIZE.0 as u32, SIZE.1 as u32, |mapped| {
            mapped
                .buffer()
                .chunks(mapped.stride() as usize)
                .take(SIZE.1 as usize)
                .flat_map(|row| row[..SIZE.0 as usize * 4].as_chunks::<4>().0)
                .map(|texel| u32::from_le_bytes(*texel))
                .map(|texel| [20, 10, 0].map(|shift| f64::from(texel >> shift & 0x3ff)))
                .collect()
        })?)
    }
}

fn fill<F: Frame<Error = bande_render::RenderError>>(
    frame: &mut F,
    colour: impl Fn(i32) -> [f64; 3],
) -> Outcome {
    for index in 0..COLUMNS * 4 {
        let [red, green, blue] = colour(index).map(|value| value as f32);
        frame.clear(Color32F::new(red, green, blue, 1.0), &[cell(index)])?;
    }
    Ok(())
}

fn centres() -> impl Iterator<Item = (i32, usize)> {
    (0..COLUMNS * 4).map(|index| {
        let at = cell(index).loc;
        let (x, y) = (at.x + CELL / 2, at.y + CELL / 2);
        (index, (y * SIZE.0 + x) as usize)
    })
}

#[test]
fn ordinary_content_keeps_its_white_and_hdr_content_its_light_up_to_the_display_peak() -> Outcome {
    let Some(mut gpu) = gpu() else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return Ok(());
    };
    let Some(modifier) = gpu
        .renderer
        .render_formats()
        .iter()
        .find(|format| format.code == Fourcc::Xrgb2101010)
        .map(|format| format.modifier)
    else {
        println!("skipped: the GPU cannot render 10 bits per channel");
        return Ok(());
    };
    let whole: Rectangle<i32, Physical> = Rectangle::from_size(SIZE.into());
    let colour = |extended| OutputColour {
        matrix: TO_2020.map(|row| row.map(|value| value as f32)),
        encode: Encode::Pq {
            white: WHITE as f32,
            peak: PEAK as f32,
        },
        extended,
    };
    let no_buffer = "GBM cannot allocate a 10-bit buffer";
    let (_kept, mut source) = gpu.buffer(modifier).ok_or(no_buffer)?;
    {
        let mut framebuffer = gpu.renderer.bind(&mut source)?;
        let mut frame = gpu
            .renderer
            .render(&mut framebuffer, SIZE.into(), Transform::Normal)?;
        fill(&mut frame, content)?;
        frame.finish()?.wait()?;
    }
    let source_codes = gpu.codes(&source)?;
    let hdr = HdrContent {
        max_nits: MASTERED as f32,
    };
    let (_shown, mut display) = gpu.buffer(modifier).ok_or(no_buffer)?;

    let grey = |index: i32| [f64::from(index * 4) / 255.0, f64::from(index) / 64.0, 0.5];
    for extended in [false, true] {
        gpu.renderer
            .converting(Some(&colour(extended)), |renderer| -> Outcome {
                let mut framebuffer = renderer.bind(&mut display)?;
                let mut frame =
                    renderer.render(&mut framebuffer, SIZE.into(), Transform::Normal)?;
                fill(&mut frame, grey)?;
                frame.finish()?.wait()?;
                Ok(())
            })??;
        let codes = gpu.codes(&display)?;
        for (index, at) in centres() {
            let quantised = grey(index).map(|value| match extended {
                true => value,
                false => (value * 1023.0).round() / 1023.0,
            });
            let wanted = through(TO_2020, quantised.map(decoded))
                .map(|value| pq(value.max(0.0) * WHITE) * 1023.0);
            for (found, wanted) in codes[at].iter().zip(wanted) {
                assert!(
                    (found - wanted).abs() <= 3.0,
                    "cell {index} shows {:?}, not {wanted:.1} (extended {extended})",
                    codes[at]
                );
            }
        }
    }

    let texture = gpu
        .renderer
        .import_dmabuf(&source, None)?
        .showing(Some(hdr));
    let place = Rectangle::from_size((f64::from(SIZE.0), f64::from(SIZE.1)).into());
    gpu.renderer
        .converting(Some(&colour(true)), |renderer| -> Outcome {
            let mut framebuffer = renderer.bind(&mut display)?;
            let mut frame = renderer.render(&mut framebuffer, SIZE.into(), Transform::Normal)?;
            frame.render_texture_from_to(
                &texture,
                place,
                whole,
                &[whole],
                &[],
                Transform::Normal,
                1.0,
            )?;
            frame.finish()?.wait()?;
            Ok(())
        })??;
    let codes = gpu.codes(&display)?;
    for (index, at) in centres() {
        let wanted = toned(source_codes[at].map(|code| code / 1023.0), PEAK);
        for (found, wanted) in codes[at].iter().zip(wanted) {
            let wanted = pq(wanted) * 1023.0;
            assert!(
                (found - wanted).abs() <= 4.0,
                "HDR cell {index} shows {:?}, not {wanted:.1}",
                codes[at]
            );
        }
    }

    let mut capture: OffscreenTarget = gpu.renderer.create_buffer(Fourcc::Argb8888, SIZE.into())?;
    let mut framebuffer = gpu.renderer.bind(&mut capture)?;
    let mut frame = gpu
        .renderer
        .render(&mut framebuffer, SIZE.into(), Transform::Normal)?;
    frame.render_texture_from_to(
        &texture,
        place,
        whole,
        &[whole],
        &[],
        Transform::Normal,
        1.0,
    )?;
    frame.finish()?.wait()?;
    let pixels = gpu.renderer.read_target(&capture)?;
    for (index, at) in centres() {
        let light = toned(source_codes[at].map(|code| code / 1023.0), WHITE);
        let wanted = through(FROM_2020, light.map(|value| value / WHITE))
            .map(|value| encoded(value.clamp(0.0, 1.0)) * 255.0);
        let found = [2, 1, 0].map(|channel| f64::from(pixels[at * 4 + channel]));
        for (found, wanted) in found.into_iter().zip(wanted) {
            assert!(
                (found - wanted).abs() <= 3.0,
                "a capture of HDR cell {index} shows {found}, not {wanted:.1}"
            );
        }
    }
    assert_eq!(vulkan_errors(), 0, "Vulkan reported errors");
    Ok(())
}
