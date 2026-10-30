use std::error::Error;
use std::fs;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use smithay::utils::Size;
use unbound_compositor::{FrameClock, FrameScheduler, Headless, REFRESH, Renderer};
use unbound_toolkit::{CpuRenderer, GpuRenderer, Palette, RENDERER_VARIABLE, Scheme, TextSystem};

const STARTS: usize = 5;
const FRAMES: usize = 120;
const SETTLE: Duration = Duration::from_millis(1500);
const START_TIMEOUT: Duration = Duration::from_secs(20);
const REFRESH_120HZ_MS: f64 = 1000.0 / 120.0;

struct Metric {
    name: &'static str,
    unit: &'static str,
    value: f64,
    budget: f64,
    enforced: bool,
}

impl Metric {
    fn within_budget(&self) -> bool {
        self.value <= self.budget
    }
}

struct FrameTimes {
    cpu: Vec<Duration>,
    gpu: Vec<Duration>,
    adapter: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Renderers {
    CpuThenGpu,
    CpuOnly,
}

struct Start {
    first_frame: Duration,
    first_frame_rss: f64,
    settled_rss: f64,
}

fn main() -> ExitCode {
    unbound_log::init("unbound-toolkit-bench");
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, Box<dyn Error>> {
    let (sample, json) = options()?;
    let starts = (0..STARTS)
        .map(|_| start(&sample, Renderers::CpuThenGpu))
        .collect::<Result<Vec<_>, _>>()?;
    let cpu_only = start(&sample, Renderers::CpuOnly)?;
    let FrameTimes { cpu, gpu, adapter } = frame_times()?;
    let first_frames: Vec<Duration> = starts.iter().map(|start| start.first_frame).collect();
    let median_start = |value: fn(&Start) -> f64| {
        let mut values: Vec<f64> = starts.iter().map(value).collect();
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    let metrics = [
        Metric {
            name: "toolkit_first_frame_p50",
            unit: "ms",
            value: percentile(&first_frames, 0.50),
            budget: 100.0,
            enforced: false,
        },
        Metric {
            name: "toolkit_first_frame_rss",
            unit: "MiB",
            value: median_start(|start| start.first_frame_rss),
            budget: 30.0,
            enforced: false,
        },
        Metric {
            name: "toolkit_cpu_only_rss",
            unit: "MiB",
            value: cpu_only.settled_rss,
            budget: 30.0,
            enforced: true,
        },
        Metric {
            name: "toolkit_settled_rss",
            unit: "MiB",
            value: median_start(|start| start.settled_rss),
            budget: 30.0,
            enforced: false,
        },
        Metric {
            name: "toolkit_cpu_frame_p50",
            unit: "ms",
            value: percentile(&cpu, 0.50),
            budget: REFRESH_120HZ_MS,
            enforced: false,
        },
        Metric {
            name: "toolkit_cpu_frame_p99",
            unit: "ms",
            value: percentile(&cpu, 0.99),
            budget: REFRESH_120HZ_MS,
            enforced: false,
        },
        Metric {
            name: "toolkit_gpu_frame_and_readback_p50",
            unit: "ms",
            value: percentile(&gpu, 0.50),
            budget: REFRESH_120HZ_MS,
            enforced: false,
        },
        Metric {
            name: "toolkit_gpu_frame_and_readback_p99",
            unit: "ms",
            value: percentile(&gpu, 0.99),
            budget: REFRESH_120HZ_MS,
            enforced: false,
        },
    ];
    print!("{}", markdown(&metrics, &adapter));
    fs::write(json, json_report(&metrics, &adapter))?;
    Ok(metrics
        .iter()
        .all(|metric| metric.within_budget() || !metric.enforced))
}

fn options() -> Result<(PathBuf, PathBuf), Box<dyn Error>> {
    let mut sample = None;
    let mut json = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--sample" => sample = Some(PathBuf::from(value)),
            "--json" => json = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown argument {flag}").into()),
        }
    }
    Ok((
        sample.ok_or("--sample <path> is required")?,
        json.ok_or("--json <path> is required")?,
    ))
}

struct Session {
    server: Headless,
    renderer: Renderer,
    scheduler: FrameScheduler,
    epoch: Instant,
}

impl Session {
    fn step(&mut self) -> Result<(), Box<dyn Error>> {
        self.server.dispatch()?;
        let epoch = self.epoch;
        self.scheduler
            .poll(|| epoch.elapsed(), &mut self.server, &mut self.renderer)?;
        Ok(())
    }

