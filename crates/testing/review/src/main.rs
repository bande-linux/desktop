mod input;
mod keys;
mod recorder;
mod report;
mod scenario;
mod screen;

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use input::{Keyboard, Pointer, Touchpad};
use recorder::{Frame, Request, Started};
use report::{Entry, Outcome, Timeline};
use scenario::{Line, Step};
use screen::Screen;

const DEFAULT_FPS: u32 = 30;
const LEAD_IN: Duration = Duration::from_millis(1500);
const TAIL: Duration = Duration::from_millis(1500);
const SETTLE_AFTER_STEP: Duration = Duration::from_millis(250);
const WINDOW_POLL: Duration = Duration::from_millis(100);

struct Arguments {
    scenario: PathBuf,
    out: PathBuf,
    fps: u32,
}

fn arguments() -> Result<Arguments, String> {
    let usage = "usage: bande-review <scenario> --out <directory> [--fps <frames per second>]";
    let mut scenario = None;
    let mut out = None;
    let mut fps = DEFAULT_FPS;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = args.next().map(PathBuf::from),
            "--fps" => {
                fps = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|&value| value > 0)
                    .ok_or(usage)?;
            }
            _ if scenario.is_none() && !arg.starts_with("--") => {
                scenario = Some(PathBuf::from(arg))
            }
            _ => return Err(usage.to_owned()),
        }
    }
    Ok(Arguments {
        scenario: scenario.ok_or(usage)?,
        out: out.ok_or(usage)?,
        fps,
    })
}

fn main() -> ExitCode {
    bande_log::init("bande-review");
    match arguments()
        .map_err(Into::into)
        .and_then(|arguments| review(&arguments))
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            eprintln!("bande-review: {error}");
            ExitCode::FAILURE
        }
    }
}

struct Runner {
    keyboard: Keyboard,
    pointer: Pointer,
    touchpad: Option<Touchpad>,
    recorder: Sender<Request>,
    epoch: Instant,
    out: PathBuf,
    shots: usize,
}

fn review(arguments: &Arguments) -> Result<(), Box<dyn Error>> {
    let source = std::fs::read_to_string(&arguments.scenario)
        .map_err(|error| format!("cannot read {}: {error}", arguments.scenario.display()))?;
    let lines = scenario::parse(&source)?;
    std::fs::create_dir_all(arguments.out.join(report::SHOTS))?;
    let screen = Screen::connect()?;
    let keyboard = Keyboard::new()?;
    let pointer = Pointer::new()?;
    let touchpad = lines
        .iter()
        .any(|line| matches!(line.step, Step::Pinch { .. }))
        .then(Touchpad::new)
        .transpose()?;
    let (requests, received) = mpsc::channel();
    let (started_sender, started) = mpsc::channel();
    let out = arguments.out.clone();
    let fps = arguments.fps;
    let recording =
        thread::spawn(move || recorder::record(screen, &out, fps, received, started_sender));
    let Ok(Started {
        width,
        height,
        epoch,
    }) = started.recv()
    else {
        return Err(recording_failure(recording.join()));
    };
    let mut runner = Runner {
        keyboard,
        pointer,
        touchpad,
        recorder: requests,
        epoch,
        out: arguments.out.clone(),
        shots: 0,
    };
    runner.pointer.set_screen(width, height)?;
    thread::sleep(LEAD_IN);
    let mut entries = Vec::with_capacity(lines.len());
    let mut fatal = None;
    for line in &lines {
        tracing::info!("line {}: {}", line.number, line.source);
        let start = runner.now();
        let outcome = runner.run(line);
        if !matches!(line.step, Step::Note(_) | Step::Wait(_)) {
            thread::sleep(SETTLE_AFTER_STEP);
        }
        let failed = match &outcome {
            Err(error) => Some(error.to_string()),
            Ok(_) => None,
        };
        entries.push(Entry::new(line, start, runner.now(), outcome));
        if let Some(error) = failed {
            fatal = Some(format!("line {}: {error}", line.number));
            break;
        }
    }
    thread::sleep(TAIL);
    let duration = runner.now();
    let _ = runner.recorder.send(Request::Stop);
    let recorded = recording.join();
    let timeline = Timeline::new(
        &arguments.scenario,
        &source,
        (width, height),
        duration,
        entries,
        fatal.is_none(),
    );
    report::write(&arguments.out, &timeline)?;
    if let Some(fatal) = fatal {
        return Err(fatal.into());
    }
    match recorded {
        Ok(Ok(())) => Ok(()),
        other => Err(recording_failure(other)),
    }
}

