#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use bande_render::VulkanRenderer;
use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::backend::renderer::{Bind, Color32F, Frame, Renderer};
use smithay::utils::{Physical, Rectangle, Transform};

const SIZE: (i32, i32) = (256, 128);
const UDMABUF_CREATE: u64 = 0x4018_7542;

#[repr(C)]
struct Create {
    memfd: u32,
    flags: u32,
    offset: u64,
    size: u64,
}

fn shared_buffer() -> Option<(File, Dmabuf)> {
    let length = (SIZE.0 * SIZE.1 * 4) as u64;
    let memory = unsafe { libc::memfd_create(c"display-buffer".as_ptr(), libc::MFD_ALLOW_SEALING) };
    assert!(memory >= 0);
    let memory = unsafe { File::from_raw_fd(memory) };
    memory.set_len(length.next_multiple_of(65536)).unwrap();
    assert_eq!(
        unsafe { libc::fcntl(memory.as_raw_fd(), libc::F_ADD_SEALS, libc::F_SEAL_SHRINK) },
        0
    );
    let device = File::options()
        .read(true)
        .write(true)
        .open("/dev/udmabuf")
        .ok()?;
    let request = Create {
        memfd: memory.as_raw_fd() as u32,
        flags: 1,
        offset: 0,
        size: length.next_multiple_of(65536),
    };
    let fd = unsafe { libc::ioctl(device.as_raw_fd(), UDMABUF_CREATE as _, &request) };
    assert!(fd >= 0);
    let mut builder = Dmabuf::builder(
        SIZE,
        Fourcc::Xrgb8888,
        Modifier::Linear,
        DmabufFlags::empty(),
    );
    builder.add_plane(
        unsafe { OwnedFd::from_raw_fd(fd) },
        0,
        0,
        (SIZE.0 * 4) as u32,
    );
    Some((memory, builder.build().unwrap()))
}

fn pixel(memory: &mut File, x: i32, y: i32) -> [u8; 3] {
    let mut bytes = Vec::new();
    std::io::Seek::rewind(memory).unwrap();
    memory.read_to_end(&mut bytes).unwrap();
    let at = ((y * SIZE.0 + x) * 4) as usize;
    [bytes[at + 2], bytes[at + 1], bytes[at]]
}

#[test]
fn a_frame_reaches_the_display_buffer() {
    let Some((mut memory, mut dmabuf)) = shared_buffer() else {
        return;
    };
    let mut renderer = VulkanRenderer::new().expect("Vulkan renderer");
    let whole = [Rectangle::<i32, Physical>::from_size(SIZE.into())];
    for (colour, expected) in [
        (Color32F::new(1.0, 0.0, 0.0, 1.0), [255, 0, 0]),
        (Color32F::new(0.0, 0.0, 1.0, 1.0), [0, 0, 255]),
    ] {
        let mut framebuffer = renderer.bind(&mut dmabuf).unwrap();
        let mut frame = renderer
            .render(&mut framebuffer, SIZE.into(), Transform::Normal)
            .unwrap();
        frame.clear(colour, &whole).unwrap();
        drop(frame.finish().unwrap());
        drop(framebuffer);
        assert_eq!(
            pixel(&mut memory, 100, 50),
            expected,
            "on {}",
            renderer.device_info().name
        );
    }
}

#[test]
fn a_converted_frame_reaches_the_display_buffer() {
    let Some((mut memory, mut dmabuf)) = shared_buffer() else {
        return;
    };
    let mut renderer = VulkanRenderer::new().expect("Vulkan renderer");
    renderer.set_output_colour(Some([[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]));
    let whole = [Rectangle::<i32, Physical>::from_size(SIZE.into())];
    let mut framebuffer = renderer.bind(&mut dmabuf).unwrap();
    let mut frame = renderer
        .render(&mut framebuffer, SIZE.into(), Transform::Normal)
        .unwrap();
    frame.clear(Color32F::new(1.0, 0.0, 0.0, 1.0), &whole).unwrap();
    drop(frame.finish().unwrap());
    drop(framebuffer);
    let converts = !renderer.device_info().software;
    let expected = if converts { [0, 255, 0] } else { [255, 0, 0] };
    assert_eq!(pixel(&mut memory, 100, 50), expected);
}
