use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{
    Compositor, FrameClock, FrameOutput, FrameScheduler, FrameStats, REFRESH, RenderError,
    Renderer, Submission,
};
use bande_xdg::find_executable;
use smithay::backend::input::{Axis, AxisSource};
use smithay::input::pointer::AxisFrame;
use smithay::utils::{Point, Size};

const OUTPUT: (i32, i32) = (2928, 1818);
const SCALE: f64 = 1.5;
const SETTLE: Duration = Duration::from_secs(6);
const AFTERMATH: Duration = Duration::from_millis(600);
const STEPS: u32 = 120;
const FOLDER_ENTRIES: usize = 400;
const BUTTON_LEFT: u32 = 0x110;
const BUTTON_RIGHT: u32 = 0x111;
const KEY_ESC: u32 = 1;
const KEY_A: u32 = 30;
const KEY_SPACE: u32 = 57;
const KEY_LEFTALT: u32 = 56;
const KEY_LEFTMETA: u32 = 125;
const KEY_UP: u32 = 103;
const TICK_MS: f64 = 10.0;
const FILES: &str = "org.bande.Files";
const TERMINAL: &str = "org.bande.Terminal";
const TERMINAL_PROGRAM: &str = "bande-terminal";

type Scenario = fn(&mut Bench) -> Result<(), Box<dyn Error>>;

struct Timed {
    renderer: Renderer,
    frames: Vec<(Duration, FrameStats)>,
}

impl FrameOutput for Timed {
    type Error = RenderError;

    fn submit(
        &mut self,
        compositor: &mut Compositor,
        output: &str,
    ) -> Result<Submission, RenderError> {
        let started = Instant::now();
        let stats = self.renderer.render_output(compositor, output)?;
        self.frames.push((started.elapsed(), stats));
        Ok(Submission::AtVblank)
    }
}

struct Bench {
    server: Compositor,
    output: Timed,
    scheduler: FrameScheduler,
    epoch: Instant,
    clients: Vec<(String, Child)>,
    list_frames: bool,
}

struct Row {
    name: &'static str,
    frames: usize,
    p50: f64,
    p99: f64,
    compositor_cpu: f64,
    clients_cpu: f64,
    busy: f64,
    repainted: f64,
    clients: String,
}

