use std::error::Error;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;

use bande_render::{VulkanRenderer, vulkan_errors};
use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBuffer, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Buffer, Format, Fourcc, Modifier};
use smithay::backend::renderer::{Bind, Color32F, Frame, ImportDma, Renderer};
use smithay::utils::{Physical, Rectangle, Transform};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

const STEPS: i32 = 1024;
const ROWS: i32 = 32;
const SIZE: (i32, i32) = (STEPS, ROWS);

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
    fn layouts(&self, code: Fourcc) -> Vec<Modifier> {
        let formats = self.renderer.render_formats();
        let mut layouts: Vec<Modifier> = formats
            .iter()
            .filter(|format| format.code == code)
            .map(|format| format.modifier)
            .collect();
        layouts.sort_by_key(|layout| u64::from(*layout));
        layouts
    }

    fn buffer(&self, code: Fourcc, modifier: Modifier) -> Option<(GbmBuffer, Dmabuf)> {
        let mut allocator = GbmAllocator::new(self.gbm.clone(), GbmBufferFlags::RENDERING);
        let buffer = allocator
            .create_buffer(STEPS as u32, ROWS as u32, code, &[modifier])
            .ok()
            .filter(|buffer| buffer.format() == Format { code, modifier })?;
        let dmabuf = buffer.export().ok()?;
        Some((buffer, dmabuf))
    }

    fn draw_ramp(&mut self, dmabuf: &mut Dmabuf) -> Outcome {
        let mut framebuffer = self.renderer.bind(dmabuf)?;
        let mut frame = self
            .renderer
            .render(&mut framebuffer, SIZE.into(), Transform::Normal)?;
        for step in 0..STEPS {
            let value = step as f32 / (STEPS - 1) as f32;
            let column = Rectangle::new((step, 0).into(), (1, ROWS).into());
            frame.clear(Color32F::new(value, 1.0 - value, value, 1.0), &[column])?;
        }
        frame.finish()?.wait()?;
        Ok(())
    }

    fn show(&mut self, source: &Dmabuf, target: &mut Dmabuf) -> Outcome {
        let whole: Rectangle<i32, Physical> = Rectangle::from_size(SIZE.into());
        let texture = self.renderer.import_dmabuf(source, None)?;
        let mut framebuffer = self.renderer.bind(target)?;
        let mut frame = self
            .renderer
            .render(&mut framebuffer, SIZE.into(), Transform::Normal)?;
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
        Ok(())
    }

    fn texels(&self, dmabuf: &Dmabuf) -> Outcome<Vec<u32>> {
        let mut fds = [None; 4];
        let (mut strides, mut offsets) = ([0; 4], [0; 4]);
        for (index, ((fd, offset), stride)) in dmabuf
            .handles()
            .zip(dmabuf.offsets())
            .zip(dmabuf.strides())
            .enumerate()
        {
            fds[index] = Some(fd);
            offsets[index] = i32::try_from(offset)?;
            strides[index] = i32::try_from(stride)?;
        }
        let (width, height) = (STEPS as u32, ROWS as u32);
        let imported = self
            .gbm
            .import_buffer_object_from_dma_buf_with_modifiers::<()>(
                dmabuf.num_planes() as u32,
                fds,
                width,
                height,
                dmabuf.format().code,
                GbmBufferFlags::RENDERING,
                strides,
                offsets,
                dmabuf.format().modifier,
            )?;
        let texels = imported.map(0, 0, width, height, |mapped| {
            mapped
                .buffer()
                .chunks(mapped.stride() as usize)
                .take(height as usize)
                .flat_map(|row| row[..width as usize * 4].as_chunks::<4>().0)
                .map(|texel| u32::from_le_bytes(*texel))
                .collect()
        })?;
        Ok(texels)
    }
}

fn channels(code: Fourcc, texel: u32) -> [i32; 3] {
    let ten = |shift: u32| (texel >> shift & 0x3ff) as i32;
    match code {
        Fourcc::Xrgb2101010 | Fourcc::Argb2101010 => [ten(20), ten(10), ten(0)],
        Fourcc::Xbgr2101010 | Fourcc::Abgr2101010 => [ten(0), ten(10), ten(20)],
        _ => [16, 8, 0].map(|shift| (texel >> shift & 0xff) as i32),
    }
}

