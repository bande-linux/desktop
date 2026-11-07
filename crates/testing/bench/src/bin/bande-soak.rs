use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{
    Compositor, FrameClock, FrameOutput, FrameScheduler, OutputConfig, REFRESH, RenderError,
    Renderer, SessionRole, Submission,
};
use rustix::process::{Pid, Signal, kill_process};
use smithay::utils::{Point, Size};

const OUTPUT: (i32, i32) = (1920, 1080);
const SECOND_OUTPUT: &str = "SOAK-2";
const SECOND_SIZE: (i32, i32) = (2560, 1440);
const SETTLE: Duration = Duration::from_secs(6);
const WINDOW_PATIENCE: Duration = Duration::from_secs(20);
const LOCK_PATIENCE: Duration = Duration::from_secs(30);
const UPKEEP_EVERY: Duration = Duration::from_millis(200);
const STALL: Duration = Duration::from_secs(5);
const HANG: Duration = Duration::from_secs(30);
const PRESSURE: Duration = Duration::from_secs(45);
const PRESSURE_CHUNK: usize = 128 << 20;
const AVAILABLE_FLOOR_PERCENT: u64 = 12;
const CGROUP_CEILING_PERCENT: u64 = 80;
const FILES_CRASH_EVERY: u64 = 12;
const SETTINGS_CRASH_EVERY: u64 = 3;
const SHELL_CRASH_EVERY: u64 = 12;
const MIN_FIT_POINTS: usize = 6;
const MIN_FIT_SPAN_HOURS: f64 = 0.4;
const RSS_GROWTH_KB_PER_HOUR: f64 = 8.0 * 1024.0;
const FD_GROWTH_PER_HOUR: f64 = 2.0;
const THREAD_GROWTH_PER_HOUR: f64 = 1.0;
const FRAME_SLOWDOWN: f64 = 2.0;
const KEY_ESC: u32 = 1;
const KEY_W: u32 = 17;
const KEY_A: u32 = 30;
const KEY_N: u32 = 49;
const KEY_ENTER: u32 = 28;
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTMETA: u32 = 125;
const BUTTON_LEFT: u32 = 0x110;
const FILES: &str = "bande-files";
const SETTINGS: &str = "bande-settings";
const WATCHED: [&str; 6] = [
    "bande-shell",
    FILES,
    SETTINGS,
    "bande-notify",
    "bande-config",
    "bande-terminal",
];

struct Timed {
    renderer: Renderer,
    frames: Vec<Duration>,
}

impl FrameOutput for Timed {
    type Error = RenderError;

    fn submit(
        &mut self,
        compositor: &mut Compositor,
        output: &str,
    ) -> Result<Submission, RenderError> {
        let started = Instant::now();
        self.renderer.render_output(compositor, output)?;
        self.frames.push(started.elapsed());
        Ok(Submission::AtVblank)
    }
}

#[derive(Clone, Copy)]
struct Point3 {
    hours: f64,
    rss_kb: f64,
    fds: f64,
    threads: f64,
}

#[derive(Default)]
struct Samples {
    series: BTreeMap<(String, u32), Vec<Point3>>,
    frame_p99_ms: Vec<f64>,
}

struct Soak {
    server: Compositor,
    output: Timed,
    schedulers: Vec<FrameScheduler>,
    epoch: Instant,
    binaries: PathBuf,
    home: PathBuf,
    apps: BTreeMap<&'static str, Child>,
    services: Vec<Child>,
    lock_states: Receiver<bool>,
    locked: bool,
    last_upkeep: Instant,
    last_pump: Instant,
    unresponsive_since: Option<Instant>,
    expected_crashes: HashSet<u32>,
    findings: Vec<String>,
    samples: Samples,
    step_growth: BTreeMap<(String, String), (i64, i64)>,
    log: File,
    out: PathBuf,
}

fn proc_field(status: &str, name: &str) -> Option<f64> {
    status
        .lines()
        .find_map(|line| line.strip_prefix(name))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn children_of_self() -> Vec<(String, u32)> {
    let own = std::process::id();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| {
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let (name, rest) = stat.split_once(" (")?.1.rsplit_once(") ")?;
            let parent: u32 = rest.split_whitespace().nth(1)?.parse().ok()?;
            (parent == own && WATCHED.contains(&name)).then(|| (name.to_owned(), pid))
        })
        .collect()
}

