use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use bande_compositor::{
    CLIPBOARD_DAEMON, Compositor, FrameClock, FrameScheduler, LOCK_SCREEN, REFRESH, Renderer,
    SCREENSHOT, SHELL, SessionRole,
};
use bande_xdg::find_executable;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use smithay::utils::{Clock, Monotonic, Point, Size};

const OUTPUT: (i32, i32) = (2928, 1818);
const SCALE: f64 = 1.5;
const START: Duration = Duration::from_secs(15);
const DEFAULT_SETTLE: u64 = 20;
const STEP: Duration = Duration::from_millis(400);
const DEFAULT_SECONDS: u64 = 60;
const BUDGET: f64 = 1.0;
const WINDOWS_OPEN: &str = "windows open";
const LIVE_BUDGETS: [(&str, f64); 2] = [("bande-monitor", 5.0), ("bande-compositor", 10.0)];
const SOFTWARE_RASTERISER: &str = "llvmpipe-";
const TICK_MS: f64 = 10.0;
const BUTTON_LEFT: u32 = 0x110;
const KEY_LEFTMETA: u32 = 125;
const KEY_H: u32 = 35;
const SESSION_CLIENTS: [(&str, SessionRole); 3] = [
    (CLIPBOARD_DAEMON, SessionRole::ClipboardDaemon),
    (SHELL, SessionRole::Shell),
    (SCREENSHOT, SessionRole::Shell),
];
const CONFIG: &str = "bande-config";
const CONFIG_BUS_NAME: &str = "org.bande.Config";
const SERVICES: [&str; 7] = [
    CONFIG,
    "bande-power",
    "bande-sound",
    "bande-tray",
    "bande-automount",
    "bande-notify",
    "xdg-desktop-portal-bande",
];
const APPS: [(&str, &str); 4] = [
    ("bande-files", "org.bande.Files"),
    ("bande-settings", "org.bande.Settings"),
    ("bande-terminal", "org.bande.Terminal"),
    ("bande-monitor", "org.bande.ActivityMonitor"),
];

struct Options {
    binaries: PathBuf,
    seconds: u64,
    settle: u64,
    json: Option<PathBuf>,
}

struct Session {
    server: Compositor,
    renderer: Renderer,
    scheduler: FrameScheduler,
    readiness: [OwnedFd; 2],
    children: Vec<Child>,
}

#[derive(Clone, Copy)]
struct ThreadSample {
    switches: u64,
    ticks: u64,
}

struct Snapshot {
    threads: HashMap<(u32, u32), ThreadSample>,
}

type Case = (&'static str, Vec<Row>);

struct Row {
    process: String,
    wakeups_per_second: f64,
    cpu_ms: f64,
    rss_mib: f64,
    threads: String,
}

fn main() -> ExitCode {
    let bus = match bande_test_bus::SessionBus::start() {
        Ok(bus) => bus,
        Err(error) => {
            eprintln!("bande-idle: cannot start a private session bus: {error}");
            return ExitCode::FAILURE;
        }
    };
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
    }
    bande_log::init("bande-idle");
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            tracing::error!("{error}");
            eprintln!("bande-idle: {error}");
            ExitCode::FAILURE
        }
    }
}

fn options() -> Result<Options, Box<dyn Error>> {
    let mut options = Options {
        binaries: std::env::current_exe()?
            .parent()
            .ok_or("no directory for the benchmark")?
            .to_path_buf(),
        seconds: DEFAULT_SECONDS,
        settle: DEFAULT_SETTLE,
        json: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--binaries" => options.binaries = PathBuf::from(value),
            "--seconds" => options.seconds = value.parse()?,
            "--settle" => options.settle = value.parse()?,
            "--json" => options.json = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown argument {flag}").into()),
        }
    }
    Ok(options)
}

