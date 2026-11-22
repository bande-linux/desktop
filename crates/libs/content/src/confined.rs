use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use bande_xdg::wait_for_exit;
use serde::{Deserialize, Serialize};

use crate::decode::{Decoded, Failure, Frame, Kind, Level, MAX_PIXELS, Pixels, decode_file};
use crate::facts::Facts;
use crate::pdf::protocol;
use crate::{confine, worker_program};

const PICTURE_FLAG: &str = "--picture-worker";
const PICTURE_TIME: Duration = Duration::from_secs(20);
const PICTURE_BYTES: u64 = MAX_PIXELS * 6;
const HEAD_BYTES: u64 = 1024;

#[derive(Serialize, Deserialize)]
enum Reply {
    Still {
        width: u32,
        height: u32,
        levels: Vec<(u32, u32)>,
    },
    Animation {
        width: u32,
        height: u32,
        delays: Vec<u64>,
    },
    Failed(Failure),
}

/// Runs this program again with `arguments` as a confined worker and returns what it wrote to
/// standard output, up to `most` bytes, when it succeeds within `time`.
pub fn run_worker(arguments: &[&OsStr], most: u64, time: Duration) -> Option<Vec<u8>> {
    let mut child = Command::new(worker_program())
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .inspect_err(|error| tracing::warn!("cannot start a file reader: {error}"))
        .ok()?;
    let stdout = child.stdout.take()?;
    let reader = thread::Builder::new()
        .name("file reader".to_owned())
        .spawn(move || {
            let mut out = Vec::new();
            stdout.take(most).read_to_end(&mut out).map(|_| out)
        })
        .inspect_err(|error| tracing::warn!("cannot read a file reader: {error}"))
        .ok()?;
    let exited = wait_for_exit(&mut child, Some(Instant::now() + time), None);
    let done = matches!(exited, Ok(Some(status)) if status.success());
    if !done {
        if let Ok(None) = exited {
            tracing::warn!("a file reader took too long");
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    let out = reader.join().ok()?.ok()?;
    done.then_some(out)
}

/// Serves the confined readers of this crate. Returns nothing for other arguments.
pub fn serve_worker(arguments: &[String]) -> Option<ExitCode> {
    match arguments {
        [flag, path] if flag == protocol::WORKER_FLAG => {
            Some(crate::pdf::worker::run(Path::new(path)))
        }
        [flag, longest, path] if flag == PICTURE_FLAG => {
            confine();
            let Ok(longest) = longest.parse() else {
                return Some(ExitCode::FAILURE);
            };
            Some(serve_picture(Path::new(path), longest))
        }
        _ => None,
    }
}

fn serve_picture(path: &Path, longest: u32) -> ExitCode {
    let (reply, payload) = match decode_file(path) {
        Ok(Decoded {
            width,
            height,
            pixels: Pixels::Still(levels),
            ..
        }) => {
            let shown: Vec<Level> = levels
                .into_iter()
                .skip_while(|level| level.width.max(level.height) > longest)
                .collect();
            let reply = Reply::Still {
                width,
                height,
                levels: shown
                    .iter()
                    .map(|level| (level.width, level.height))
                    .collect(),
            };
            (
                reply,
                shown
                    .iter()
                    .map(|level| level.rgba.as_slice())
                    .collect::<Vec<_>>()
                    .concat(),
            )
        }
        Ok(Decoded {
            width,
            height,
            pixels: Pixels::Animation(frames),
            ..
        }) => {
            let reply = Reply::Animation {
                width,
                height,
                delays: frames
                    .iter()
                    .map(|frame| frame.delay.as_millis() as u64)
                    .collect(),
            };
            (
                reply,
                frames
                    .iter()
                    .map(|frame| frame.rgba.as_slice())
                    .collect::<Vec<_>>()
                    .concat(),
            )
        }
        Err(failure) => (Reply::Failed(failure), Vec::new()),
    };
    match protocol::write(&mut io::stdout().lock(), &reply, &payload) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn kind_of(path: &Path) -> Result<Kind, Failure> {
    let mut head = Vec::new();
    File::open(path)
        .and_then(|file| file.take(HEAD_BYTES).read_to_end(&mut head))
        .map_err(|_| Failure::Unreadable)?;
    match Kind::of(&head, path) {
        Some(Kind::Pdf) | None => Err(Failure::NotSupported),
        Some(kind) => Ok(kind),
    }
}

fn rgba_bytes(width: u32, height: u32) -> usize {
    width as usize * height as usize * 4
}

fn split(payload: &[u8], sizes: impl Iterator<Item = usize>) -> Option<Vec<Arc<Vec<u8>>>> {
    let mut rest = payload;
    let mut parts = Vec::new();
    for size in sizes {
        let (part, after) = rest.split_at_checked(size)?;
        parts.push(Arc::new(part.to_vec()));
        rest = after;
    }
    rest.is_empty().then_some(parts)
}

/// Decodes the image at `path` in a confined worker, keeping only the levels whose longest side
/// is at most `longest` pixels.
pub fn decode_confined(path: &Path, longest: u32) -> Result<Decoded, Failure> {
    let kind = kind_of(path)?;
    let longest = longest.to_string();
    let arguments = [
        OsStr::new(PICTURE_FLAG),
        OsStr::new(&longest),
        path.as_os_str(),
    ];
    let out = run_worker(&arguments, PICTURE_BYTES, PICTURE_TIME).ok_or(Failure::Damaged)?;
    let mut input = out.as_slice();
    let reply: Reply = protocol::read(&mut input)
        .ok()
        .flatten()
        .ok_or(Failure::Damaged)?;
    let (width, height, pixels) = match reply {
        Reply::Failed(failure) => return Err(failure),
        Reply::Still {
            width,
            height,
            levels,
        } => {
            let parts = split(input, levels.iter().map(|&(w, h)| rgba_bytes(w, h)))
                .ok_or(Failure::Damaged)?;
            let levels: Vec<Level> = levels
                .into_iter()
                .zip(parts)
                .map(|((width, height), rgba)| Level {
                    width,
                    height,
                    rgba,
                })
                .collect();
            if levels.is_empty() {
                return Err(Failure::Damaged);
            }
            (width, height, Pixels::Still(levels))
        }
        Reply::Animation {
            width,
            height,
            delays,
        } => {
            let frame = rgba_bytes(width, height);
            let parts = split(input, delays.iter().map(|_| frame)).ok_or(Failure::Damaged)?;
            let frames: Vec<Frame> = delays
                .into_iter()
                .zip(parts)
                .map(|(delay, rgba)| Frame {
                    rgba,
                    delay: Duration::from_millis(delay),
                })
                .collect();
            if frames.is_empty() {
                return Err(Failure::Damaged);
            }
            (width, height, Pixels::Animation(frames))
        }
    };
    Ok(Decoded {
        width,
        height,
        pixels,
        facts: Facts {
            kind: Some(kind),
            ..Facts::default()
        },
    })
}
