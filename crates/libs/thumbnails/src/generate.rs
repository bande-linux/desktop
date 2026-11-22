use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use bande_content::pdf::worker::first_page;
use bande_content::{confine, run_worker};
use bande_xdg::{find_executable, wait_for_exit};
use image::DynamicImage;

use crate::cache::{Cache, Picture, ThumbnailSize, modified, uri};

pub const WORKER_FLAG: &str = "--thumbnail-worker";
const THUMBNAILER_TIME: Duration = Duration::from_secs(15);
const WORKER_TIME: Duration = Duration::from_secs(20);
const LARGEST_OUTPUT: u64 = 4 * 1024 * 1024 + 64;
const PDF: &str = "application/pdf";
const KEPT_VARIABLES: [&str; 6] = [
    "PATH",
    "HOME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "XDG_DATA_DIRS",
];
const LARGEST_ERROR_LINES: usize = 20;
const ENTRY_GROUP: &str = "[Thumbnailer Entry]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnailer {
    exec: Vec<String>,
    mime_types: Vec<String>,
}

impl Thumbnailer {
    pub fn parse(text: &str) -> Option<Self> {
        let mut in_entry = false;
        let (mut exec, mut try_exec, mut mime_types) = (None, None, Vec::new());
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_entry = line == ENTRY_GROUP;
                continue;
            }
            let Some((key, value)) = line.split_once('=').filter(|_| in_entry) else {
                continue;
            };
            match key.trim() {
                "Exec" => exec = Some(value.trim().to_owned()),
                "TryExec" => try_exec = Some(value.trim().to_owned()),
                "MimeType" => {
                    mime_types = value
                        .split(';')
                        .map(str::trim)
                        .filter(|mime| !mime.is_empty())
                        .map(str::to_owned)
                        .collect();
                }
                _ => {}
            }
        }
        let exec = exec?
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let program = try_exec.as_deref().or(exec.first().map(String::as_str))?;
        find_executable(program)?;
        Some(Self { exec, mime_types })
    }

    fn command(&self, input: &Path, output: &Path, size: ThumbnailSize) -> Vec<String> {
        self.exec
            .iter()
            .map(|argument| {
                let mut expanded = String::new();
                let mut characters = argument.chars();
                while let Some(character) = characters.next() {
                    if character != '%' {
                        expanded.push(character);
                        continue;
                    }
                    match characters.next() {
                        Some('s') => expanded.push_str(&size.pixels().to_string()),
                        Some('u') => expanded.push_str(&uri(input)),
                        Some('i') => expanded.push_str(&input.to_string_lossy()),
                        Some('o') => expanded.push_str(&output.to_string_lossy()),
                        Some('%') => expanded.push('%'),
                        _ => {}
                    }
                }
                expanded
            })
            .collect()
    }

    fn run(&self, input: &Path, output: &Path, size: ThumbnailSize) -> bool {
        let command = self.command(input, output, size);
        let Some((program, arguments)) = command.split_first() else {
            return false;
        };
        let kept = KEPT_VARIABLES
            .iter()
            .filter_map(|name| Some((*name, std::env::var_os(name)?)));
        let child = Command::new(program)
            .args(arguments)
            .env_clear()
            .envs(kept)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!("cannot run the thumbnailer {program}: {error}");
                return false;
            }
        };
        let errors = child.stderr.take().map(|stderr| {
            let program = program.clone();
            thread::spawn(move || {
                let mut errors = BufReader::new(stderr);
                for line in (&mut errors)
                    .lines()
                    .map_while(Result::ok)
                    .take(LARGEST_ERROR_LINES)
                {
                    tracing::info!("thumbnailer {program}: {line}");
                }
                if let Err(error) = io::copy(&mut errors, &mut io::sink()) {
                    tracing::debug!("cannot read the thumbnailer {program}: {error}");
                }
            })
        });
        let exited = wait_for_exit(&mut child, Some(Instant::now() + THUMBNAILER_TIME), None);
        let finished = match exited {
            Ok(Some(status)) => Some(status.success()),
            Ok(None) => {
                tracing::warn!("the thumbnailer {program} took too long");
                None
            }
            Err(error) => {
                tracing::warn!("cannot wait for the thumbnailer {program}: {error}");
                None
            }
        };
        if finished.is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(errors) = errors
            && errors.join().is_err()
        {
            tracing::debug!("the thumbnailer {program} left no readable errors");
        }
        finished.unwrap_or(false)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Thumbnailers {
    installed: Vec<Thumbnailer>,
}

impl Thumbnailers {
    pub fn load(data_dirs: &[PathBuf]) -> Self {
        let mut seen = HashSet::new();
        let mut installed = Vec::new();
        for folder in data_dirs.iter().map(|dir| dir.join("thumbnailers")) {
            let Ok(entries) = fs::read_dir(&folder) else {
                continue;
            };
            let mut files = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "thumbnailer"))
                .collect::<Vec<_>>();
            files.sort();
            for file in files {
                let name = file.file_name().map(|name| name.to_owned());
                if !seen.insert(name) {
                    continue;
                }
                if let Some(thumbnailer) = fs::read_to_string(&file)
                    .ok()
                    .and_then(|text| Thumbnailer::parse(&text))
                {
                    installed.push(thumbnailer);
                }
            }
        }
        Self { installed }
    }

    fn for_mime(&self, mime: &str) -> Option<&Thumbnailer> {
        self.installed
            .iter()
            .find(|thumbnailer| thumbnailer.mime_types.iter().any(|known| known == mime))
    }
}