fn run() -> Result<bool, Box<dyn Error>> {
    let options = options()?;
    let home = tempfile::tempdir()?;
    let mut session = Session::start(&options.binaries, home.path())?;
    let result = measure_cases(&mut session, &options);
    session.stop();
    let cases = result?;
    let mut within = true;
    let mut json = Vec::new();
    for (case, rows) in &cases {
        println!(
            "\n### Idle for {} s: {case}\n\n| Process | Wakeups/s | CPU ms | RSS MiB | Waking threads |\n| :--- | ---: | ---: | ---: | :--- |",
            options.seconds
        );
        for row in rows {
            let allowed = budget(case, &row.process);
            let over = row.wakeups_per_second > allowed;
            within &= !over;
            println!(
                "| {} | {:.2}{} | {:.0} | {:.1} | {} |",
                row.process,
                row.wakeups_per_second,
                if over { " over" } else { "" },
                row.cpu_ms,
                row.rss_mib,
                row.threads
            );
            json.push(format!(
                "    {{\"case\": \"{case}\", \"process\": \"{}\", \"wakeups_per_second\": {:.3}, \"cpu_ms\": {:.0}, \"rss_mib\": {:.1}, \"budget\": {:.3}}}",
                row.process, row.wakeups_per_second, row.cpu_ms, row.rss_mib, allowed
            ));
        }
    }
    if let Some(path) = &options.json {
        fs::write(path, format!("[\n{}\n]\n", json.join(",\n")))?;
    }
    if !within {
        println!("\nA process woke more often than its budget while nothing happened.");
    }
    Ok(within)
}

fn budget(case: &str, process: &str) -> f64 {
    LIVE_BUDGETS
        .iter()
        .find(|(name, _)| case == WINDOWS_OPEN && *name == process)
        .map_or(BUDGET, |&(_, budget)| budget)
}

fn measure_cases(session: &mut Session, options: &Options) -> Result<Vec<Case>, Box<dyn Error>> {
    let window = Duration::from_secs(options.seconds);
    let settle = Duration::from_secs(options.settle);
    session.run_for(settle)?;
    let mut cases = vec![(WINDOWS_OPEN, session.measure(window)?)];
    session.minimize_apps()?;
    session.run_for(settle)?;
    cases.push(("windows minimised", session.measure(window)?));
    session.server.request_lock_screen();
    session.run_for(settle)?;
    if session.server.lock_screen_pid().is_none() {
        return Err("the lock screen did not start".into());
    }
    cases.push(("screen locked", session.measure(window)?));
    Ok(cases)
}

fn beside(binaries: &Path, installed: &str) -> Result<PathBuf, Box<dyn Error>> {
    let name = Path::new(installed)
        .file_name()
        .ok_or_else(|| format!("{installed} names no file"))?;
    Ok(binaries.join(name))
}

fn now() -> Duration {
    Duration::from(Clock::<Monotonic>::new().now())
}

impl Session {
    fn start(binaries: &Path, home: &Path) -> Result<Self, Box<dyn Error>> {
        let mut server = Compositor::new(Size::from(OUTPUT))?;
        server.set_output_scale(SCALE);
        server.set_lock_screen(&beside(binaries, LOCK_SCREEN)?);
        let readiness = [server.poll_fd()?, server.fence_fd()?];
        let mut session = Self {
            server,
            renderer: Renderer::new()?,
            scheduler: FrameScheduler::new(FrameClock::new(now(), REFRESH)),
            readiness,
            children: Vec::new(),
        };
        for service in SERVICES {
            let child = Command::new(binaries.join(service))
                .env("HOME", home)
                .stdout(Stdio::null())
                .spawn()
                .map_err(|error| format!("cannot start {service}: {error}"))?;
            session.children.push(child);
            if service == CONFIG {
                session.wait_for_bus_name(CONFIG_BUS_NAME)?;
            }
        }
        for (client, role) in SESSION_CLIENTS {
            session
                .server
                .start_session_client(&beside(binaries, client)?, role)
                .map_err(|error| format!("cannot start {client}: {error}"))?;
        }
        session.run_for(START)?;
        for (app, app_id) in APPS {
            let built = binaries.join(app);
            let executable = if built.exists() {
                built
            } else {
                find_executable(app)
                    .ok_or_else(|| format!("{app} is neither built nor installed"))?
            };
            session.open_app(&executable, home)?;
            session.wait_for_window(app_id)?;
        }
        Ok(session)
    }