    fn mapped(&self) -> bool {
        self.server
            .windows()
            .first()
            .is_some_and(|window| window.size().is_some())
    }
}

fn start(sample: &PathBuf, renderers: Renderers) -> Result<Start, Box<dyn Error>> {
    let mut session = Session {
        server: Headless::new(Size::from((1280, 800)))?,
        renderer: Renderer::new()?,
        scheduler: FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH)),
        epoch: Instant::now(),
    };
    let (client_end, server_end) = UnixStream::pair()?;
    session.server.insert_client(server_end)?;
    let started = Instant::now();
    let mut command = Command::new(sample);
    command.env(
        RENDERER_VARIABLE,
        match renderers {
            Renderers::CpuThenGpu => "gpu",
            Renderers::CpuOnly => "cpu",
        },
    );
    let mut child = command
        .env("WAYLAND_SOCKET", "0")
        .env_remove("WAYLAND_DISPLAY")
        .stdin(Stdio::from(OwnedFd::from(client_end)))
        .stdout(Stdio::null())
        .spawn()?;
    let result = measure(&mut session, &mut child, started);
    child.kill()?;
    child.wait()?;
    result
}

fn measure(
    session: &mut Session,
    child: &mut Child,
    started: Instant,
) -> Result<Start, Box<dyn Error>> {
    while !session.mapped() {
        if started.elapsed() > START_TIMEOUT {
            return Err("the sample did not map a window".into());
        }
        if let Some(status) = child.try_wait()? {
            return Err(format!("the sample exited with {status}").into());
        }
        session.step()?;
        thread::sleep(Duration::from_micros(100));
    }
    let first_frame = started.elapsed();
    let first_frame_rss = resident_mib(child.id())?;
    let settle = Instant::now();
    while settle.elapsed() < SETTLE {
        session.step()?;
        thread::sleep(Duration::from_millis(1));
    }
    Ok(Start {
        first_frame,
        first_frame_rss,
        settled_rss: resident_mib(child.id())?,
    })
}

fn frame_times() -> Result<FrameTimes, Box<dyn Error>> {
    let mut text = TextSystem::new();
    let scene = unbound_sample::scene(
        &mut text,
        &Palette::for_scheme(Scheme::Light),
        unbound_sample::SIZE,
    );
    let mut cpu = CpuRenderer::new();
    let mut gpu = GpuRenderer::new()?;
    let mut cpu_times = Vec::with_capacity(FRAMES);
    let mut gpu_times = Vec::with_capacity(FRAMES);
    for _ in 0..FRAMES {
        let started = Instant::now();
        cpu.render(&scene)?;
        cpu_times.push(started.elapsed());
        let started = Instant::now();
        gpu.render(&scene)?;
        gpu_times.push(started.elapsed());
    }
    Ok(FrameTimes {
        cpu: cpu_times,
        gpu: gpu_times,
        adapter: gpu.adapter_info().name,
    })
}

fn percentile(times: &[Duration], fraction: f64) -> f64 {
    let mut sorted = times.to_vec();
    sorted.sort();
    let rank = ((sorted.len() as f64 * fraction).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1].as_secs_f64() * 1000.0
}

fn resident_mib(pid: u32) -> Result<f64, Box<dyn Error>> {
    let status = fs::read_to_string(format!("/proc/{pid}/status"))?;
    let kib: f64 = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.trim().strip_suffix("kB"))
        .ok_or("no VmRSS in /proc/<pid>/status")?
        .trim()
        .parse()?;
    Ok(kib / 1024.0)
}

fn markdown(metrics: &[Metric], adapter: &str) -> String {
    let mut table = format!(
        "### unbound-toolkit-bench on {adapter}\n\n| Metric | Value | Budget | |\n| :--- | ---: | ---: | :---: |\n"
    );
    for metric in metrics {
        table += &format!(
            "| `{}` | {:.2} {unit} | {:.2} {unit} | {} |\n",
            metric.name,
            metric.value,
            metric.budget,
            match (metric.within_budget(), metric.enforced) {
                (true, _) => "ok",
                (false, true) => "over",
                (false, false) => "over, reported only",
            },
            unit = metric.unit,
        );
    }
    table
}

fn json_report(metrics: &[Metric], adapter: &str) -> String {
    let entries: Vec<String> = metrics
        .iter()
        .map(|metric| {
            format!(
                "    {{\"name\": \"{}\", \"unit\": \"{}\", \"value\": {:.3}, \"budget\": {:.3}, \"enforced\": {}}}",
                metric.name, metric.unit, metric.value, metric.budget, metric.enforced
            )
        })
        .collect();
    format!(
        "{{\n  \"adapter\": \"{}\",\n  \"metrics\": [\n{}\n  ]\n}}\n",
        adapter.replace('"', "'"),
        entries.join(",\n")
    )
}
