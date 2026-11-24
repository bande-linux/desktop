use std::error::Error;
use std::os::fd::AsFd;

use bande_graphics::{
    Color, GpuDevice, GpuRenderer, Item, Paint, Rect, Region, Scene, Shape, SharedImage,
};
use bande_render::{OffscreenTarget, VulkanRenderer};
use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::backend::renderer::{Bind, Frame, ImportDma, Offscreen, Renderer};
use smithay::utils::{Physical, Rectangle, Size, Transform};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

const CELL: u32 = 16;
const SIZES: [(u32, u32); 3] = [(64, 48), (301, 203), (1400, 900)];
const FRAMES: u32 = 4;
const CHANGED: Region = Region {
    x: CELL,
    y: CELL,
    width: 2 * CELL,
    height: CELL,
};

fn colour((x, y): (u32, u32), turn: u32) -> u32 {
    let (column, row) = (x / CELL, y / CELL);
    let changed = (CHANGED.x..CHANGED.x + CHANGED.width).contains(&x)
        && (CHANGED.y..CHANGED.y + CHANGED.height).contains(&y);
    let turn = if changed { turn } else { 0 };
    let red = (column * 37 + turn * 91) % 256;
    let green = (row * 53 + turn * 17) % 256;
    let blue = ((column + row) * 29 + turn * 71) % 256;
    red << 16 | green << 8 | blue
}

fn pattern((width, height): (u32, u32), turn: u32) -> Scene {
    let cells = (0..height.div_ceil(CELL))
        .flat_map(|row| (0..width.div_ceil(CELL)).map(move |column| (column * CELL, row * CELL)));
    let items = cells
        .map(|(x, y)| Item::Fill {
            shape: Shape::rect(Rect::new(x as f32, y as f32, CELL as f32, CELL as f32)),
            paint: Paint::Solid(Color::hex(colour((x, y), turn))),
        })
        .collect();
    Scene {
        width,
        height,
        background: Color::hex(0),
        items,
    }
}

fn exported(image: &SharedImage, (width, height): (u32, u32)) -> Outcome<Dmabuf> {
    let mut builder = Dmabuf::builder(
        (width as i32, height as i32),
        Fourcc::Argb8888,
        Modifier::from(image.modifier),
        DmabufFlags::empty(),
    );
    let fd = image.fd.as_fd().try_clone_to_owned()?;
    builder.add_plane(fd, 0, image.offset, image.stride);
    Ok(builder.build().ok_or("no dma-buf")?)
}

fn composed(
    compositor: &mut VulkanRenderer,
    dmabuf: &Dmabuf,
    (width, height): (u32, u32),
) -> Outcome<Vec<u8>> {
    let size: Size<i32, Physical> = (width as i32, height as i32).into();
    let whole = Rectangle::from_size(size);
    let mut target: OffscreenTarget =
        compositor.create_buffer(Fourcc::Argb8888, (size.w, size.h).into())?;
    let texture = compositor.import_dmabuf(dmabuf, None)?;
    let mut framebuffer = compositor.bind(&mut target)?;
    let mut frame = compositor.render(&mut framebuffer, size, Transform::Normal)?;
    frame.render_texture_from_to(
        &texture,
        Rectangle::from_size((f64::from(size.w), f64::from(size.h)).into()),
        whole,
        &[whole],
        &[],
        Transform::Normal,
        1.0,
    )?;
    frame.finish()?.wait()?;
    drop(framebuffer);
    Ok(compositor.read_target(&target)?)
}

fn wrong_pixels(found: &[u8], (width, height): (u32, u32), turn: u32) -> usize {
    (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .zip(found.chunks_exact(4))
        .filter(|(at, pixel)| pixel[..3] != colour(*at, turn).to_le_bytes()[..3])
        .count()
}

#[test]
fn the_compositor_reads_every_pixel_of_a_toolkit_frame() -> Outcome {
    let Ok(toolkit) = GpuDevice::shared(false, None) else {
        println!("skipped: no single hardware Vulkan device");
        return Ok(());
    };
    let mut compositor = VulkanRenderer::new(None)?;
    let modifiers: Vec<u64> = compositor
        .dmabuf_formats()
        .iter()
        .filter(|format| format.code == Fourcc::Argb8888)
        .map(|format| format.modifier.into())
        .filter(|modifier| toolkit.shares_images(&[vec![*modifier]]))
        .collect();
    assert!(!modifiers.is_empty(), "no layout to check");
    for modifier in modifiers {
        for size in SIZES {
            let mut renderer = GpuRenderer::with_device(toolkit.clone(), (256, 256));
            let image = toolkit.shared_image(size, &[vec![modifier]])?;
            let dmabuf = exported(&image, size)?;
            for turn in 0..FRAMES {
                let damage = (turn > 0).then_some([CHANGED]);
                let scene = pattern(size, turn);
                renderer.present(&scene, 1.0, &image, damage.as_ref().map(<[_; 1]>::as_slice))?;
                let found = composed(&mut compositor, &dmabuf, size)?;
                assert_eq!(
                    wrong_pixels(&found, size, turn),
                    0,
                    "wrong pixels in frame {turn} of {size:?} in layout {modifier:#x}"
                );
            }
        }
    }
    Ok(())
}