fn long_lived_counts() -> BTreeMap<String, (i64, i64)> {
    let own = ("compositor".to_owned(), std::process::id());
    std::iter::once(own)
        .chain(children_of_self())
        .filter_map(|(name, pid)| {
            let point = point_of(pid, 0.0)?;
            Some((
                format!("{name}:{pid}"),
                (point.fds as i64, point.threads as i64),
            ))
        })
        .collect()
}

fn point_of(pid: u32, hours: f64) -> Option<Point3> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    Some(Point3 {
        hours,
        rss_kb: proc_field(&status, "VmRSS:")?,
        threads: proc_field(&status, "Threads:")?,
        fds: fs::read_dir(format!("/proc/{pid}/fd")).ok()?.count() as f64,
    })
}

fn slope(points: &[(f64, f64)]) -> f64 {
    let count = points.len() as f64;
    let mean_x = points.iter().map(|point| point.0).sum::<f64>() / count;
    let mean_y = points.iter().map(|point| point.1).sum::<f64>() / count;
    let spread: f64 = points.iter().map(|point| (point.0 - mean_x).powi(2)).sum();
    if spread == 0.0 {
        return 0.0;
    }
    points
        .iter()
        .map(|point| (point.0 - mean_x) * (point.1 - mean_y))
        .sum::<f64>()
        / spread
}

fn percentile(sorted: &[Duration], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((sorted.len() as f64 * fraction).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1].as_secs_f64() * 1000.0
}

impl Soak {
    fn start(binaries: PathBuf, home: PathBuf, out: PathBuf) -> Result<Self, Box<dyn Error>> {
        let mut server = Compositor::new(Size::from(OUTPUT))?;
        let (states, lock_states) = mpsc::channel();
        server.report_lock_states(states);
        server.set_lock_screen(&binaries.join("bande-lock"));
        server.start_session_client(&binaries.join("bande-shell"), SessionRole::Shell)?;
        let mut services = Vec::new();
        for service in ["bande-config", "bande-notify"] {
            services.push(Command::new(binaries.join(service)).spawn()?);
        }
        fs::create_dir_all(&out)?;
        let now = Instant::now();
        Ok(Self {
            server,
            output: Timed {
                renderer: Renderer::new()?,
                frames: Vec::new(),
            },
            schedulers: vec![FrameScheduler::new(FrameClock::new(
                Duration::ZERO,
                REFRESH,
            ))],
            epoch: now,
            binaries,
            home,
            apps: BTreeMap::new(),
            services,
            lock_states,
            locked: false,
            last_upkeep: now,
            last_pump: now,
            unresponsive_since: None,
            expected_crashes: HashSet::new(),
            findings: Vec::new(),
            samples: Samples::default(),
            step_growth: BTreeMap::new(),
            log: File::create(out.join("samples.tsv"))?,
            out,
        })
    }

    fn hours(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64() / 3600.0
    }

    fn finding(&mut self, text: String) {
        let line = format!("{:.2} h: {text}", self.hours());
        println!("{line}");
        self.findings.push(line);
    }

    fn pump(&mut self, duration: Duration) -> Result<(), Box<dyn Error>> {
        let started = Instant::now();
        while started.elapsed() < duration {
            self.server.dispatch()?;
            let epoch = self.epoch;
            for scheduler in &mut self.schedulers {
                scheduler.poll(|| epoch.elapsed(), &mut self.server, &mut self.output)?;
            }
            let gap = self.last_pump.elapsed();
            if gap > STALL {
                self.finding(format!("the compositor stalled for {gap:.1?}"));
            }
            self.last_pump = Instant::now();
            if self.last_upkeep.elapsed() >= UPKEEP_EVERY {
                self.last_upkeep = Instant::now();
                let upkeep = self.server.keep_session_clients_running();
                for (path, error) in upkeep.failures {
                    self.finding(format!("cannot run {}: {error}", path.display()));
                }
                self.watch_for_hangs();
            }
            while let Ok(locked) = self.lock_states.try_recv() {
                self.locked = locked;
            }
            thread::sleep(Duration::from_micros(500));
        }
        Ok(())
    }

