use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use bande_content::pdf::protocol::WORKER_FLAG;
use bande_content::{Contents, confine, worker_program};
use bande_fs::archive_contents;
use bande_toolkit::{Color, CpuRenderer, Item, Point, Scene, TextStyle, TextSystem};
use bande_xdg::wait_for_exit;

use crate::L10N;
use crate::paint::colored;

const FONT_FLAG: &str = "--quick-look-font";
const ARCHIVE_FLAG: &str = "--quick-look-archive";
const FONT_BYTES: u64 = 64 << 20;
const ARCHIVE_ENTRIES: u64 = 100_000;
const WORKER_TIME: Duration = Duration::from_secs(15);
const MOST_PIXELS: u64 = 40 << 20;
pub const SAMPLE_WIDTH: f32 = 760.0;
const SAMPLE_SCALE: f64 = 2.0;
const ALPHABET: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ\nabcdefghijklmnopqrstuvwxyz\n0123456789 .,;:!?&@#%()[]";

pub struct Sample {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn serve_worker(arguments: &[String]) -> Option<ExitCode> {
    let [flag, path] = arguments else {
        return None;
    };
    let path = Path::new(path);
    match flag.as_str() {
        WORKER_FLAG => Some(bande_content::pdf::worker::run(path)),
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

fn font_sample(path: &Path) -> io::Result<Sample> {
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
    let sample = L10N.tr("font-sample");
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
    let mut child = Command::new(worker_program())
        .arg(flag)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .inspect_err(|error| tracing::warn!("cannot start a file reader: {error}"))
        .ok()?;
    let stdout = child.stdout.take()?;
    let reader = thread::Builder::new()
        .name("quick look reader".to_owned())
        .spawn(move || {
            let mut out = Vec::new();
            stdout
                .take(MOST_PIXELS * 4 + 64)
                .read_to_end(&mut out)
                .map(|_| out)
        })
        .ok()?;
    let exited = wait_for_exit(&mut child, Some(Instant::now() + WORKER_TIME), None);
    if !matches!(exited, Ok(Some(status)) if status.success()) {
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();
        return None;
    }
    reader.join().ok()?.ok()
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
