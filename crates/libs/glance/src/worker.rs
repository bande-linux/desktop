use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use bande_l10n::tr;

use crate::paint::colored;
use bande_content::{Contents, confine, only_compute_from_now_on, run_worker};
use bande_fs::archive_contents;
use bande_thumbnails::Picture;
use bande_toolkit::{Color, CpuRenderer, Item, Point, Scene, TextStyle, TextSystem};

const FONT_FLAG: &str = "--glance-font";
const ARCHIVE_FLAG: &str = "--glance-archive";
const FONT_BYTES: u64 = 64 << 20;
const ARCHIVE_ENTRIES: u64 = 100_000;
const WORKER_TIME: Duration = Duration::from_secs(15);
const MOST_PIXELS: u64 = 40 << 20;
pub const SAMPLE_WIDTH: f32 = 760.0;
const SAMPLE_SCALE: f64 = 2.0;
const FONT_THUMBNAIL_TEXT: &str = "Aa";
const ALPHABET: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ\nabcdefghijklmnopqrstuvwxyz\n0123456789 .,;:!?&@#%()[]";

pub struct Sample {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn serve_worker(arguments: &[String]) -> Option<ExitCode> {
    if let Some(code) = bande_thumbnails::serve_worker(arguments, &font_thumbnail) {
        return Some(code);
    }
    if let Some(code) = bande_content::serve_worker(arguments) {
        return Some(code);
    }
    let [flag, path] = arguments else {
        return None;
    };
    let path = Path::new(path);
    match flag.as_str() {
        FONT_FLAG => {
            confine();
            Some(exit(font_sample(path).and_then(|sample| {
                let mut out = io::stdout().lock();
                writeln!(out, "{} {}", sample.width, sample.height)?;
                out.write_all(&sample.rgba)?;
                out.flush()
            })))
        }
        ARCHIVE_FLAG => {
            confine();
            let contents = archive_contents(path, ARCHIVE_ENTRIES)
                .ok_or_else(|| io::Error::other("not a readable archive"));
            Some(exit(contents.and_then(|contents| {
                writeln!(io::stdout().lock(), "{} {}", contents.items, contents.bytes)
            })))
        }
        _ => None,
    }
}

fn exit(result: io::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn load_font(path: &Path) -> io::Result<(TextSystem, String)> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(FONT_BYTES)
        .read_to_end(&mut bytes)?;
    let mut text = TextSystem::new();
    let (fonts, _) = text.contexts();
    let registered = fonts
        .collection
        .register_fonts(parley::fontique::Blob::new(Arc::new(bytes)), None);
    let family = registered
        .first()
        .and_then(|(family, _)| fonts.collection.family_name(*family))
        .map(str::to_owned)
        .ok_or_else(|| io::Error::other("not a font"))?;
    Ok((text, family))
}

fn font_thumbnail(path: &Path, pixels: u32) -> Option<Picture> {
    let (mut text, family) = load_font(path).ok()?;
    let side = pixels as f32;
    let paragraph = colored(
        &mut text,
        FONT_THUMBNAIL_TEXT,
        TextStyle::new(side * 0.5, Color::BLACK),
        Some(&family),
        &[],
        None,
    );
    let origin = Point::new(
        ((side - paragraph.width()) / 2.0).max(0.0),
        ((side - paragraph.height()) / 2.0).max(0.0),
    );
    let scene = Scene {
        width: pixels,
        height: pixels,
        background: Color::WHITE,
        items: vec![Item::Text { origin, paragraph }],
    };
    let mut rgba = vec![0; pixels as usize * pixels as usize * 4];
    CpuRenderer::new()
        .render_into(&scene, 1.0, &mut rgba)
        .inspect_err(|error| tracing::warn!("cannot draw a font thumbnail: {error:?}"))
        .ok()?;
    Some(Picture {
        width: pixels,
        height: pixels,
        rgba,
    })
}

fn font_sample(path: &Path) -> io::Result<Sample> {
    let (mut text, family) = load_font(path)?;
    let sample = tr!("The quick brown fox jumps over the lazy dog.");
    let mut lines = vec![(family.clone(), 36.0), (ALPHABET.to_owned(), 22.0)];
    lines.extend([12.0, 18.0, 24.0, 36.0, 48.0, 64.0].map(|size| (sample.clone(), size)));
    let mut items = Vec::new();
    let mut y = 0.0;
    for (line, size) in lines {
        let paragraph = colored(
            &mut text,
            &line,
            TextStyle::new(size, Color::BLACK),
            Some(&family),
            &[],
            Some(SAMPLE_WIDTH),
        );
        let height = paragraph.height();
        items.push(Item::Text {
            origin: Point::new(0.0, y),
            paragraph,
        });
        y += height + 18.0;
    }
    let scene = Scene {
        width: SAMPLE_WIDTH as u32,
        height: y.ceil().max(1.0) as u32,
        background: Color::TRANSPARENT,
        items,
    };
    let width = (f64::from(scene.width) * SAMPLE_SCALE).ceil() as u32;
    let height = (f64::from(scene.height) * SAMPLE_SCALE).ceil() as u32;
    if u64::from(width) * u64::from(height) > MOST_PIXELS {
        return Err(io::Error::other("the sample is too large"));
    }
    if !only_compute_from_now_on() {
        return Err(io::Error::other("cannot confine a font reader"));
    }
    let mut rgba = vec![0; width as usize * height as usize * 4];
    CpuRenderer::new()
        .render_into(&scene, SAMPLE_SCALE, &mut rgba)
        .map_err(|error| io::Error::other(format!("{error:?}")))?;
    Ok(Sample {
        width,
        height,
        rgba,
    })
}

fn run(flag: &str, path: &Path) -> Option<Vec<u8>> {
    run_worker(
        &[OsStr::new(flag), path.as_os_str()],
        MOST_PIXELS * 4 + 64,
        WORKER_TIME,
    )
    .ok()
}

fn header(out: &[u8]) -> Option<(u64, u64, &[u8])> {
    let end = out.iter().position(|byte| *byte == b'\n')?;
    let line = std::str::from_utf8(&out[..end]).ok()?;
    let (first, second) = line.split_once(' ')?;
    Some((first.parse().ok()?, second.parse().ok()?, &out[end + 1..]))
}

pub fn read_font_sample(path: &Path) -> Option<Sample> {
    let out = run(FONT_FLAG, path)?;
    let (width, height, rgba) = header(&out)?;
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    (rgba.len() as u64 == u64::from(width) * u64::from(height) * 4).then(|| Sample {
        width,
        height,
        rgba: rgba.to_vec(),
    })
}

pub fn read_archive(path: &Path) -> Option<Contents> {
    let out = run(ARCHIVE_FLAG, path)?;
    let (items, bytes, _) = header(&out)?;
    Some(Contents { items, bytes })
}
