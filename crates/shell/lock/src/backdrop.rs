use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use bande_content::{confine, only_compute_from_now_on, run_worker};
use bande_inbox::Inbox;
use bande_toolkit::Image;
use bande_wallpaper::{Choice, Fit, Picture, Source, Variant, read};
use bande_xdg::data_dirs;
use rustix::process::{Resource, Rlimit, setrlimit};

const WORKER_FLAG: &str = "--backdrop-worker";
const WORKER_TIME: Duration = Duration::from_secs(20);
const STANDARD_INPUT: &str = "/proc/self/fd/0";
const WORKER_MEMORY: u64 = 1 << 30;
const HEADER: usize = 8;

#[derive(Debug, Clone)]
pub enum BackdropSource {
    Session,
    Picture(Option<Arc<File>>, Fit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Blurred,
    Sharp,
}

impl Look {
    fn name(self) -> &'static str {
        match self {
            Self::Blurred => "blurred",
            Self::Sharp => "sharp",
        }
    }

    fn named(name: &str) -> Option<Self> {
        [Self::Blurred, Self::Sharp]
            .into_iter()
            .find(|look| look.name() == name)
    }
}

fn fit_name(fit: Fit) -> &'static str {
    match fit {
        Fit::Fill => "fill",
        Fit::Fit => "fit",
    }
}

fn fit_named(name: &str) -> Option<Fit> {
    [Fit::Fill, Fit::Fit]
        .into_iter()
        .find(|fit| fit_name(*fit) == name)
}

fn draw(look: Look, fit: Fit, size: (u32, u32), path: &Path) -> io::Result<Picture> {
    let source = Source::read(path).map_err(io::Error::other)?;
    if !only_compute_from_now_on() {
        return Err(io::Error::other("cannot confine a picture reader"));
    }
    match look {
        Look::Blurred => source.backdrop(fit, size.0, size.1),
        Look::Sharp => source.render(fit, size.0, size.1),
    }
    .map_err(io::Error::other)
}

fn write_picture(picture: &Picture) -> io::Result<()> {
    let mut out = io::stdout().lock();
    out.write_all(&picture.width.to_le_bytes())?;
    out.write_all(&picture.height.to_le_bytes())?;
    out.write_all(&picture.rgba)?;
    out.flush()
}

/// Serves the confined process that decodes one picture, so that a picture someone made cannot
/// reach the process that reads passwords. Returns nothing for other arguments.
pub fn serve_picture_worker(arguments: &[String]) -> Option<ExitCode> {
    let [flag, look, fit, width, height, path] = arguments else {
        return None;
    };
    if flag != WORKER_FLAG {
        return None;
    }
    confine();
    let memory = Rlimit {
        current: Some(WORKER_MEMORY),
        maximum: Some(WORKER_MEMORY),
    };
    if let Err(error) = setrlimit(Resource::As, memory) {
        tracing::warn!("cannot limit a picture reader's memory: {error}");
        return Some(ExitCode::FAILURE);
    }
    let (Some(look), Some(fit), Ok(width), Ok(height)) = (
        Look::named(look),
        fit_named(fit),
        width.parse(),
        height.parse(),
    ) else {
        return Some(ExitCode::FAILURE);
    };
    let drawn = draw(look, fit, (width, height), Path::new(path));
    Some(match drawn.and_then(|picture| write_picture(&picture)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::warn!("cannot draw a picture: {error}");
            ExitCode::FAILURE
        }
    })
}

fn picture_from(out: &[u8], size: (u32, u32)) -> Option<Picture> {
    let (header, rgba) = out.split_first_chunk::<HEADER>()?;
    let [width, height] = [&header[..4], &header[4..]]
        .map(|half| u32::from_le_bytes([half[0], half[1], half[2], half[3]]));
    let fits = width <= size.0.max(1) && height <= size.1.max(1);
    (fits && rgba.len() as u64 == u64::from(width) * u64::from(height) * 4).then(|| Picture {
        width,
        height,
        rgba: rgba.to_vec(),
    })
}