fn main() -> ExitCode {
    let bus = match bande_test_bus::SessionBus::start() {
        Ok(bus) => bus,
        Err(error) => {
            eprintln!("bande-session-bench: cannot start a private session bus: {error}");
            return ExitCode::FAILURE;
        }
    };
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
    }
    bande_log::init("bande-session-bench");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let binaries = std::env::current_exe()?
        .parent()
        .ok_or("no directory for the benchmark")?
        .to_path_buf();
    let mut only = Vec::new();
    let mut snapshots = None;
    let mut full_effects = false;
    let mut list_frames = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--full-effects" => full_effects = true,
            "--frames" => list_frames = true,
            "--snapshots" => {
                snapshots = Some(PathBuf::from(
                    args.next().ok_or("--snapshots needs a folder")?,
                ));
            }
            _ => only.push(arg),
        }
    }
    let home = tempfile::tempdir()?;
    let folder = home.path().join("Many");
    fs::create_dir_all(&folder)?;
    for entry in 0..FOLDER_ENTRIES {
        fs::write(folder.join(format!("Document {entry:03}.txt")), b"text")?;
    }
    let mut bench = Bench::start(&binaries, home.path(), &folder, full_effects)?;
    bench.list_frames = list_frames;
    let adapter = bench.output.renderer.device_info().name.clone();
    let scenarios: [(&'static str, Scenario); 9] = [
        ("idle", |bench| bench.wait(STEPS)),
        ("pointer over a window", Bench::hover),
        ("dragging a window", Bench::drag),
        ("typing in the terminal", Bench::type_text),
        ("scrolling a list", Bench::scroll),
        ("opening the launcher", Bench::launcher),
        ("opening the overview", Bench::overview),
        ("opening a menu", Bench::menu),
        ("a notification banner", Bench::notification),
    ];
    println!(
        "### Session on {adapter}, {}×{} at {SCALE}×\n\nHost load average: {}\n",
        OUTPUT.0,
        OUTPUT.1,
        fs::read_to_string("/proc/loadavg")
            .unwrap_or_default()
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!(
        "| Action | Frames | Frame p50 ms | Frame p99 ms | Compositor CPU per frame ms | Clients CPU per frame ms | CPU busy (cores) | Repainted Mpx per frame |"
    );
    println!("| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    for (name, scenario) in scenarios {
        if !only.is_empty() && !only.iter().any(|wanted| name.contains(wanted.as_str())) {
            continue;
        }
        let row = bench.measure(name, scenario)?;
        println!(
            "| {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.2} | {:.2} |",
            row.name,
            row.frames,
            row.p50,
            row.p99,
            row.compositor_cpu,
            row.clients_cpu,
            row.busy,
            row.repainted
        );
        eprintln!("{}: {}", row.name, row.clients);
        if let Some(folder) = &snapshots {
            bench.save_snapshot(&folder.join(format!("{}.ppm", name.replace(' ', "-"))))?;
        }
    }
    bench.stop();
    Ok(())
}

impl Bench {
    fn start(
        binaries: &Path,
        home: &Path,
        folder: &Path,
        full_effects: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let mut server = Compositor::new(Size::from(OUTPUT))?;
        server.set_output_scale(SCALE);
        let mut bench = Self {
            server,
            output: Timed {
                renderer: Renderer::new()?,
                frames: Vec::new(),
            },
            scheduler: FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH)),
            epoch: Instant::now(),
            clients: Vec::new(),
            list_frames: false,
        };
        let notify = Command::new(binaries.join("bande-notify")).spawn()?;
        bench.clients.push(("bande-notify".to_owned(), notify));
        let shell = "bande-shell";
        let child = bench.server.spawn_shell_client(&binaries.join(shell))?;
        bench.clients.push((shell.to_owned(), child));
        if full_effects {
            bench.output.renderer.set_reduced_effects(false);
        }
        bench.run_for(SETTLE)?;
        bench.open_app(
            Command::new(binaries.join("bande-files"))
                .arg(folder)
                .env("HOME", home),
            "bande-files",
        )?;
        let built = binaries.join(TERMINAL_PROGRAM);
        let terminal = if built.exists() {
            built
        } else {
            find_executable(TERMINAL_PROGRAM)
                .ok_or("bande-terminal is neither built nor installed")?
        };
        bench.open_app(Command::new(terminal).env("HOME", home), TERMINAL_PROGRAM)?;
        bench.run_for(SETTLE)?;
        bench.move_aside(FILES)?;
        Ok(bench)
    }

    fn open_app(&mut self, command: &mut Command, name: &str) -> Result<(), Box<dyn Error>> {
        let (client_end, server_end) = std::os::unix::net::UnixStream::pair()?;
        self.server.insert_client(server_end)?;
        let child = command
            .env("WAYLAND_SOCKET", "0")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(client_end)))
            .stdout(Stdio::null())
            .spawn()?;
        self.clients.push((name.to_owned(), child));
        Ok(())
    }

    fn stop(&mut self) {
        for (_, child) in &mut self.clients {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn run_for(&mut self, duration: Duration) -> Result<(), Box<dyn Error>> {
        let started = Instant::now();
        while started.elapsed() < duration {
            self.server.dispatch()?;
            let epoch = self.epoch;
            self.scheduler
                .poll(|| epoch.elapsed(), &mut self.server, &mut self.output)?;
            thread::sleep(Duration::from_micros(500));
        }
        Ok(())
    }

    fn wait(&mut self, steps: u32) -> Result<(), Box<dyn Error>> {
        self.run_for(REFRESH * steps)
    }

    fn tick(&self) -> u32 {
        u32::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u32::MAX)
    }

    fn point(&mut self, x: f64, y: f64) {
        let time = self.tick();
        self.server.move_pointer_to(Point::from((x, y)), time);
    }

    fn key(&mut self, code: u32, pressed: bool) {
        let time = self.tick();
        self.server.key(code, pressed, time);
    }

    fn tap(&mut self, codes: &[u32]) {
        codes.iter().for_each(|&code| self.key(code, true));
        codes.iter().rev().for_each(|&code| self.key(code, false));
    }

    fn window(&self, app_id: &str) -> Result<(f64, f64, f64, f64), Box<dyn Error>> {
        let geometry = self
            .server
            .windows()
            .iter()
            .find(|window| window.app_id() == app_id)
            .and_then(|window| window.geometry())
            .ok_or_else(|| format!("{app_id} has no window"))?;
        Ok((
            f64::from(geometry.loc.x),
            f64::from(geometry.loc.y),
            f64::from(geometry.size.w),
            f64::from(geometry.size.h),
        ))
    }

    fn raise(&mut self, app_id: &str) -> Result<(f64, f64, f64, f64), Box<dyn Error>> {
        let (x, y, width, height) = self.window(app_id)?;
        self.point(x + width / 2.0, y + 24.0);
        let time = self.tick();
        self.server.press_button(BUTTON_LEFT, true, time);
        let time = self.tick();
        self.server.press_button(BUTTON_LEFT, false, time);
        self.run_for(AFTERMATH)?;
        Ok((x, y, width, height))
    }

    fn move_aside(&mut self, app_id: &str) -> Result<(), Box<dyn Error>> {
        let (x, y, width, _) = self.window(app_id)?;
        let (start_x, start_y) = (x + width / 2.0, y + 24.0);
        self.point(start_x, start_y);
        let time = self.tick();
        self.server.press_button(BUTTON_LEFT, true, time);
        for step in 1..=20 {
            self.point(
                start_x - f64::from(step) * 20.0,
                start_y + f64::from(step) * 12.5,
            );
            self.wait(1)?;
        }
        let time = self.tick();
        self.server.press_button(BUTTON_LEFT, false, time);
        self.run_for(AFTERMATH)?;
        Ok(())
    }

    fn hover(&mut self) -> Result<(), Box<dyn Error>> {
        let (x, y, width, height) = self.raise(FILES)?;
        for step in 0..STEPS {
            let along = f64::from(step) / f64::from(STEPS);
            self.point(
                x + width * (0.1 + 0.8 * along),
                y + height * (0.3 + 0.4 * along),
            );
            self.wait(1)?;
        }
        Ok(())
    }

    fn drag(&mut self) -> Result<(), Box<dyn Error>> {
        let (x, y, width, _) = self.raise(FILES)?;
        let (start_x, start_y) = (x + width / 2.0, y + 24.0);
        self.point(start_x, start_y);
        let time = self.tick();
        self.server.press_button(BUTTON_LEFT, true, time);
        for step in 0..STEPS {
            let out = f64::from(step.min(STEPS - step)) * 6.0;
            self.point(start_x + out, start_y + out / 2.0);
            self.wait(1)?;
            if step == STEPS / 2 && self.window(FILES)?.0 == x {
                return Err("Files did not move when dragged by its title bar".into());
            }
        }
        let time = self.tick();
        self.server.press_button(BUTTON_LEFT, false, time);
        Ok(())
    }

    fn type_text(&mut self) -> Result<(), Box<dyn Error>> {
        let (x, y, width, height) = self.raise(TERMINAL)?;
        self.point(x + width / 2.0, y + height / 2.0);
        for _ in 0..STEPS / 2 {
            self.tap(&[KEY_A]);
            self.wait(2)?;
        }
        Ok(())
    }

    fn scroll(&mut self) -> Result<(), Box<dyn Error>> {
        let (x, y, width, height) = self.raise(FILES)?;
        self.point(x + width * 0.25, y + height * 0.6);
        for step in 0..STEPS {
            let direction = if step < STEPS / 2 { 1.0 } else { -1.0 };
            let time = self.tick();
            self.server.scroll(
                AxisFrame::new(time)
                    .source(AxisSource::Wheel)
                    .value(Axis::Vertical, 15.0 * direction)
                    .v120(Axis::Vertical, (120.0 * direction) as i32),
            );
            self.wait(1)?;
        }
        Ok(())
    }

    fn launcher(&mut self) -> Result<(), Box<dyn Error>> {
        for _ in 0..2 {
            self.tap(&[KEY_LEFTMETA, KEY_SPACE]);
            self.wait(STEPS / 2)?;
            self.tap(&[KEY_ESC]);
            self.wait(STEPS / 2)?;
        }
        Ok(())
    }

    fn menu(&mut self) -> Result<(), Box<dyn Error>> {
        let (x, y, width, height) = self.raise(FILES)?;
        self.point(x + width * 0.6, y + height * 0.5);
        for _ in 0..2 {
            let time = self.tick();
            self.server.press_button(BUTTON_RIGHT, true, time);
            let time = self.tick();
            self.server.press_button(BUTTON_RIGHT, false, time);
            self.wait(STEPS / 2)?;
            self.tap(&[KEY_ESC]);
            self.wait(STEPS / 2)?;
        }
        Ok(())
    }

    fn notification(&mut self) -> Result<(), Box<dyn Error>> {
        for round in 0..2 {
            let status = Command::new("gdbus")
                .args([
                    "call",
                    "--session",
                    "--dest",
                    "org.freedesktop.Notifications",
                    "--object-path",
                    "/org/freedesktop/Notifications",
                    "--method",
                    "org.freedesktop.Notifications.Notify",
                    "Benchmark",
                    "0",
                    "",
                    &format!("Message {round}"),
                    "A notification banner slides in and out",
                    "[]",
                    "{}",
                    "1500",
                ])
                .stdout(Stdio::null())
                .status()?;
            if !status.success() {
                return Err("cannot send a notification".into());
            }
            self.wait(STEPS * 3)?;
        }
        Ok(())
    }

    fn overview(&mut self) -> Result<(), Box<dyn Error>> {
        for _ in 0..2 {
            self.tap(&[KEY_LEFTALT, KEY_LEFTMETA, KEY_UP]);
            self.wait(STEPS / 2)?;
            self.tap(&[KEY_LEFTALT, KEY_LEFTMETA, KEY_UP]);
            self.wait(STEPS / 2)?;
        }
        Ok(())
    }

    fn measure(&mut self, name: &'static str, scenario: Scenario) -> Result<Row, Box<dyn Error>> {
        self.run_for(AFTERMATH)?;
        self.output.frames.clear();
        let compositor_before = cpu_ticks(std::process::id())?;
        let clients_before = self.clients_ticks()?;
        let started = Instant::now();
        scenario(self)?;
        self.run_for(AFTERMATH)?;
        let compositor = cpu_ticks(std::process::id())? - compositor_before;
        let per_client: Vec<u64> = self
            .clients_ticks()?
            .iter()
            .zip(&clients_before)
            .map(|(after, before)| after - before)
            .collect();
        let clients: u64 = per_client.iter().sum();
        let busiest = self
            .clients
            .iter()
            .zip(&per_client)
            .map(|((name, _), ticks)| format!("{name} {} ms", *ticks as f64 * TICK_MS))
            .collect::<Vec<_>>()
            .join(", ");
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let frames = &self.output.frames;
        if self.list_frames {
            for (time, stats) in frames {
                eprintln!(
                    "FRAME {name} {:.1} ms {stats:?}",
                    time.as_secs_f64() * 1000.0
                );
            }
        }
        let count = frames.len().max(1) as f64;
        let mut times: Vec<Duration> = frames.iter().map(|(time, _)| *time).collect();
        times.sort();
        Ok(Row {
            name,
            frames: frames.len(),
            p50: percentile(&times, 0.50),
            p99: percentile(&times, 0.99),
            compositor_cpu: compositor as f64 * TICK_MS / count,
            clients_cpu: clients as f64 * TICK_MS / count,
            busy: (compositor + clients) as f64 * TICK_MS / elapsed_ms,
            repainted: frames
                .iter()
                .map(|(_, stats)| stats.repainted_pixels as f64)
                .sum::<f64>()
                / count
                / 1e6,
            clients: busiest,
        })
    }

    fn save_snapshot(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        let snapshot = self.output.renderer.snapshot(&self.server)?;
        let mut ppm = format!("P6\n{} {}\n255\n", snapshot.size.w, snapshot.size.h).into_bytes();
        for pixel in snapshot.rgba.as_chunks::<4>().0 {
            ppm.extend_from_slice(&pixel[..3]);
        }
        fs::write(path, ppm)?;
        Ok(())
    }

    fn clients_ticks(&mut self) -> Result<Vec<u64>, Box<dyn Error>> {
        let mut ticks = Vec::new();
        for (name, child) in &mut self.clients {
            if let Some(status) = child.try_wait()? {
                return Err(format!("{name} exited with {status}").into());
            }
            ticks.push(cpu_ticks(child.id())?);
        }
        Ok(ticks)
    }
}

fn percentile(sorted: &[Duration], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((sorted.len() as f64 * fraction).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1].as_secs_f64() * 1000.0
}

fn cpu_ticks(pid: u32) -> Result<u64, Box<dyn Error>> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let after_name = stat.rsplit_once(')').ok_or("malformed stat")?.1;
    let fields: Vec<&str> = after_name.split_whitespace().collect();
    let field = |index: usize| -> Result<u64, Box<dyn Error>> {
        Ok(fields.get(index).ok_or("short stat")?.parse()?)
    };
    Ok(field(11)? + field(12)?)
}
