use std::error::Error;
use std::fs;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use bande_compositor::{Compositor, FrameClock, FrameScheduler, REFRESH, Renderer};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use smithay::utils::{Clock, Monotonic, Point, Size};

const OUTPUT: (i32, i32) = (2928, 1818);
const SCALE: f64 = 1.5;
const OPEN_WITHIN: Duration = Duration::from_secs(20);
const STEP: Duration = Duration::from_millis(100);
const BUTTON_LEFT: u32 = 0x110;
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_W: u32 = 17;
const CLOSE_KEYS: [&[u32]; 2] = [
    &[KEY_LEFTCTRL, KEY_W],
    &[KEY_LEFTSHIFT, KEY_LEFTCTRL, KEY_W],
];
const SOFTWARE_RASTERISER: &str = "llvmpipe-";
const USAGE: &str = "usage: bande-background [--settle seconds] [--seconds seconds] [--cycles n] <binary>=<app id>[,background]...";

struct Options {
    settle: Duration,
    seconds: Duration,
    cycles: u32,
    apps: Vec<App>,
}

struct App {
    binary: String,
    app_id: String,
    background: bool,
}

struct Session {
    server: Compositor,
    renderer: Renderer,
    scheduler: FrameScheduler,
    readiness: [OwnedFd; 2],
}

struct Sample {
    anonymous_mib: f64,
    resident_mib: f64,
    wakeups_per_second: f64,
}

fn main() -> ExitCode {
    let bus = match bande_test_bus::SessionBus::start() {
        Ok(bus) => bus,
        Err(error) => {
            eprintln!("bande-background: cannot start a private session bus: {error}");
            return ExitCode::FAILURE;
        }
    };
    let home = match tempfile::tempdir() {
        Ok(home) => home,
        Err(error) => {
            eprintln!("bande-background: cannot make a home folder: {error}");
            return ExitCode::FAILURE;
        }
    };
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
        std::env::set_var("HOME", home.path());
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("XDG_DATA_HOME");
        std::env::remove_var("XDG_STATE_HOME");
        std::env::remove_var("XDG_CACHE_HOME");
    }
    bande_log::init("bande-background");
    match options().and_then(|options| run(&options)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bande-background: {error}");
            ExitCode::FAILURE
        }
    }
}

fn options() -> Result<Options, Box<dyn Error>> {
    let mut options = Options {
        settle: Duration::from_secs(5),
        seconds: Duration::from_secs(10),
        cycles: 2,
        apps: Vec::new(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--settle" => options.settle = Duration::from_secs(args.next().ok_or(USAGE)?.parse()?),
            "--seconds" => {
                options.seconds = Duration::from_secs(args.next().ok_or(USAGE)?.parse()?);
            }
            "--cycles" => options.cycles = args.next().ok_or(USAGE)?.parse()?,
            _ => {
                let (binary, rest) = arg.split_once('=').ok_or(USAGE)?;
                let (app_id, background) = match rest.strip_suffix(",background") {
                    Some(app_id) => (app_id, true),
                    None => (rest, false),
                };
                options.apps.push(App {
                    binary: binary.to_owned(),
                    app_id: app_id.to_owned(),
                    background,
                });
            }
        }
    }
    if options.apps.is_empty() {
        return Err(USAGE.into());
    }
    Ok(options)
}

fn run(options: &Options) -> Result<(), Box<dyn Error>> {
    println!(
        "| App | State | Anonymous MiB | RSS MiB | Wake-ups/s | Window shown after ms |\n| :--- | :--- | ---: | ---: | ---: | ---: |"
    );
    for app in &options.apps {
        let mut session = Session::start()?;
        let mut children = Vec::new();
        let result = session.cycle(app, options, &mut children);
        for child in &mut children {
            let _ = child.kill();
            let _ = child.wait();
        }
        result?;
    }
    Ok(())
}

fn now() -> Duration {
    Duration::from(Clock::<Monotonic>::new().now())
}

fn row(app: &App, state: &str, sample: Option<&Sample>, shown: Option<Duration>) {
    let numbers = sample.map_or_else(
        || "- | - | -".to_owned(),
        |sample| {
            format!(
                "{:.1} | {:.1} | {:.2}",
                sample.anonymous_mib, sample.resident_mib, sample.wakeups_per_second
            )
        },
    );
    let shown = shown.map_or_else(|| "-".to_owned(), |shown| shown.as_millis().to_string());
    println!("| {} | {state} | {numbers} | {shown} |", app.binary);
}

impl Session {
    fn start() -> Result<Self, Box<dyn Error>> {
        let mut server = Compositor::new(Size::from(OUTPUT))?;
        server.set_output_scale(SCALE);
        let readiness = [server.poll_fd()?, server.fence_fd()?];
        Ok(Self {
            server,
            renderer: Renderer::new()?,
            scheduler: FrameScheduler::new(FrameClock::new(now(), REFRESH)),
            readiness,
        })
    }