fn fit_pixels(width: u32, height: u32, rgba: Vec<u8>, size: ThumbnailSize) -> Option<Picture> {
    let image = image::RgbaImage::from_raw(width, height, rgba)?;
    Some(fit(DynamicImage::ImageRgba8(image), size))
}

fn fit(image: DynamicImage, size: ThumbnailSize) -> Picture {
    let pixels = size.pixels();
    let image = if image.width().max(image.height()) > pixels {
        image.thumbnail(pixels, pixels)
    } else {
        image
    };
    let rgba = image.into_rgba8();
    let (width, height) = rgba.dimensions();
    Picture {
        width,
        height,
        rgba: rgba.into_raw(),
    }
}

fn decode(path: &Path) -> Option<DynamicImage> {
    image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()
}

fn external(
    root: &Path,
    thumbnailer: &Thumbnailer,
    path: &Path,
    size: ThumbnailSize,
) -> Option<Picture> {
    fs::create_dir_all(root).ok()?;
    let output = root.join(format!(
        ".bande-{}-{}.png",
        std::process::id(),
        size.pixels()
    ));
    let made = thumbnailer
        .run(path, &output, size)
        .then(|| decode(&output))
        .flatten();
    if let Err(error) = fs::remove_file(&output)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::debug!("cannot remove {}: {error}", output.display());
    }
    made.map(|image| fit(image, size))
}

fn is_font(mime: &str) -> bool {
    mime.starts_with("font/") || mime.starts_with("application/x-font")
}

fn built_in(mime: &str) -> bool {
    mime.starts_with("image/") || mime == PDF || is_font(mime)
}

/// Draws a font at `path` into a square of the given pixels, for font thumbnails.
pub type FontDrawing<'a> = &'a dyn Fn(&Path, u32) -> Option<Picture>;

fn size_of(pixels: &str) -> Option<ThumbnailSize> {
    ThumbnailSize::ALL
        .into_iter()
        .find(|size| size.pixels().to_string() == pixels)
}

fn generate(
    path: &Path,
    mime: &str,
    size: ThumbnailSize,
    root: &Path,
    font: FontDrawing<'_>,
) -> Option<Picture> {
    let made = if mime.starts_with("image/") {
        decode(path).map(|image| fit(image, size))
    } else if mime == PDF {
        first_page(path, size.pixels())
            .and_then(|(width, height, rgba)| fit_pixels(width, height, rgba, size))
    } else if is_font(mime) {
        font(path, size.pixels())
    } else {
        None
    };
    made.or_else(|| {
        let thumbnailers = Thumbnailers::load(&bande_xdg::data_dirs());
        external(root, thumbnailers.for_mime(mime)?, path, size)
    })
}

/// Serves `WORKER_FLAG`: makes one thumbnail in this confined process and writes it to standard
/// output. Returns nothing for other arguments.
pub fn serve_worker(arguments: &[String], font: FontDrawing<'_>) -> Option<ExitCode> {
    let [flag, pixels, mime, path, root] = arguments else {
        return None;
    };
    if flag != WORKER_FLAG {
        return None;
    }
    confine();
    let picture = size_of(pixels)
        .and_then(|size| generate(Path::new(path), mime, size, Path::new(root), font));
    let Some(picture) = picture else {
        return Some(ExitCode::FAILURE);
    };
    let mut out = io::stdout().lock();
    let written = writeln!(out, "{} {}", picture.width, picture.height)
        .and_then(|()| out.write_all(&picture.rgba))
        .and_then(|()| out.flush());
    Some(match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    })
}

fn picture_from(out: &[u8]) -> Option<Picture> {
    let end = out.iter().position(|byte| *byte == b'\n')?;
    let (width, height) = std::str::from_utf8(&out[..end]).ok()?.split_once(' ')?;
    let (width, height): (u32, u32) = (width.parse().ok()?, height.parse().ok()?);
    let rgba = &out[end + 1..];
    (width.max(height) <= ThumbnailSize::XxLarge.pixels()
        && rgba.len() as u64 == u64::from(width) * u64::from(height) * 4)
        .then(|| Picture {
            width,
            height,
            rgba: rgba.to_vec(),
        })
}

fn in_worker(cache: &Cache, path: &Path, mime: &str, size: ThumbnailSize) -> Option<Picture> {
    let pixels = size.pixels().to_string();
    let arguments = [
        OsStr::new(WORKER_FLAG),
        OsStr::new(&pixels),
        OsStr::new(mime),
        path.as_os_str(),
        cache.root().as_os_str(),
    ];
    let out = run_worker(&arguments, LARGEST_OUTPUT, WORKER_TIME)?;
    picture_from(&out)
}

pub fn thumbnail(
    cache: &Cache,
    thumbnailers: &Thumbnailers,
    path: &Path,
    mime: &str,
    size: ThumbnailSize,
) -> Option<Picture> {
    if path.starts_with(cache.root()) {
        return None;
    }
    if !built_in(mime) && thumbnailers.for_mime(mime).is_none() {
        return None;
    }
    let mtime = modified(path)?;
    let uri = uri(path);
    if let Some(picture) = cache.load(&uri, mtime, size) {
        return Some(picture);
    }
    if cache.has_failed(&uri, mtime) {
        return None;
    }
    let made = in_worker(cache, path, mime, size);
    let stored = match &made {
        Some(picture) => cache.store(&uri, mtime, size, picture),
        None => cache.record_failure(&uri, mtime),
    };
    if let Err(error) = stored {
        tracing::warn!("cannot store the thumbnail of {}: {error}", path.display());
    }
    made
}
