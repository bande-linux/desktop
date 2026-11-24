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
const SIZE: (i32, i32) = (CELL * 16, CELL);
const WHITE: f32 = 203.0;
const PEAK: f32 = 600.0;
const MASTERED: f32 = 1000.0;
const TO_2020: [[f32; 3]; 3] = [
    [0.627_403_9, 0.329_283_04, 0.043_313_066],
    [0.069_097_29, 0.919_540_4, 0.011_362_316],
    [0.016_391_44, 0.088_013_31, 0.895_595_25],
];

/// sRGB colours and the 10-bit PQ BT.2020 codes of their light with white at 203 cd/m². All tables
/// were computed at 40 digits from the constants of SMPTE ST 2084 and BT.2390 and matrices derived
/// from the primaries, apart from the renderer.
const ORDINARY: [([u8; 3], [f64; 3]); 10] = [
    ([0, 0, 0], [0.0, 0.0, 0.0]),
    ([255, 255, 255], [594.0, 594.0, 594.0]),
    ([128, 128, 128], [437.7, 437.7, 437.7]),
    ([255, 0, 0], [544.8, 334.5, 225.1]),
    ([0, 255, 0], [479.0, 585.1, 355.3]),
    ([0, 0, 255], [296.3, 201.3, 582.3]),
    ([12, 34, 200], [274.3, 243.9, 525.1]),
    ([200, 180, 16], [523.9, 512.7, 314.5]),
    ([1, 2, 3], [60.2, 68.8, 80.8]),
    ([250, 128, 114], [552.9, 458.1, 423.1]),
];

/// PQ BT.2020 codes of content mastered at 1000 cd/m², the codes a 600 cd/m² display gets and the
/// sRGB values of a capture.
const BRIGHT: [([u16; 3], [f64; 3], [f64; 3]); 11] = [
    ([0, 0, 0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
    (
        [594, 594, 594],
        [594.0, 594.0, 594.0],
        [229.0, 229.0, 229.0],
    ),
    (
        [520, 520, 520],
        [520.0, 520.0, 520.0],
        [185.9, 185.9, 185.9],
    ),
    (
        [668, 668, 668],
        [668.0, 668.0, 668.0],
        [249.8, 249.8, 249.8],
    ),
    (
        [713, 713, 713],
        [704.2, 704.2, 704.2],
        [254.1, 254.1, 254.1],
    ),
    (
        [740, 740, 740],
        [711.2, 711.2, 711.2],
        [254.9, 254.9, 254.9],
    ),
    (
        [769, 769, 769],
        [712.3, 712.3, 712.3],
        [255.0, 255.0, 255.0],
    ),
    ([700, 400, 500], [697.2, 397.6, 497.4], [255.0, 0.0, 106.6]),
    ([300, 650, 450], [300.0, 650.0, 450.0], [0.0, 255.0, 68.0]),
    ([450, 420, 760], [409.1, 380.4, 712.3], [0.0, 48.1, 255.0]),
    ([769, 600, 300], [712.3, 546.0, 260.9], [255.0, 96.7, 0.0]),
];

fn cell(index: usize) -> Rectangle<i32, Physical> {
    Rectangle::new((index as i32 * CELL, 0).into(), (CELL, CELL).into())
}

fn centre(index: usize) -> usize {
    let (x, y) = (index as i32 * CELL + CELL / 2, CELL / 2);
    (y * SIZE.0 + x) as usize
}

fn close(what: &str, found: [f64; 3], wanted: [f64; 3], slack: f64) {
    assert!(
        found
            .iter()
            .zip(wanted)
            .all(|(found, wanted)| (found - wanted).abs() <= slack),
        "{what} shows {found:?}, not {wanted:?}"
    );
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
    colours: impl Iterator<Item = [f32; 3]>,
) -> Outcome {
    for (index, [red, green, blue]) in colours.enumerate() {
        frame.clear(Color32F::new(red, green, blue, 1.0), &[cell(index)])?;
    }
    Ok(())
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
        matrix: TO_2020,
        encode: Encode::Pq {
            white: WHITE,
            peak: PEAK,
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
        let codes = BRIGHT.map(|(codes, ..)| codes.map(|code| f32::from(code) / 1023.0));
        fill(&mut frame, codes.into_iter())?;
        frame.finish()?.wait()?;
    }
    let drawn = gpu.codes(&source)?;
    for (index, (codes, ..)) in BRIGHT.iter().enumerate() {
        close(
            "the content",
            drawn[centre(index)],
            codes.map(f64::from),
            0.0,
        );
    }
    let (_shown, mut display) = gpu.buffer(modifier).ok_or(no_buffer)?;
    for extended in [false, true] {
        gpu.renderer
            .converting(Some(&colour(extended)), |renderer| -> Outcome {
                let mut framebuffer = renderer.bind(&mut display)?;
                let mut frame =
                    renderer.render(&mut framebuffer, SIZE.into(), Transform::Normal)?;
                let colours = ORDINARY.map(|(rgb, _)| rgb.map(|value| f32::from(value) / 255.0));
                fill(&mut frame, colours.into_iter())?;
                frame.finish()?.wait()?;
                Ok(())
            })??;
        let codes = gpu.codes(&display)?;
        for (index, (rgb, wanted)) in ORDINARY.into_iter().enumerate() {
            let what = format!("{rgb:?} (extended {extended})");
            close(&what, codes[centre(index)], wanted, 2.5);
        }
    }

    let texture = gpu
        .renderer
        .import_dmabuf(&source, None)?
        .showing(Some(HdrContent { max_nits: MASTERED }));
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
    for (index, (content, wanted, _)) in BRIGHT.into_iter().enumerate() {
        let what = format!("HDR {content:?}");
        close(&what, codes[centre(index)], wanted, 2.5);
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
    for (index, (content, _, wanted)) in BRIGHT.into_iter().enumerate() {
        let found = [2, 1, 0].map(|channel| f64::from(pixels[centre(index) * 4 + channel]));
        close(&format!("a capture of {content:?}"), found, wanted, 2.0);
    }
    assert_eq!(vulkan_errors(), 0, "Vulkan reported errors");
    Ok(())
}