    fn open_app(&mut self, executable: &Path, home: &Path) -> Result<(), Box<dyn Error>> {
        let (client_end, server_end) = UnixStream::pair()?;
        self.server.insert_client(server_end)?;
        let child = Command::new(executable)
            .env("HOME", home)
            .env("WAYLAND_SOCKET", "0")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::from(OwnedFd::from(client_end)))
            .stdout(Stdio::null())
            .spawn()
            .map_err(|error| format!("cannot start {}: {error}", executable.display()))?;
        self.children.push(child);
        Ok(())
    }

    fn wait_for_window(&mut self, app_id: &str) -> Result<(), Box<dyn Error>> {
        let started = Instant::now();
        while !self.has_window(app_id) {
            if started.elapsed() > START {
                return Err(format!("{app_id} did not open a window").into());
            }
            self.run_for(STEP)?;
        }
        Ok(())
    }

    fn wait_for_bus_name(&mut self, name: &str) -> Result<(), Box<dyn Error>> {
        let started = Instant::now();
        loop {
            let owned = Command::new("gdbus")
                .args([
                    "call",
                    "--session",
                    "--dest",
                    "org.freedesktop.DBus",
                    "--object-path",
                    "/org/freedesktop/DBus",
                    "--method",
                    "org.freedesktop.DBus.NameHasOwner",
                    name,
                ])
                .output()?;
            if String::from_utf8_lossy(&owned.stdout).contains("true") {
                return Ok(());
            }
            if started.elapsed() > START {
                return Err(format!("{name} did not appear on the session bus").into());
            }
            self.run_for(STEP)?;
        }
    }

    fn has_window(&self, app_id: &str) -> bool {
        self.server
            .windows()
            .iter()
            .any(|window| window.app_id() == app_id)
    }

    fn minimize_apps(&mut self) -> Result<(), Box<dyn Error>> {
        for (_, app_id) in APPS {
            let geometry = self
                .server
                .windows()
                .iter()
                .find(|window| window.app_id() == app_id)
                .and_then(|window| window.geometry())
                .ok_or_else(|| format!("{app_id} has no window"))?;
            let title = Point::from((
                f64::from(geometry.loc.x + geometry.size.w / 2),
                f64::from(geometry.loc.y + 12),
            ));
            let time = now().as_millis() as u32;
            self.server.move_pointer_to(title, time);
            self.server.press_button(BUTTON_LEFT, true, time);
            self.server.press_button(BUTTON_LEFT, false, time);
            self.run_for(STEP)?;
            self.server.key(KEY_LEFTMETA, true, time);
            self.server.key(KEY_H, true, time);
            self.server.key(KEY_H, false, time);
            self.server.key(KEY_LEFTMETA, false, time);
            self.run_for(STEP)?;
        }
        let shown: Vec<String> = self
            .server
            .windows()
            .iter()
            .filter(|window| !window.is_minimized())
            .map(|window| window.app_id())
            .collect();
        if shown.is_empty() {
            Ok(())
        } else {
            Err(format!("still shown after minimising: {shown:?}").into())
        }
    }

    fn run_for(&mut self, duration: Duration) -> Result<(), Box<dyn Error>> {
        let until = now() + duration;
        loop {
            self.server.dispatch()?;
            let upkeep = self.server.keep_session_clients_running();
            for (executable, error) in &upkeep.failures {
                tracing::warn!("cannot keep {} running: {error}", executable.display());
            }
            let snap = self.server.snap_tick(now());
            let frame = self
                .scheduler
                .poll(now, &mut self.server, &mut self.renderer)?;
            let idle = self.server.idle_tick(now());
            let watchdog = self.server.watchdog_tick(now());
            let wake_at = [snap, frame.wake_at, idle, watchdog, Some(until)]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(until);
            let current = now();
            if current >= until {
                return Ok(());
            }
            let wait = upkeep
                .retry_in
                .map_or(wake_at.saturating_sub(current), |retry| {
                    retry.min(wake_at.saturating_sub(current))
                });
            let timeout = Timespec {
                tv_sec: i64::try_from(wait.as_secs())?,
                tv_nsec: i64::from(wait.subsec_nanos()),
            };
            let mut fds = self
                .readiness
                .iter()
                .map(|fd| PollFd::new(fd, PollFlags::IN))
                .collect::<Vec<_>>();
            match poll(&mut fds, Some(&timeout)) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn measure(&mut self, window: Duration) -> Result<Vec<Row>, Box<dyn Error>> {
        let before = Snapshot::take()?;
        let started = Instant::now();
        self.run_for(window)?;
        let after = Snapshot::take()?;
        let seconds = started.elapsed().as_secs_f64();
        self.ensure_running()?;
        rows(&before, &after, seconds)
    }

    fn ensure_running(&mut self) -> Result<(), Box<dyn Error>> {
        for child in &mut self.children {
            if let Some(status) = child.try_wait()? {
                return Err(format!("process {} exited with {status}", child.id()).into());
            }
        }
        Ok(())
    }

    fn stop(&mut self) {
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Snapshot {
    fn take() -> Result<Self, Box<dyn Error>> {
        let mut threads = HashMap::new();
        for pid in session_processes()? {
            let Ok(tasks) = fs::read_dir(format!("/proc/{pid}/task")) else {
                continue;
            };
            for task in tasks.flatten() {
                let Some(tid) = task.file_name().to_str().and_then(|name| name.parse().ok()) else {
                    continue;
                };
                if let Some(sample) = thread_sample(pid, tid) {
                    threads.insert((pid, tid), sample);
                }
            }
        }
        Ok(Self { threads })
    }
}

fn thread_sample(pid: u32, tid: u32) -> Option<ThreadSample> {
    let status = fs::read_to_string(format!("/proc/{pid}/task/{tid}/status")).ok()?;
    let field = |name: &str| -> Option<u64> {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|value| value.trim().parse().ok())
    };
    let switches = field("voluntary_ctxt_switches:")? + field("nonvoluntary_ctxt_switches:")?;
    let stat = fs::read_to_string(format!("/proc/{pid}/task/{tid}/stat")).ok()?;
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let ticks = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
    Some(ThreadSample { switches, ticks })
}

fn session_processes() -> Result<Vec<u32>, Box<dyn Error>> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for entry in fs::read_dir("/proc")?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        else {
            continue;
        };
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let parent = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().nth(1))
            .and_then(|parent| parent.parse().ok());
        if let Some(parent) = parent {
            children.entry(parent).or_default().push(pid);
        }
    }
    let mut found = vec![std::process::id()];
    let mut index = 0;
    while let Some(&pid) = found.get(index) {
        found.extend(children.get(&pid).into_iter().flatten());
        index += 1;
    }
    Ok(found)
}

