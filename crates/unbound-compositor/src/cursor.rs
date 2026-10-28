use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::Transform;

pub const ARROW_WIDTH: i32 = 11;
pub const ARROW_HEIGHT: i32 = 17;
pub const ARROW_FILL: [u8; 4] = [0x00, 0x00, 0x00, 0xff];
pub const ARROW_OUTLINE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];

fn inside(x: i32, y: i32) -> bool {
    (0..ARROW_WIDTH).contains(&x) && (0..ARROW_HEIGHT).contains(&y) && x * 3 <= y * 2
}

fn on_edge(x: i32, y: i32) -> bool {
    [(-1, 0), (1, 0), (0, -1), (0, 1)]
        .iter()
        .any(|(dx, dy)| !inside(x + dx, y + dy))
}

pub(crate) fn arrow() -> MemoryRenderBuffer {
    let pixels: Vec<u8> = (0..ARROW_HEIGHT)
        .flat_map(|y| (0..ARROW_WIDTH).map(move |x| (x, y)))
        .flat_map(|(x, y)| match (inside(x, y), on_edge(x, y)) {
            (false, _) => [0; 4],
            (true, true) => ARROW_OUTLINE,
            (true, false) => ARROW_FILL,
        })
        .collect();
    MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Argb8888,
        (ARROW_WIDTH, ARROW_HEIGHT),
        1,
        Transform::Normal,
        None,
    )
}