    fn watch_for_hangs(&mut self) {
        if self.server.unresponsive_clients() == 0 {
            self.unresponsive_since = None;
            return;
        }
        let since = *self.unresponsive_since.get_or_insert_with(Instant::now);
        if since.elapsed() > HANG {
            self.finding(format!(
                "a client stopped answering for {:.1?}",
                since.elapsed()
            ));
            self.unresponsive_since = None;
        }
    }

    fn wait_for(
        &mut self,
        what: &str,
        patience: Duration,
        done: impl Fn(&Self) -> bool,
    ) -> Result<(), Box<dyn Error>> {
        let started = Instant::now();
        while !done(self) {
            if started.elapsed() > patience {
                return Err(format!("{what} did not happen within {patience:?}").into());
            }
            self.pump(Duration::from_millis(50))?;
        }
        Ok(())
    }

    fn tick(&self) -> u32 {
        u32::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u32::MAX)
    }

    fn tap(&mut self, codes: &[u32]) {
        for &code in codes {
            let time = self.tick();
            self.server.key(code, true, time);
        }
        for &code in codes.iter().rev() {
            let time = self.tick();
            self.server.key(code, false, time);
        }
    }

    fn open_app(&mut self, name: &'static str) -> Result<(), Box<dyn Error>> {
        let path = self.binaries.join(name);
        let (client_end, server_end) = std::os::unix::net::UnixStream::pair()?;
        self.server.insert_client(server_end)?;
        let before = self.server.windows().len();
        let child = Command::new(path)
            .env("HOME", &self.home)
            .env("WAYLAND_SOCKET", "0")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(client_end)))
            .stdout(Stdio::null())
            .spawn()?;
        self.apps.insert(name, child);
        self.wait_for("the window of an app", WINDOW_PATIENCE, |soak| {
            soak.server.windows().len() > before
        })
    }

    fn ensure_app(&mut self, name: &'static str) -> Result<(), Box<dyn Error>> {
        let exited = match self.apps.get_mut(name) {
            Some(child) => child.try_wait()?,
            None => {
                return self.open_app(name);
            }
        };
        if let Some(status) = exited {
            self.finding(format!("{name} exited by itself: {status}"));
            return self.open_app(name);
        }
        Ok(())
    }

    fn window_churn(&mut self) -> Result<(), Box<dyn Error>> {
        self.ensure_app(FILES)?;
        self.ensure_app(SETTINGS)?;
        self.pump(SETTLE)?;
        let before = self.server.windows().len();
        for _ in 0..4 {
            self.tap(&[KEY_LEFTCTRL, KEY_N]);
            self.pump(Duration::from_secs(2))?;
        }
        for step in 0..30u32 {
            let along = f64::from(step) * 18.0;
            let time = self.tick();
            self.server
                .move_pointer_to(Point::from((300.0 + along, 40.0 + along / 2.0)), time);
            self.pump(REFRESH * 2)?;
        }
        let time = self.tick();
        self.server.press_button(BUTTON_LEFT, true, time);
        let time = self.tick();
        self.server.press_button(BUTTON_LEFT, false, time);
        for _ in 0..4 {
            self.tap(&[KEY_LEFTCTRL, KEY_W]);
            self.pump(Duration::from_secs(1))?;
        }
        self.pump(SETTLE)?;
        let after = self.server.windows().len();
        if after > before {
            return Err(format!("{before} windows became {after} after closing them").into());
        }
        self.ensure_app(FILES)?;
        self.ensure_app(SETTINGS)
    }

    fn overlays(&mut self) -> Result<(), Box<dyn Error>> {
        for _ in 0..3 {
            self.tap(&[KEY_LEFTMETA, KEY_W]);
            self.pump(Duration::from_secs(2))?;
            self.tap(&[KEY_LEFTMETA, KEY_W]);
            self.pump(Duration::from_secs(2))?;
            self.tap(&[KEY_LEFTMETA]);
            self.pump(Duration::from_secs(2))?;
            self.tap(&[KEY_A]);
            self.pump(Duration::from_secs(1))?;
            self.tap(&[KEY_ESC]);
            self.pump(Duration::from_secs(1))?;
        }
        Ok(())
    }

    fn displays(&mut self) -> Result<(), Box<dyn Error>> {
        self.server.add_output(OutputConfig::virtual_display(
            SECOND_OUTPUT,
            Size::from(SECOND_SIZE),
        ));
        self.schedulers.push(FrameScheduler::for_output(
            FrameClock::new(self.epoch.elapsed(), REFRESH),
            SECOND_OUTPUT,
        ));
        self.pump(Duration::from_secs(8))?;
        for scale in [1.5, 2.0, 1.0] {
            if let Some(mut settings) = self.server.output_settings(SECOND_OUTPUT) {
                settings.scale = scale;
                self.server.configure_output(SECOND_OUTPUT, settings);
            }
            self.pump(Duration::from_secs(4))?;
        }
        self.schedulers
            .retain(|scheduler| scheduler.output() != Some(SECOND_OUTPUT));
        self.server.remove_output(SECOND_OUTPUT);
        self.pump(SETTLE)?;
        Ok(())
    }

    fn lock_and_unlock(&mut self) -> Result<(), Box<dyn Error>> {
        self.server.request_lock_screen();
        self.wait_for("the lock", LOCK_PATIENCE, |soak| soak.locked)?;
        self.pump(Duration::from_secs(4))?;
        self.tap(&[KEY_A]);
        self.pump(Duration::from_secs(1))?;
        self.tap(&[KEY_ENTER]);
        self.wait_for("the unlock", LOCK_PATIENCE, |soak| !soak.locked)?;
        self.pump(Duration::from_secs(2))?;
        Ok(())
    }

    fn memory_pressure(&mut self) -> Result<(), Box<dyn Error>> {
        let mut hog = Command::new(std::env::current_exe()?)
            .arg("--hog")
            .spawn()?;
        self.pump(PRESSURE)?;
        let _ = hog.kill();
        let _ = hog.wait();
        self.pump(SETTLE)?;
        Ok(())
    }

    fn crash_app(&mut self, name: &'static str, signal: Signal) -> Result<(), Box<dyn Error>> {
        let Some(mut child) = self.apps.remove(name) else {
            return Ok(());
        };
        let pid = child.id();
        self.expected_crashes.insert(pid);
        kill_process(Pid::from_raw(pid as i32).ok_or("no process")?, signal)?;
        self.pump(SETTLE)?;
        if child.try_wait()?.is_none() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{name} survived {signal:?}").into());
        }
        self.open_app(name)
    }

    fn crash_shell(&mut self) -> Result<(), Box<dyn Error>> {
        let Some(old) = children_of_self()
            .into_iter()
            .find(|(name, _)| name == "bande-shell")
            .map(|(_, pid)| pid)
        else {
            return Err("the shell is not running".into());
        };
        self.expected_crashes.insert(old);
        kill_process(Pid::from_raw(old as i32).ok_or("no process")?, Signal::ILL)?;
        self.wait_for("a new shell", WINDOW_PATIENCE, |_| {
            children_of_self()
                .iter()
                .any(|(name, pid)| name == "bande-shell" && *pid != old)
        })
    }

    fn crashes(&mut self, cycle: u64) -> Result<(), Box<dyn Error>> {
        if cycle.is_multiple_of(SETTINGS_CRASH_EVERY) {
            self.crash_app(SETTINGS, Signal::ABORT)?;
        }
        if cycle % FILES_CRASH_EVERY == FILES_CRASH_EVERY - 1 {
            self.crash_app(FILES, Signal::ILL)?;
        }
        if cycle % SHELL_CRASH_EVERY == SHELL_CRASH_EVERY / 2 {
            self.crash_shell()?;
        }
        Ok(())
    }

    fn step(
        &mut self,
        name: &str,
        run: impl FnOnce(&mut Self) -> Result<(), Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let before = long_lived_counts();
        let result = match run(self) {
            Ok(()) => Ok(()),
            Err(error) if error.downcast_ref::<RenderError>().is_some() => Err(error),
            Err(error) => {
                self.finding(format!("{name}: {error}"));
                Ok(())
            }
        };
        for (process, (fds, threads)) in long_lived_counts() {
            if let Some((old_fds, old_threads)) = before.get(&process) {
                let added = self
                    .step_growth
                    .entry((name.to_owned(), process))
                    .or_default();
                added.0 += fds - old_fds;
                added.1 += threads - old_threads;
            }
        }
        result
    }

    fn sample(&mut self) -> Result<(), Box<dyn Error>> {
        self.pump(SETTLE)?;
        let hours = self.hours();
        let own = ("compositor".to_owned(), std::process::id());
        let mut row = format!("{hours:.3}");
        for (name, pid) in std::iter::once(own).chain(children_of_self()) {
            let Some(point) = point_of(pid, hours) else {
                continue;
            };
            let _ = write!(
                row,
                "\t{name}:{pid} rss={} fds={} threads={}",
                point.rss_kb, point.fds, point.threads
            );
            self.samples
                .series
                .entry((name, pid))
                .or_default()
                .push(point);
        }
        let mut frames = std::mem::take(&mut self.output.frames);
        frames.sort();
        let p99 = percentile(&frames, 0.99);
        self.samples.frame_p99_ms.push(p99);
        let memory = self.output.renderer.memory_use();
        let _ = writeln!(
            row,
            "\tframes={} p50={:.1} p99={p99:.1} worst={:.1}\trenderer: {memory}",
            frames.len(),
            percentile(&frames, 0.5),
            percentile(&frames, 1.0),
        );
        self.log.write_all(row.as_bytes())?;
        self.log.flush()?;
        Ok(())
    }

    fn judge(&mut self) -> String {
        let mut table = String::from("process pid samples rss-MB fds threads | growth per hour\n");
        let series: Vec<_> = self
            .samples
            .series
            .iter()
            .map(|(key, points)| (key.clone(), points.clone()))
            .collect();
        for ((name, pid), points) in series {
            let (Some(first), Some(last)) = (points.first(), points.last()) else {
                continue;
            };
            let second_half = &points[points.len() / 2..];
            let span = last.hours - second_half[0].hours;
            let fit = |value: fn(&Point3) -> f64| {
                slope(
                    &second_half
                        .iter()
                        .map(|point| (point.hours, value(point)))
                        .collect::<Vec<_>>(),
                )
            };
            let (rss, fds, threads) = (
                fit(|point| point.rss_kb),
                fit(|point| point.fds),
                fit(|point| point.threads),
            );
            let _ = writeln!(
                table,
                "{name} {pid} {} {:.0}->{:.0} {}->{} {}->{} | {:.1} MB {fds:.1} fds {threads:.1} threads",
                points.len(),
                first.rss_kb / 1024.0,
                last.rss_kb / 1024.0,
                first.fds,
                last.fds,
                first.threads,
                last.threads,
                rss / 1024.0,
            );
            if second_half.len() < MIN_FIT_POINTS || span < MIN_FIT_SPAN_HOURS {
                continue;
            }
            if rss > RSS_GROWTH_KB_PER_HOUR {
                self.finding(format!("{name} memory grows {:.1} MB/h", rss / 1024.0));
            }
            if fds > FD_GROWTH_PER_HOUR {
                self.finding(format!("{name} file descriptors grow {fds:.1}/h"));
            }
            if threads > THREAD_GROWTH_PER_HOUR {
                self.finding(format!("{name} threads grow {threads:.1}/h"));
            }
        }
        let _ = writeln!(
            table,
            "\nfile descriptors and threads each step added, in total:"
        );
        for ((step, process), (fds, threads)) in &self.step_growth {
            if *fds > 0 || *threads > 0 {
                let _ = writeln!(table, "{step} {process} +{fds} fds +{threads} threads");
            }
        }
        let frames = &self.samples.frame_p99_ms;
        if frames.len() >= 2 * MIN_FIT_POINTS {
            let (early, late) = frames.split_at(frames.len() / 2);
            let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len() as f64;
            let (early, late) = (mean(early), mean(late));
            let _ = writeln!(
                table,
                "frame p99 ms: first half {early:.1}, second half {late:.1}"
            );
            if late > early * FRAME_SLOWDOWN {
                self.finding(format!(
                    "frames slowed from {early:.1} to {late:.1} ms at p99"
                ));
            }
        }
        table
    }

    fn finish(mut self) -> Result<bool, Box<dyn Error>> {
        let table = self.judge();
        let mut summary = format!(
            "{}\nran {:.2} h, {} findings\n\n{table}\n",
            if self.findings.is_empty() {
                "PASS"
            } else {
                "FAIL"
            },
            self.hours(),
            self.findings.len(),
        );
        for finding in &self.findings {
            let _ = writeln!(summary, "{finding}");
        }
        fs::write(self.out.join("summary.txt"), &summary)?;
        let expected: Vec<String> = self.expected_crashes.iter().map(u32::to_string).collect();
        fs::write(self.out.join("expected-crashes"), expected.join("\n"))?;
        print!("{summary}");
        for child in self.apps.values_mut().chain(&mut self.services) {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(self.findings.is_empty())
    }
}

