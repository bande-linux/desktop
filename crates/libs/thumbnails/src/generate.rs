use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use bande_xdg::find_executable;
use image::DynamicImage;

use crate::cache::{Cache, Picture, ThumbnailSize, modified, uri};

const THUMBNAILER_TIME: Duration = Duration::from_secs(15);
const WAIT_STEP: Duration = Duration::from_millis(20);
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
        let child = Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!("cannot run the thumbnailer {program}: {error}");
                return false;
            }
        };
        let deadline = Instant::now() + THUMBNAILER_TIME;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if Instant::now() < deadline => thread::sleep(WAIT_STEP),
                _ => {
                    tracing::warn!("the thumbnailer {program} took too long");
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
            }
        }
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
    cache: &Cache,
    thumbnailer: &Thumbnailer,
    path: &Path,
    size: ThumbnailSize,
) -> Option<Picture> {
    fs::create_dir_all(cache.root()).ok()?;
    let output = cache.root().join(format!(
        ".bande-{}-{:?}-{}.png",
        std::process::id(),
        thread::current().id(),
        size.pixels()
    ));
    let made = thumbnailer
        .run(path, &output, size)
        .then(|| decode(&output))
        .flatten();
    let _ = fs::remove_file(&output);
    made.map(|image| fit(image, size))
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
    let image = mime.starts_with("image/");
    let thumbnailer = thumbnailers.for_mime(mime);
    if !image && thumbnailer.is_none() {
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
    let made = image
        .then(|| decode(path).map(|image| fit(image, size)))
        .flatten()
        .or_else(|| external(cache, thumbnailer?, path, size));
    let stored = match &made {
        Some(picture) => cache.store(&uri, mtime, size, picture),
        None => cache.record_failure(&uri, mtime),
    };
    if let Err(error) = stored {
        tracing::warn!("cannot store the thumbnail of {}: {error}", path.display());
    }
    made
}
