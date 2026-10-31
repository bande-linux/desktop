use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use unbound_files::{Browser, FilesView, Location};
use unbound_fs::Trash;
use unbound_toolkit::ui::{Event, Key, Modifiers, Scroll, ScrollSource};
use unbound_toolkit::{Palette, Point, Scheme, TextSystem};
use unbound_xdg::{Apps, config_dirs, data_dirs};

const ENTRIES: usize = 100_000;
const RUNS: usize = 5;
const SCROLL_FRAMES: usize = 120;
const WIDTH: u32 = 1400;
const HEIGHT: u32 = 900;
const SETTLE_TIMEOUT: Duration = Duration::from_secs(60);

struct Sample {
    name: &'static str,
    values: Vec<f64>,
}

impl Sample {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            values: Vec::new(),
        }
    }

    fn percentile(&self, fraction: f64) -> f64 {
        let mut values = self.values.clone();
        values.sort_by(f64::total_cmp);
        let index = ((values.len() as f64 - 1.0) * fraction).round() as usize;
        values.get(index).copied().unwrap_or_default()
    }
}

fn main() -> ExitCode {
    unbound_log::init("unbound-files-bench");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

fn command(key: char) -> Event {
    Event::Key {
        key: Key::Character(key),
        modifiers: Modifiers {
            logo: true,
            ..Modifiers::default()
        },
    }
}

fn frame(view: &mut FilesView) {
    view.scene(WIDTH, HEIGHT);
}

fn timed(view: &mut FilesView, work: impl FnOnce(&mut FilesView)) -> f64 {
    let started = Instant::now();
    work(view);
    frame(view);
    milliseconds(started.elapsed())
}

fn ready_sources(view: &FilesView, timeout: Duration) -> Result<Vec<usize>, Box<dyn Error>> {
    let fds = view.fds();
    let mut polls = fds
        .iter()
        .map(|fd| PollFd::new(fd, PollFlags::IN))
        .collect::<Vec<_>>();
    let timeout = Timespec {
        tv_sec: 0,
        tv_nsec: i64::from(timeout.subsec_nanos()),
    };
    poll(&mut polls, Some(&timeout))?;
    Ok(polls
        .iter()
        .enumerate()
        .filter(|(_, fd)| !fd.revents().is_empty())
        .map(|(index, _)| index)
        .collect())
}

fn settle(
    view: &mut FilesView,
    done: impl Fn(&FilesView) -> bool,
) -> Result<(f64, f64), Box<dyn Error>> {
    let started = Instant::now();
    let mut busy = 0.0;
    let mut longest: f64 = 0.0;
    while !done(view) {
        if started.elapsed() > SETTLE_TIMEOUT {
            return Err("Files did not settle".into());
        }
        let ready = ready_sources(view, Duration::from_millis(5))?;
        if ready.is_empty() {
            continue;
        }
        let work = timed(view, |view| {
            for index in ready.into_iter().rev() {
                view.source_ready(index);
            }
            view.tick(Instant::now() + Duration::from_secs(1));
        });
        busy += work;
        longest = longest.max(work);
    }
    Ok((busy, longest))
}

fn listed(view: &FilesView, count: usize) -> bool {
    view.browser().columns()[0].entries.len() >= count
}

fn open(folder: &Path, trash: &Trash) -> Result<FilesView, Box<dyn Error>> {
    let browser = Browser::open(Location::Folder(folder.to_path_buf()), trash.clone());
    Ok(FilesView::new(
        browser,
        Vec::new(),
        Apps::load(data_dirs(), &config_dirs()),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    )?)
}

fn run() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let folder = root.path().join("Big");
    fs::create_dir_all(&folder)?;
    for entry in 0..ENTRIES {
        fs::write(folder.join(format!("Photo {entry:06}.jpg")), b"jpeg")?;
    }
    let trash = Trash::new(root.path().join("Trash"), 0);
    let mut first_frame = Sample::new("Open to first frame");
    let mut fully_listed = Sample::new("Open to fully listed");
    let mut longest_stall = Sample::new("Longest stall while listing");
    let mut list_switch = Sample::new("Switch to the list view");
    let mut scrolling = Sample::new("Frame while scrolling the list (p99 of each run)");
    let mut select_all = Sample::new("Select All in the list view");
    let mut change = Sample::new("One external change in the list view");
    let mut change_stall = Sample::new("Longest stall for that change");
    for run in 0..RUNS {
        let started = Instant::now();
        let mut view = open(&folder, &trash)?;
        frame(&mut view);
        first_frame.values.push(milliseconds(started.elapsed()));
        let (_, longest) = settle(&mut view, |view| listed(view, ENTRIES))?;
        fully_listed.values.push(milliseconds(started.elapsed()));
        longest_stall.values.push(longest);

        list_switch
            .values
            .push(timed(&mut view, |view| _ = view.handle(&command('2'))));
        settle(&mut view, |view| listed(view, ENTRIES))?;

        let mut frames = Sample::new("scroll");
        for step in 0..SCROLL_FRAMES {
            frames.values.push(timed(&mut view, |view| {
                _ = view.handle(&Event::Scroll(Scroll {
                    position: Point::new(WIDTH as f32 * 0.6, HEIGHT as f32 * 0.5),
                    delta: Point::new(0.0, 240.0),
                    source: ScrollSource::Wheel,
                    time: Duration::from_millis(step as u64 * 8),
                    ..Scroll::default()
                }));
            }));
        }
        scrolling.values.push(frames.percentile(0.99));

        select_all
            .values
            .push(timed(&mut view, |view| _ = view.handle(&command('a'))));

        let name = format!("New {run}.txt");
        fs::write(folder.join(&name), b"new")?;
        let (busy, longest) = settle(&mut view, |view| {
            view.browser().columns()[0]
                .entries
                .iter()
                .any(|entry| entry.name == name)
        })?;
        change.values.push(busy);
        change_stall.values.push(longest);
        fs::remove_file(folder.join(&name))?;
    }
    println!("### Files with {ENTRIES} items, {RUNS} runs, {WIDTH}×{HEIGHT}\n");
    println!("| Action | Median ms | Worst ms |");
    println!("| :--- | ---: | ---: |");
    for sample in [
        first_frame,
        fully_listed,
        longest_stall,
        list_switch,
        scrolling,
        select_all,
        change,
        change_stall,
    ] {
        println!(
            "| {} | {:.1} | {:.1} |",
            sample.name,
            sample.percentile(0.5),
            sample.percentile(1.0)
        );
    }
    Ok(())
}