fn cgroup_room_kb() -> Option<u64> {
    let cgroup = fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = cgroup.lines().find_map(|line| line.strip_prefix("0::"))?;
    let mut folder = Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'));
    let mut room: Option<u64> = None;
    while folder.starts_with("/sys/fs/cgroup") {
        let limit = fs::read_to_string(folder.join("memory.max"))
            .ok()
            .and_then(|limit| limit.trim().parse::<u64>().ok());
        let current = fs::read_to_string(folder.join("memory.current"))
            .ok()
            .and_then(|current| current.trim().parse::<u64>().ok());
        if let (Some(limit), Some(current)) = (limit, current) {
            let free = (limit * CGROUP_CEILING_PERCENT / 100).saturating_sub(current) / 1024;
            room = Some(room.map_or(free, |room| room.min(free)));
        }
        if !folder.pop() {
            break;
        }
    }
    room
}

fn room_for_more() -> bool {
    let Ok(meminfo) = fs::read_to_string("/proc/meminfo") else {
        return false;
    };
    let (Some(total), Some(available)) = (
        proc_field(&meminfo, "MemTotal:"),
        proc_field(&meminfo, "MemAvailable:"),
    ) else {
        return false;
    };
    let chunk_kb = (PRESSURE_CHUNK / 1024) as u64;
    available > total * AVAILABLE_FLOOR_PERCENT as f64 / 100.0
        && cgroup_room_kb().is_none_or(|room| room > chunk_kb)
}

