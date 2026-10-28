use std::error::Error;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use smithay::utils::Size;
use unbound_compositor::{FrameStats, Headless, Renderer};
use unbound_test_wayland::{ShmImage, TestClient, quadrants};
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (1920, 1080);
const WINDOWS: usize = 8;
const WINDOW_WIDTH: usize = 640;
const WINDOW_HEIGHT: usize = 480;
const TYPICAL_DAMAGE: i32 = 64;
const FRAMES: usize = 240;
const IDLE_SETTLE: Duration = Duration::from_secs(1);

const REFRESH_120HZ_MS: f64 = 1000.0 / 120.0;
const REFRESH_60HZ_MS: f64 = 1000.0 / 60.0;

struct Metric {
    name: &'static str,
    unit: &'static str,
    value: f64,
    budget: f64,
}

impl Metric {
    fn within_budget(&self) -> bool {
        self.value <= self.budget
    }
}

struct Options {
    compositor: PathBuf,
    json: PathBuf,
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("unbound-bench: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, Box<dyn Error>> {
    let options = options()?;
    let info = Renderer::new()?.device_info().clone();
    eprintln!(
        "unbound-bench: rendering on {} ({})",
        info.name, info.driver
    );

    let (typical, typical_work) = frame_times(Repaint::OneWindowCorner)?;
    let (full, _) = frame_times(Repaint::EveryWindow)?;
    let damaged_pixels = (TYPICAL_DAMAGE * TYPICAL_DAMAGE) as f64;
    let (start, idle_rss) = compositor_start(&options.compositor)?;

    let metrics = [
        Metric {
            name: "frame_typical_p50",
            unit: "ms",
            value: percentile(&typical, 0.50),
            budget: REFRESH_120HZ_MS,
        },
        Metric {
            name: "frame_typical_p99",
            unit: "ms",
            value: percentile(&typical, 0.99),
            budget: REFRESH_120HZ_MS,
        },
        Metric {
            name: "frame_typical_uploaded",
            unit: "B",
            value: typical_work.uploaded_bytes as f64,
            budget: damaged_pixels * 4.0,
        },
        Metric {
            name: "frame_typical_repainted",
            unit: "px",
            value: typical_work.repainted_pixels as f64,
            budget: damaged_pixels,
        },
        Metric {
            name: "frame_typical_shaded",
            unit: "px",
            value: typical_work.shaded_pixels as f64,
            budget: damaged_pixels,
        },
        Metric {
            name: "frame_full_p50",
            unit: "ms",
            value: percentile(&full, 0.50),
            budget: REFRESH_60HZ_MS,
        },
        Metric {
            name: "frame_full_p99",
            unit: "ms",
            value: percentile(&full, 0.99),
            budget: REFRESH_60HZ_MS,
        },
        Metric {
            name: "compositor_start",
            unit: "ms",
            value: start.as_secs_f64() * 1000.0,
            budget: 500.0,
        },
        Metric {
            name: "compositor_idle_rss",
            unit: "MiB",
            value: idle_rss,
            budget: 128.0,
        },
    ];

    print!("{}", markdown(&metrics, &info.name));
    fs::write(&options.json, json(&metrics, &info.name))?;
    Ok(metrics.iter().all(Metric::within_budget))
}

fn options() -> Result<Options, Box<dyn Error>> {
    let mut compositor = None;
    let mut json = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--compositor" => compositor = Some(PathBuf::from(value)),
            "--json" => json = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown argument {flag}").into()),
        }
    }
    Ok(Options {
        compositor: compositor.ok_or("--compositor <path> is required")?,
        json: json.ok_or("--json <path> is required")?,
    })
}

#[derive(Clone, Copy)]
enum Repaint {
    OneWindowCorner,
    EveryWindow,
}

