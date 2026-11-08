use std::collections::HashSet;
use std::env;
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::Path;

use bande_config::{CURSOR_THEME, DEFAULT_CURSOR_SIZE};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::{Physical, Point, Transform};

use crate::cursor_set::{Image, KINDS, LEGACY_NAMES, Pointer};

const ARROW_WIDTH: i32 = 11;
const ARROW_HEIGHT: i32 = 17;
const ARROW_FILL: [u8; 4] = [0x00, 0x00, 0x00, 0xff];
const ARROW_OUTLINE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
const XCURSOR_SIZES: [u32; 6] = [24, 32, 48, 64, 96, 128];
const XCURSOR_HEADER: u32 = 16;
const XCURSOR_VERSION: u32 = 0x0001_0000;
const XCURSOR_IMAGE: u32 = 0xfffd_0002;
const XCURSOR_IMAGE_HEADER: u32 = 36;
const XCURSOR_IMAGE_VERSION: u32 = 1;
const XCURSOR_TOC_ENTRY: u32 = 12;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorTheme {
    pub size: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct Cursor {
    pub(crate) buffer: MemoryRenderBuffer,
    pub(crate) hotspot: Point<i32, Physical>,
}

impl CursorTheme {
    pub fn from_environment() -> Self {
        Self {
            size: env::var("XCURSOR_SIZE")
                .ok()
                .and_then(|size| size.parse().ok())
                .filter(|size| *size > 0)
                .unwrap_or(DEFAULT_CURSOR_SIZE),
        }
    }

    pub(crate) fn builtin() -> Self {
        Self {
            size: DEFAULT_CURSOR_SIZE,
        }
    }

    pub fn name(&self) -> &'static str {
        CURSOR_THEME
    }

    pub(crate) fn draw(&self, pointer: Pointer, scale: f64) -> Option<Cursor> {
        let side = (f64::from(self.size) * scale).round().max(1.0) as u32;
        match pointer.render(side) {
            Ok(image) => Some(Cursor {
                buffer: MemoryRenderBuffer::from_slice(
                    &image.rgba,
                    Fourcc::Abgr8888,
                    (image.side as i32, image.side as i32),
                    1,
                    Transform::Normal,
                    None,
                ),
                hotspot: (image.hotspot.0 as i32, image.hotspot.1 as i32).into(),
            }),
            Err(error) => {
                tracing::warn!(
                    "cannot draw the {} pointer at {side} px, showing the built-in arrow: {error}",
                    pointer.name()
                );
                None
            }
        }
    }
}

pub fn write_cursor_theme(directory: &Path) -> io::Result<()> {
    let cursors = directory.join("cursors");
    fs::create_dir_all(&cursors)?;
    fs::write(
        directory.join("index.theme"),
        format!("[Icon Theme]\nName={CURSOR_THEME}\nComment=The pointers of the Bande desktop\n"),
    )?;
    let mut written = HashSet::new();
    for icon in KINDS {
        let images = XCURSOR_SIZES
            .iter()
            .map(|size| {
                Pointer::Kind(icon)
                    .render(*size)
                    .map(|image| (*size, image))
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(io::Error::other)?;
        fs::write(cursors.join(icon.name()), xcursor(&images))?;
        written.insert(icon.name());
    }
    let aliases = LEGACY_NAMES.into_iter().chain(
        KINDS
            .iter()
            .flat_map(|icon| icon.alt_names().iter().map(move |alias| (*alias, *icon))),
    );
    for (alias, icon) in aliases {
        if written.insert(alias) {
            symlink(icon.name(), cursors.join(alias))?;
        }
    }
    Ok(())
}

fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn xcursor(images: &[(u32, Image)]) -> Vec<u8> {
    let mut bytes = b"Xcur".to_vec();
    word(&mut bytes, XCURSOR_HEADER);
    word(&mut bytes, XCURSOR_VERSION);
    word(&mut bytes, images.len() as u32);
    let mut position = XCURSOR_HEADER + XCURSOR_TOC_ENTRY * images.len() as u32;
    for (nominal, image) in images {
        word(&mut bytes, XCURSOR_IMAGE);
        word(&mut bytes, *nominal);
        word(&mut bytes, position);
        position += XCURSOR_IMAGE_HEADER + 4 * image.side * image.side;
    }
    for (nominal, image) in images {
        for value in [
            XCURSOR_IMAGE_HEADER,
            XCURSOR_IMAGE,
            *nominal,
            XCURSOR_IMAGE_VERSION,
            image.side,
            image.side,
            image.hotspot.0,
            image.hotspot.1,
            0,
        ] {
            word(&mut bytes, value);
        }
        bytes.extend(
            image
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|[red, green, blue, alpha]| [*blue, *green, *red, *alpha]),
        );
    }
    bytes
}