    fn cycle(
        &mut self,
        app: &App,
        options: &Options,
        children: &mut Vec<Child>,
    ) -> Result<(), Box<dyn Error>> {
        let launch = |session: &mut Self, children: &mut Vec<Child>, extra: Option<&str>| {
            session.launch(&app.binary, extra).map(|child| {
                let pid = child.id();
                children.push(child);
                pid
            })
        };
        let pid = if app.background {
            let pid = launch(self, children, Some("--background"))?;
            self.run_for(options.settle)?;
            let cold = self.sample(pid, options.seconds)?;
            row(app, "background, cold", Some(&cold), None);
            pid
        } else {
            0
        };
        let started = Instant::now();
        let launched = launch(self, children, None)?;
        let pid = if app.background { pid } else { launched };
        let shown = self.wait_for_window(&app.app_id, started)?;
        self.run_for(options.settle)?;
        let open = self.sample(pid, options.seconds)?;
        row(app, "window open", Some(&open), Some(shown));
        for cycle in 1..=options.cycles {
            self.close_window(&app.app_id)?;
            self.run_for(options.settle)?;
            let closed = self.sample(pid, options.seconds)?;
            row(app, &format!("closed ({cycle})"), Some(&closed), None);
            let started = Instant::now();
            launch(self, children, None)?;
            let shown = self.wait_for_window(&app.app_id, started)?;
            row(app, &format!("reopened ({cycle})"), None, Some(shown));
            self.run_for(options.settle)?;
        }
        Ok(())
    }

    fn launch(&mut self, binary: &str, extra: Option<&str>) -> Result<Child, Box<dyn Error>> {
        let (client_end, server_end) = UnixStream::pair()?;
        self.server.insert_client(server_end)?;
        Command::new(binary)
            .args(extra)
            .env("WAYLAND_SOCKET", "0")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::from(OwnedFd::from(client_end)))
            .stdout(Stdio::null())
            .spawn()
            .map_err(|error| format!("cannot start {binary}: {error}").into())
    }

    fn shown(&self, app_id: &str) -> Option<Point<f64, smithay::utils::Logical>> {
        self.server
            .windows()
            .iter()
            .filter(|window| window.app_id() == app_id)
            .find_map(|window| window.geometry())
            .map(|geometry| {
                Point::from((
                    f64::from(geometry.loc.x + geometry.size.w / 2),
                    f64::from(geometry.loc.y + 12),
                ))
            })
    }

    fn wait_for_window(
        &mut self,
        app_id: &str,
        started: Instant,
    ) -> Result<Duration, Box<dyn Error>> {
        while self.shown(app_id).is_none() {
            if started.elapsed() > OPEN_WITHIN {
                return Err(format!("{app_id} did not show a window").into());
            }
            self.run_for(Duration::from_millis(1))?;
        }
        Ok(started.elapsed())
    }

    fn close_window(&mut self, app_id: &str) -> Result<(), Box<dyn Error>> {
        let title = self.shown(app_id).ok_or("no window to close")?;
        let time = now().as_millis() as u32;
        self.server.move_pointer_to(title, time);
        self.server.press_button(BUTTON_LEFT, true, time);
        self.server.press_button(BUTTON_LEFT, false, time);
        self.run_for(STEP)?;
        for keys in CLOSE_KEYS {
            for &key in keys {
                self.server.key(key, true, time);
            }
            for &key in keys.iter().rev() {
                self.server.key(key, false, time);
            }
            let started = Instant::now();
            while started.elapsed() < OPEN_WITHIN / 10 {
                if self.shown(app_id).is_none() {
                    return Ok(());
                }
                self.run_for(STEP)?;
            }
        }
        Err(format!("{app_id} kept its window").into())
    }

    fn sample(&mut self, pid: u32, seconds: Duration) -> Result<Sample, Box<dyn Error>> {
        let before = switches(pid)?;
        self.run_for(seconds)?;
        let woke = switches(pid)?.saturating_sub(before);
        let rollup = fs::read_to_string(format!("/proc/{pid}/smaps_rollup"))?;
        let field = |name: &str| -> f64 {
            rollup
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .and_then(|value| value.trim().strip_suffix("kB"))
                .and_then(|kib| kib.trim().parse::<f64>().ok())
                .map_or(0.0, |kib| kib / 1024.0)
        };
        Ok(Sample {
            anonymous_mib: field("Anonymous:"),
            resident_mib: field("Rss:"),
            wakeups_per_second: woke as f64 / seconds.as_secs_f64(),
        })
    }

    fn run_for(&mut self, duration: Duration) -> Result<(), Box<dyn Error>> {
        let until = now() + duration;
        loop {
            self.server.dispatch()?;
            let frame = self
                .scheduler
                .poll(now, &mut self.server, &mut self.renderer)?;
            let current = now();
            if current >= until {
                return Ok(());
            }
            let wake_at = frame.wake_at.map_or(until, |wake_at| wake_at.min(until));
            let wait = wake_at.saturating_sub(current);
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
}

fn switches(pid: u32) -> Result<u64, Box<dyn Error>> {
    let mut total = 0;
    for task in fs::read_dir(format!("/proc/{pid}/task"))?.flatten() {
        let path = task.path();
        let name = fs::read_to_string(path.join("comm")).unwrap_or_default();
        if name.starts_with(SOFTWARE_RASTERISER) {
            continue;
        }
        let Ok(status) = fs::read_to_string(path.join("status")) else {
            continue;
        };
        total += status
            .lines()
            .find_map(|line| line.strip_prefix("voluntary_ctxt_switches:"))
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(0);
    }
    Ok(total)
}
