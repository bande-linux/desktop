use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use crate::screen::Screen;

const LARGEST_VIDEO_WIDTH: u32 = 1920;

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgrx: Vec<u8>,
}

pub enum Request {
    Shot(Sender<Frame>),
    OpenApps(Sender<Vec<String>>),
    Stop,
}

pub struct Started {
    pub width: u32,
    pub height: u32,
    pub epoch: Instant,
}

struct Video {
    encoder: Child,
    input: Option<ChildStdin>,
    fps: u32,
    written: u64,
    held: Vec<u8>,
}

impl Video {
    fn start(path: &Path, log: &Path, (width, height): (u32, u32), fps: u32) -> io::Result<Self> {
        let scale =
            format!("scale='min({LARGEST_VIDEO_WIDTH},iw)':-2:flags=lanczos,format=yuv420p");
        let mut encoder = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "warning", "-y"])
            .args(["-f", "rawvideo", "-pixel_format", "bgr0"])
            .args(["-video_size", &format!("{width}x{height}")])
            .args(["-framerate", &fps.to_string(), "-i", "-"])
            .args(["-vf", &scale])
            .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "20"])
            .args(["-movflags", "+faststart"])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(File::create(log)?)
            .spawn()
            .map_err(|error| {
                io::Error::new(error.kind(), format!("cannot start ffmpeg: {error}"))
            })?;
        let input = encoder.stdin.take();
        Ok(Self {
            encoder,
            input,
            fps,
            written: 0,
            held: Vec::new(),
        })
    }

    fn tick_at(&self, at: Duration) -> u64 {
        (at.as_secs_f64() * f64::from(self.fps)) as u64
    }

    fn write_held_until(&mut self, tick: u64) -> io::Result<()> {
        let Some(input) = self.input.as_mut() else {
            return Err(io::Error::other("the encoder is closed"));
        };
        while self.written < tick && !self.held.is_empty() {
            input.write_all(&self.held)?;
            self.written += 1;
        }
        Ok(())
    }

    fn advance(&mut self, at: Duration, frame: &mut Vec<u8>) -> io::Result<()> {
        self.write_held_until(self.tick_at(at))?;
        std::mem::swap(&mut self.held, frame);
        Ok(())
    }

    fn finish(mut self, at: Duration) -> io::Result<()> {
        let last = self.tick_at(at).max(self.written + 1);
        self.write_held_until(last)?;
        drop(self.input.take());
        let status = self.encoder.wait()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("ffmpeg failed ({status})")))
        }
    }
}

pub const VIDEO: &str = "review.mp4";
pub const ENCODER_LOG: &str = "ffmpeg.log";

type RecordError = Box<dyn std::error::Error + Send + Sync>;

enum Encode {
    Frame(Duration, Vec<u8>),
    Finish(Duration),
}

fn encode(mut video: Video, frames: Receiver<Encode>, spare: Sender<Vec<u8>>) -> io::Result<()> {
    for message in frames {
        match message {
            Encode::Frame(at, mut pixels) => {
                video.advance(at, &mut pixels)?;
                let _ = spare.send(pixels);
            }
            Encode::Finish(at) => return video.finish(at),
        }
    }
    Err(io::Error::other(
        "the recording ended without finishing the video",
    ))
}

pub fn record(
    mut screen: Screen,
    out: &Path,
    fps: u32,
    requests: Receiver<Request>,
    started: Sender<Started>,
) -> Result<(), RecordError> {
    let (width, height) = screen.size();
    let video = Video::start(
        &out.join(VIDEO),
        &out.join(ENCODER_LOG),
        (width, height),
        fps,
    )?;
    let (frames, to_encode) = mpsc::sync_channel(1);
    let (recycle, spare) = mpsc::channel();
    let encoder = thread::spawn(move || encode(video, to_encode, recycle));
    let epoch = Instant::now();
    let _ = started.send(Started {
        width,
        height,
        epoch,
    });
    let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
    let captured = capture(&mut screen, epoch, interval, &requests, &frames, &spare);
    let _ = frames.send(Encode::Finish(epoch.elapsed()));
    drop(frames);
    let encoded = encoder.join().map_err(|_| "the encoder crashed")?;
    captured?;
    Ok(encoded?)
}

fn capture(
    screen: &mut Screen,
    epoch: Instant,
    interval: Duration,
    requests: &Receiver<Request>,
    frames: &SyncSender<Encode>,
    spare: &Receiver<Vec<u8>>,
) -> Result<(), RecordError> {
    let (width, height) = screen.size();
    let mut captures = 0u32;
    loop {
        let pending: Vec<Request> = requests.try_iter().collect();
        let mut pixels = spare.try_recv().unwrap_or_default();
        let began = epoch.elapsed();
        screen.capture(&mut pixels)?;
        captures += 1;
        let at = epoch.elapsed();
        let mut stop = false;
        for request in pending {
            match request {
                Request::Shot(reply) => {
                    let _ = reply.send(Frame {
                        width,
                        height,
                        bgrx: pixels.clone(),
                    });
                }
                Request::OpenApps(reply) => {
                    let _ = reply.send(screen.open_apps());
                }
                Request::Stop => stop = true,
            }
        }
        frames
            .send(Encode::Frame(at, pixels))
            .map_err(|_| "the encoder stopped")?;
        if stop {
            tracing::info!(
                "captured {captures} frames in {:.1} s ({:.1} per second)",
                at.as_secs_f64(),
                f64::from(captures) / at.as_secs_f64()
            );
            return Ok(());
        }
        if let Some(wait) = (began + interval).checked_sub(epoch.elapsed()) {
            thread::sleep(wait);
        }
    }
}