fn frame_times(repaint: Repaint) -> Result<(Vec<Duration>, FrameStats), Box<dyn Error>> {
    let mut renderer = Renderer::new()?;
    let mut server = Headless::new(Size::from(OUTPUT))?;
    let mut client = TestClient::connect(&mut server);
    let stride = WINDOW_WIDTH * 4;
    let frames = [
        quadrants(WINDOW_WIDTH, WINDOW_HEIGHT, stride),
        quadrants(WINDOW_WIDTH, WINDOW_HEIGHT, stride)
            .into_iter()
            .map(|channel| !channel)
            .collect(),
    ];
    let image = |bytes| ShmImage {
        width: WINDOW_WIDTH as i32,
        height: WINDOW_HEIGHT as i32,
        stride: stride as i32,
        format: Format::Xrgb8888,
        bytes,
    };
    for _ in 0..WINDOWS {
        client.show_window(&mut server, &image(&frames[0]));
    }
    renderer.render(&mut server)?;

    let start = Instant::now();
    let mut times = Vec::with_capacity(FRAMES);
    let mut most = FrameStats::default();
    for frame in 0..FRAMES {
        let bytes = &frames[(frame + 1) % 2];
        match repaint {
            Repaint::OneWindowCorner => client.commit_buffer(
                WINDOWS - 1,
                &image(bytes),
                (0, 0, TYPICAL_DAMAGE, TYPICAL_DAMAGE),
            ),
            Repaint::EveryWindow => (0..WINDOWS).for_each(|window| {
                client.commit_buffer(
                    window,
                    &image(bytes),
                    (0, 0, WINDOW_WIDTH as i32, WINDOW_HEIGHT as i32),
                )
            }),
        }
        let composed = Instant::now();
        server.dispatch()?;
        if server.take_redraw() {
            let stats = renderer.render(&mut server)?;
            most.uploaded_bytes = most.uploaded_bytes.max(stats.uploaded_bytes);
            most.repainted_pixels = most.repainted_pixels.max(stats.repainted_pixels);
            most.shaded_pixels = most.shaded_pixels.max(stats.shaded_pixels);
            server.frame_done(start.elapsed())?;
        }
        times.push(composed.elapsed());
        client.exchange(&mut server);
    }
    Ok((times, most))
}

fn percentile(times: &[Duration], fraction: f64) -> f64 {
    let mut sorted = times.to_vec();
    sorted.sort();
    let rank = ((sorted.len() as f64 * fraction).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1].as_secs_f64() * 1000.0
}

fn compositor_start(executable: &Path) -> Result<(Duration, f64), Box<dyn Error>> {
    let runtime = tempfile::tempdir()?;
    let started = Instant::now();
    let mut child = Command::new(executable)
        .env("XDG_RUNTIME_DIR", runtime.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().ok_or("compositor stdout")?;
    let listening = BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .any(|line| line.starts_with("WAYLAND_DISPLAY="));
    let start = started.elapsed();
    std::thread::sleep(IDLE_SETTLE);
    let rss = resident_mib(child.id());
    child.kill()?;
    child.wait()?;
    if !listening {
        return Err("the compositor exited before listening".into());
    }
    Ok((start, rss?))
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
        "### unbound-bench on {adapter}\n\n| Metric | Value | Budget | |\n| :--- | ---: | ---: | :---: |\n"
    );
    for metric in metrics {
        table += &format!(
            "| `{}` | {:.2} {unit} | {:.2} {unit} | {} |\n",
            metric.name,
            metric.value,
            metric.budget,
            if metric.within_budget() { "ok" } else { "over" },
            unit = metric.unit,
        );
    }
    table
}

fn json(metrics: &[Metric], adapter: &str) -> String {
    let entries: Vec<String> = metrics
        .iter()
        .map(|metric| {
            format!(
                "    {{\"name\": \"{}\", \"unit\": \"{}\", \"value\": {:.3}, \"budget\": {:.3}}}",
                metric.name, metric.unit, metric.value, metric.budget
            )
        })
        .collect();
    format!(
        "{{\n  \"adapter\": \"{}\",\n  \"metrics\": [\n{}\n  ]\n}}\n",
        adapter.replace('"', "'"),
        entries.join(",\n")
    )
}