#[test]
fn deep_colour_buffers_keep_every_step_and_reach_eight_bits_dithered() -> Outcome {
    let Some(mut gpu) = gpu() else {
        println!("skipped: no hardware Vulkan device with a DRM render node");
        return Ok(());
    };
    let mut ramps = Vec::new();
    for code in [
        Fourcc::Xrgb2101010,
        Fourcc::Xbgr2101010,
        Fourcc::Argb2101010,
        Fourcc::Abgr2101010,
    ] {
        for layout in gpu.layouts(code) {
            let Some((buffer, mut ramp)) = gpu.buffer(code, layout) else {
                println!("skipped {code:?} {layout:?}: GBM cannot allocate it");
                continue;
            };
            let Some((_copy, mut copy)) = gpu.buffer(code, layout) else {
                continue;
            };
            gpu.draw_ramp(&mut ramp)?;
            gpu.show(&ramp, &mut copy)?;
            for (stage, dmabuf) in [("drawn", &ramp), ("shown", &copy)] {
                let texels = gpu.texels(dmabuf)?;
                for (index, texel) in texels.into_iter().enumerate() {
                    let step = index as i32 % STEPS;
                    assert_eq!(
                        channels(code, texel),
                        [step, STEPS - 1 - step, step],
                        "step {step} {stage} in {code:?} {layout:?}"
                    );
                }
            }
            println!("every step of {code:?} {layout:?} is exact");
            ramps.push((buffer, ramp));
        }
    }
    let Some((_, ramp)) = ramps.first() else {
        println!("skipped: the GPU cannot render 10 bits per channel");
        return Ok(());
    };
    for layout in gpu.layouts(Fourcc::Xrgb8888) {
        let Some((_shown, mut shown)) = gpu.buffer(Fourcc::Xrgb8888, layout) else {
            continue;
        };
        gpu.show(ramp, &mut shown)?;
        let texels = gpu.texels(&shown)?;
        let eight = |step: i32| f64::from(step) * 255.0 / f64::from(STEPS - 1);
        let mut furthest: f64 = 0.0;
        let mut errors = Vec::new();
        for step in 0..STEPS {
            let wanted = eight(step);
            let found: Vec<f64> = (0..ROWS)
                .map(|row| texels[(row * STEPS + step) as usize])
                .map(|texel| f64::from(channels(Fourcc::Xrgb8888, texel)[0]))
                .collect();
            for value in &found {
                furthest = furthest.max((value - wanted).abs());
            }
            errors.push(found.iter().sum::<f64>() / f64::from(ROWS) - wanted);
        }
        let bands: Vec<f64> = errors
            .chunks(4)
            .map(|steps| steps.iter().sum::<f64>().abs() / 4.0)
            .collect();
        let worst = bands.iter().copied().fold(0.0, f64::max);
        println!("{layout:?}: no pixel off by {furthest:.2}, no four steps by {worst:.3}");
        assert!(furthest <= 1.05, "a pixel is {furthest} off in {layout:?}");
        assert!(worst < 0.2, "four steps average {worst} off in {layout:?}");
        let (Some((_source, mut plain)), Some((_copied, mut copied))) = (
            gpu.buffer(Fourcc::Xrgb8888, layout),
            gpu.buffer(Fourcc::Xrgb8888, layout),
        ) else {
            continue;
        };
        gpu.draw_ramp(&mut plain)?;
        gpu.show(&plain, &mut copied)?;
        let colours = |dmabuf: &Dmabuf| -> Outcome<Vec<u32>> {
            Ok(gpu.texels(dmabuf)?.iter().map(|texel| texel << 8).collect())
        };
        assert!(
            colours(&plain)? == colours(&copied)?,
            "8 bits are not passed through exactly in {layout:?}"
        );
    }
    assert_eq!(vulkan_errors(), 0, "Vulkan reported errors");
    Ok(())
}
