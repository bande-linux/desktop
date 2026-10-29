use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::input::pointer::CursorIcon;
use smithay::utils::{Physical, Point, Transform};

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

const XCURSOR_MAGIC: &[u8; 4] = b"Xcur";
const XCURSOR_IMAGE: u32 = 0xfffd_0002;
const XCURSOR_IMAGE_HEADER: usize = 36;
const XCURSOR_MAX_SIDE: u32 = 0x7fff;
const THEME_INHERITANCE_DEPTH: usize = 16;
pub const DEFAULT_CURSOR_THEME: &str = "default";
pub const DEFAULT_CURSOR_SIZE: u32 = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorTheme {
    pub search_path: Vec<PathBuf>,
    pub name: String,
    pub size: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct Cursor {
    pub(crate) buffer: MemoryRenderBuffer,
    pub(crate) hotspot: Point<i32, Physical>,
}

struct XcursorImage {
    nominal: u32,
    width: u32,
    height: u32,
    hotspot: (u32, u32),
    pixels: Vec<u8>,
}

impl CursorTheme {
    pub fn from_environment() -> Self {
        let var = |name: &str| env::var_os(name).filter(|value| !value.is_empty());
        let search_path = match var("XCURSOR_PATH") {
            Some(path) => env::split_paths(&path).collect(),
            None => default_search_path(
                var("HOME").map(PathBuf::from),
                var("XDG_DATA_HOME").map(PathBuf::from),
                var("XDG_DATA_DIRS"),
            ),
        };
        Self {
            search_path,
            name: var("XCURSOR_THEME")
                .and_then(|name| name.into_string().ok())
                .unwrap_or_else(|| DEFAULT_CURSOR_THEME.to_owned()),
            size: var("XCURSOR_SIZE")
                .and_then(|size| size.to_str()?.parse().ok())
                .filter(|size| *size > 0)
                .unwrap_or(DEFAULT_CURSOR_SIZE),
        }
    }

    pub(crate) fn builtin() -> Self {
        Self {
            search_path: Vec::new(),
            name: DEFAULT_CURSOR_THEME.to_owned(),
            size: DEFAULT_CURSOR_SIZE,
        }
    }

    pub(crate) fn load(&self, icon: CursorIcon, scale: f64) -> Option<Cursor> {
        let pixel_size = (f64::from(self.size) * scale).round() as u32;
        std::iter::once(icon.name())
            .chain(icon.alt_names().iter().copied())
            .find_map(|name| self.find(name))
            .and_then(|file| fs::read(file).ok())
            .and_then(|bytes| best_image(parse_xcursor(&bytes), pixel_size))
            .map(|image| Cursor {
                buffer: MemoryRenderBuffer::from_slice(
                    &image.pixels,
                    Fourcc::Argb8888,
                    (image.width as i32, image.height as i32),
                    1,
                    Transform::Normal,
                    None,
                ),
                hotspot: (image.hotspot.0 as i32, image.hotspot.1 as i32).into(),
            })
    }

    fn find(&self, cursor: &str) -> Option<PathBuf> {
        let mut pending = vec![self.name.clone()];
        let mut visited = Vec::new();
        while visited.len() < THEME_INHERITANCE_DEPTH && !pending.is_empty() {
            let theme = pending.remove(0);
            if visited.contains(&theme) {
                continue;
            }
            let directories: Vec<PathBuf> = self
                .search_path
                .iter()
                .map(|directory| directory.join(&theme))
                .collect();
            if let Some(file) = directories
                .iter()
                .map(|directory| directory.join("cursors").join(cursor))
                .find(|file| file.is_file())
            {
                return Some(file);
            }
            if let Some(inherits) = directories
                .iter()
                .find_map(|directory| fs::read_to_string(directory.join("index.theme")).ok())
                .map(|index| inherited_themes(&index))
            {
                pending.extend(inherits);
            }
            visited.push(theme);
        }
        None
    }
}

fn default_search_path(
    home: Option<PathBuf>,
    data_home: Option<PathBuf>,
    data_dirs: Option<OsString>,
) -> Vec<PathBuf> {
    let data_dirs = data_dirs.unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    data_home
        .or_else(|| Some(home.as_ref()?.join(".local/share")))
        .map(|data_home| data_home.join("icons"))
        .into_iter()
        .chain(home.map(|home| home.join(".icons")))
        .chain(env::split_paths(&data_dirs).map(|directory| directory.join("icons")))
        .chain(std::iter::once(PathBuf::from("/usr/share/pixmaps")))
        .collect()
}

fn inherited_themes(index: &str) -> Vec<String> {
    let mut in_icon_theme = false;
    for line in index.lines().map(str::trim) {
        if line.starts_with('[') {
            in_icon_theme = line == "[Icon Theme]";
        } else if let Some(value) = line
            .strip_prefix("Inherits")
            .and_then(|rest| rest.trim_start().strip_prefix('='))
            .filter(|_| in_icon_theme)
        {
            return value
                .split([',', ';'])
                .map(str::trim)
                .filter(|theme| !theme.is_empty())
                .map(str::to_owned)
                .collect();
        }
    }
    Vec::new()
}

fn word(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    Some(u32::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

fn parse_xcursor(bytes: &[u8]) -> Vec<XcursorImage> {
    if bytes.get(..4) != Some(XCURSOR_MAGIC) {
        return Vec::new();
    }
    let (Some(header), Some(entries)) = (word(bytes, 4), word(bytes, 12)) else {
        return Vec::new();
    };
    (0..entries as usize)
        .filter_map(|entry| {
            let at = (header as usize).checked_add(entry.checked_mul(12)?)?;
            (word(bytes, at)? == XCURSOR_IMAGE).then_some(())?;
            parse_image(bytes, word(bytes, at + 8)? as usize)
        })
        .collect()
}

fn parse_image(bytes: &[u8], at: usize) -> Option<XcursorImage> {
    let field = |index: usize| word(bytes, at.checked_add(index * 4)?);
    if field(1)? != XCURSOR_IMAGE {
        return None;
    }
    let (width, height) = (field(4)?, field(5)?);
    let hotspot = (field(6)?, field(7)?);
    let valid = (1..=XCURSOR_MAX_SIDE).contains(&width)
        && (1..=XCURSOR_MAX_SIDE).contains(&height)
        && hotspot.0 < width
        && hotspot.1 < height;
    let start = at.checked_add(XCURSOR_IMAGE_HEADER)?;
    let length = (width as usize * height as usize).checked_mul(4)?;
    let pixels = bytes.get(start..start.checked_add(length)?)?.to_vec();
    valid.then_some(XcursorImage {
        nominal: field(2)?,
        width,
        height,
        hotspot,
        pixels,
    })
}

fn best_image(images: Vec<XcursorImage>, size: u32) -> Option<XcursorImage> {
    let best = images
        .iter()
        .map(|image| image.nominal)
        .min_by_key(|nominal| nominal.abs_diff(size))?;
    images.into_iter().find(|image| image.nominal == best)
}