fn hog() -> ! {
    let mut held = Vec::new();
    loop {
        if room_for_more() {
            held.push(vec![1u8; PRESSURE_CHUNK]);
        }
        thread::sleep(Duration::from_millis(500));
    }
}

fn run(hours: f64, out: PathBuf) -> Result<bool, Box<dyn Error>> {
    let binaries = std::env::current_exe()?
        .parent()
        .ok_or("no directory for the soak")?
        .to_path_buf();
    let home = tempfile::tempdir()?;
    let deadline = Duration::from_secs_f64(hours * 3600.0);
    let mut soak = Soak::start(binaries, home.path().to_path_buf(), out)?;
    soak.pump(SETTLE)?;
    let mut cycle = 0;
    while soak.epoch.elapsed() < deadline {
        soak.step("window churn", Soak::window_churn)?;
        soak.step("overview and launcher", Soak::overlays)?;
        soak.step("display changes", Soak::displays)?;
        soak.step("lock", Soak::lock_and_unlock)?;
        soak.step("memory pressure", Soak::memory_pressure)?;
        soak.step("crashes", |soak| soak.crashes(cycle))?;
        soak.sample()?;
        cycle += 1;
    }
    soak.finish()
}

fn main() -> ExitCode {
    let mut hours = 1.0;
    let mut out = PathBuf::from("soak-out");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--hog" => hog(),
            "--hours" => hours = args.next().and_then(|v| v.parse().ok()).unwrap_or(hours),
            "--out" => out = args.next().map_or(out, PathBuf::from),
            other => {
                eprintln!("usage: bande-soak [--hours H] [--out DIR]; unknown {other}");
                return ExitCode::FAILURE;
            }
        }
    }
    let bus = match bande_test_bus::SessionBus::start() {
        Ok(bus) => bus,
        Err(error) => {
            eprintln!("bande-soak: cannot start a private session bus: {error}");
            return ExitCode::FAILURE;
        }
    };
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
    }
    bande_log::init("bande-soak");
    match run(hours, out) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("bande-soak: {error}");
            ExitCode::from(2)
        }
    }
}