struct Drawing {
    path: PathBuf,
    opened: Option<Arc<File>>,
    look: Look,
    fit: Fit,
    size: (u32, u32),
}

impl Drawing {
    fn made(&self) -> Option<Picture> {
        let (path, stdin) = match &self.opened {
            Some(file) => (
                Path::new(STANDARD_INPUT),
                file.try_clone()
                    .inspect_err(|error| tracing::warn!("cannot pass on a picture: {error}"))
                    .ok()?
                    .into(),
            ),
            None => (self.path.as_path(), Stdio::null()),
        };
        let (width, height) = (self.size.0.to_string(), self.size.1.to_string());
        let arguments = [
            OsStr::new(WORKER_FLAG),
            OsStr::new(self.look.name()),
            OsStr::new(fit_name(self.fit)),
            OsStr::new(&width),
            OsStr::new(&height),
            path.as_os_str(),
        ];
        let most = u64::from(self.size.0) * u64::from(self.size.1) * 4 + HEADER as u64;
        let drawn = run_worker(&arguments, stdin, most, WORKER_TIME)
            .ok()
            .and_then(|out| picture_from(&out, self.size));
        if drawn.is_none() {
            tracing::warn!("cannot show the picture {}", self.path.display());
        }
        drawn
    }
}

pub fn load_backdrop(
    source: BackdropSource,
    variant: Variant,
    size: (u32, u32),
    looks: Vec<Look>,
) -> io::Result<Inbox<(Look, Image)>> {
    let received = Inbox::new()?;
    let sender = received.outbox();
    thread::Builder::new()
        .name("backdrop".to_owned())
        .spawn(move || {
            let (path, opened, fit) = match source {
                BackdropSource::Picture(Some(picture), fit) => {
                    (PathBuf::from(STANDARD_INPUT), Some(picture), fit)
                }
                BackdropSource::Picture(None, fit) => {
                    let Some(path) = chosen(&Choice::default(), variant) else {
                        return;
                    };
                    (path, None, fit)
                }
                BackdropSource::Session => {
                    let choice = bande_bus::session()
                        .map(|session| read(&session))
                        .inspect_err(|error| tracing::warn!("no session bus: {error}"))
                        .unwrap_or_default();
                    let Some(path) = chosen(&choice, variant) else {
                        return;
                    };
                    (path, None, choice.fit)
                }
            };
            for look in looks {
                let drawing = Drawing {
                    path: path.clone(),
                    opened: opened.clone(),
                    look,
                    fit,
                    size,
                };
                let image = drawing
                    .made()
                    .and_then(|drawn| Image::from_rgba(drawn.width, drawn.height, drawn.rgba));
                if let Some(image) = image
                    && !sender.send((look, image))
                {
                    return;
                }
            }
        })?;
    Ok(received)
}

fn chosen(choice: &Choice, variant: Variant) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    choice.path(variant, home.as_deref(), &data_dirs())
}

pub fn load_pictures(logins: Vec<String>, size: u32) -> io::Result<Inbox<(usize, Image)>> {
    let received = Inbox::new()?;
    let sender = received.outbox();
    thread::Builder::new()
        .name("picture".to_owned())
        .spawn(move || {
            let system = match bande_bus::system() {
                Ok(system) => system,
                Err(error) => {
                    tracing::warn!("no account pictures without the system bus: {error}");
                    return;
                }
            };
            for (index, login) in logins.iter().enumerate() {
                let Some(path) = bande_status::accounts::picture(&system, login) else {
                    continue;
                };
                let drawing = Drawing {
                    path,
                    opened: None,
                    look: Look::Sharp,
                    fit: Fit::Fill,
                    size: (size, size),
                };
                let image = drawing.made().and_then(|picture| {
                    Image::from_rgba(picture.width, picture.height, picture.rgba)
                });
                if let Some(image) = image
                    && !sender.send((index, image))
                {
                    return;
                }
            }
        })?;
    Ok(received)
}