type Recorded = thread::Result<Result<(), Box<dyn Error + Send + Sync>>>;

fn recording_failure(recorded: Recorded) -> Box<dyn Error> {
    match recorded {
        Ok(Err(error)) => format!("recording failed: {error}").into(),
        Ok(Ok(())) => "the recording ended early".into(),
        Err(_) => "the recorder crashed".into(),
    }
}

impl Runner {
    fn now(&self) -> Duration {
        self.epoch.elapsed()
    }

    fn ask<T>(&self, request: impl FnOnce(Sender<T>) -> Request) -> Result<T, Box<dyn Error>> {
        let (reply, answer) = mpsc::channel();
        self.recorder
            .send(request(reply))
            .map_err(|_| "the recorder stopped")?;
        Ok(answer.recv().map_err(|_| "the recorder stopped")?)
    }

    fn run(&mut self, line: &Line) -> Result<Outcome, Box<dyn Error>> {
        match &line.step {
            Step::Key(chords) => {
                for chord in chords {
                    self.keyboard.press(chord)?;
                }
            }
            Step::Type(text) => self.keyboard.type_text(text)?,
            Step::Move(point) => self.pointer.glide(*point)?,
            Step::Click { button, count, at } => {
                if let Some(point) = at {
                    self.pointer.glide(*point)?;
                }
                self.pointer.click(*button, *count)?;
            }
            Step::Drag { from, to } => self.pointer.drag(*from, *to)?,
            Step::Scroll { direction, detents } => self.pointer.scroll(*direction, *detents)?,
            Step::Pinch { pinch, fingers } => {
                if let Some(touchpad) = &mut self.touchpad {
                    touchpad.pinch(*pinch, *fingers)?;
                }
            }
            Step::Wait(duration) => thread::sleep(*duration),
            Step::WaitForWindow { app_id, timeout } => {
                return self.wait_for_window(app_id, *timeout);
            }
            Step::Shot(name) => return self.shot(name),
            Step::Note(_) => {}
        }
        Ok(Outcome::Done)
    }

    fn wait_for_window(&self, app_id: &str, timeout: Duration) -> Result<Outcome, Box<dyn Error>> {
        let started = Instant::now();
        loop {
            let open = self.ask(Request::OpenApps)?;
            if open.iter().any(|open| open == app_id) {
                return Ok(Outcome::Done);
            }
            if started.elapsed() >= timeout {
                tracing::warn!("{app_id} did not open within {timeout:?}; open: {open:?}");
                return Ok(Outcome::TimedOut);
            }
            thread::sleep(WINDOW_POLL);
        }
    }

    fn shot(&mut self, name: &str) -> Result<Outcome, Box<dyn Error>> {
        let frame = self.ask(Request::Shot)?;
        self.shots += 1;
        let file = report::shot_file(self.shots, name);
        save_png(&frame, &self.out.join(&file))?;
        Ok(Outcome::Shot(file))
    }
}

fn save_png(frame: &Frame, path: &Path) -> Result<(), Box<dyn Error>> {
    let rgb = frame
        .bgrx
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[blue, green, red, _]| [red, green, blue])
        .collect();
    let image = image::RgbImage::from_raw(frame.width, frame.height, rgb)
        .ok_or("the captured frame has the wrong size")?;
    image.save(path)?;
    Ok(())
}