fn process_name(pid: u32) -> String {
    if pid == std::process::id() {
        return "bande-compositor".to_owned();
    }
    fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .and_then(|cmdline| {
            let first = cmdline.split(|&byte| byte == 0).next()?;
            let path = Path::new(std::str::from_utf8(first).ok()?);
            Some(path.file_name()?.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| format!("pid {pid}"))
}

fn thread_name(pid: u32, tid: u32) -> String {
    fs::read_to_string(format!("/proc/{pid}/task/{tid}/comm"))
        .map(|name| name.trim().to_owned())
        .unwrap_or_else(|_| tid.to_string())
}

fn resident_mib(pid: u32) -> f64 {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("VmRSS:"))
                .and_then(|value| value.trim().strip_suffix("kB"))
                .and_then(|kib| kib.trim().parse::<f64>().ok())
        })
        .map_or(0.0, |kib| kib / 1024.0)
}

fn rows(before: &Snapshot, after: &Snapshot, seconds: f64) -> Result<Vec<Row>, Box<dyn Error>> {
    let pids: HashSet<u32> = after.threads.keys().map(|&(pid, _)| pid).collect();
    let mut pids: Vec<u32> = pids.into_iter().collect();
    pids.sort();
    let mut rows = Vec::new();
    for pid in pids {
        let mut threads: Vec<(String, u64, u64)> = Vec::new();
        for (&(owner, tid), sample) in &after.threads {
            if owner != pid {
                continue;
            }
            let earlier = before.threads.get(&(owner, tid));
            let woke = sample.switches
                - earlier.map_or(0, |earlier| earlier.switches.min(sample.switches));
            let ticks = sample.ticks - earlier.map_or(0, |earlier| earlier.ticks.min(sample.ticks));
            if woke > 0 || ticks > 0 {
                threads.push((thread_name(pid, tid), woke, ticks));
            }
        }
        threads.sort_by_key(|&(_, woke, ticks)| std::cmp::Reverse(woke + ticks));
        let wakeups: u64 = threads
            .iter()
            .filter(|(name, _, _)| !name.starts_with(SOFTWARE_RASTERISER))
            .map(|&(_, woke, _)| woke)
            .sum();
        let ticks: u64 = threads.iter().map(|&(_, _, ticks)| ticks).sum();
        rows.push(Row {
            process: process_name(pid),
            wakeups_per_second: wakeups as f64 / seconds,
            cpu_ms: ticks as f64 * TICK_MS,
            rss_mib: resident_mib(pid),
            threads: threads
                .iter()
                .take(4)
                .map(|(name, woke, ticks)| {
                    format!("{name} {woke}× {:.0} ms", *ticks as f64 * TICK_MS)
                })
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    if rows.is_empty() {
        return Err("no session processes to measure".into());
    }
    Ok(rows)
}
