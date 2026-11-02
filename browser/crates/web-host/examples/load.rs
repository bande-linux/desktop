use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, poll};
use rustix::time::Timespec;
use web_host::{Command, Helpers, Host, LoadId, Outcome, Program, Session, Url, ViewId, Viewport};

const WIDTH: u16 = 1000;
const HEIGHT: u16 = 2000;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some(url) = arguments.first().and_then(|url| Url::parse(url).ok()) else {
        eprintln!("usage: load URL [OUTPUT.png]");
        return ExitCode::FAILURE;
    };
    match run(url, arguments.get(1).map(PathBuf::from)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("load: {error}");
            ExitCode::FAILURE
        }
    }
}

fn helper(name: &str) -> std::io::Result<Program> {
    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .and_then(Path::parent)
        .ok_or(std::io::ErrorKind::NotFound)?;
    Ok(Program {
        path: directory.join(name),
        arguments: Vec::new(),
    })
}

fn children() -> Vec<u32> {
    let mut pids = Vec::new();
    for task in std::fs::read_dir("/proc/self/task")
        .into_iter()
        .flatten()
        .flatten()
    {
        let listed = std::fs::read_to_string(task.path().join("children")).unwrap_or_default();
        pids.extend(
            listed
                .split_whitespace()
                .filter_map(|pid| pid.parse::<u32>().ok()),
        );
    }
    pids
}

fn status(pid: u32) -> (String, u64, u64) {
    let text = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .map(|value| value.trim().to_owned())
            .unwrap_or_default()
    };
    let switches = |name: &str| field(name).parse::<u64>().unwrap_or(0);
    let memory = format!(
        "{} rss {} anon {}",
        field("Name:"),
        field("VmRSS:"),
        field("RssAnon:")
    );
    (
        memory,
        switches("voluntary_ctxt_switches:"),
        switches("nonvoluntary_ctxt_switches:"),
    )
}

fn wait(host: &Host, timeout: Duration) -> bool {
    let mut fds = [PollFd::from_borrowed_fd(host.fd(), PollFlags::IN)];
    let timeout = Timespec {
        tv_sec: timeout.as_secs() as i64,
        tv_nsec: i64::from(timeout.subsec_nanos()),
    };
    matches!(poll(&mut fds, Some(&timeout)), Ok(count) if count > 0)
}

fn run(url: Url, output: Option<PathBuf>) -> std::io::Result<()> {
    let helpers = Helpers {
        renderer: helper("web-renderer")?,
        network: helper("web-netd")?,
    };
    let started = Instant::now();
    let host = Host::new(helpers, Box::new(|pid| eprintln!("renderer {pid}")))?;
    let view = ViewId(1);
    host.link().send(Command::Load {
        view,
        load: LoadId(1),
        url,
        viewport: Viewport {
            width: f32::from(WIDTH),
            height: f32::from(HEIGHT),
        },
        session: Session::Default,
    });
    let mut last = None;
    while wait(&host, Duration::from_secs(5)) {
        for update in host.updates() {
            let elapsed = started.elapsed().as_millis();
            match update.outcome {
                Outcome::Frame(frame) => {
                    println!(
                        "{elapsed} ms: frame {:?}, {} items, {} links",
                        frame.title,
                        frame.list.items.len(),
                        frame.links.targets.len()
                    );
                    last = Some(frame.list);
                }
                Outcome::Relaid(list, _, damage) => {
                    println!(
                        "{elapsed} ms: relaid, {} images, damage {damage:?}",
                        list.images.len()
                    );
                    last = Some(list);
                }
                Outcome::Failed { message, .. } => println!("{elapsed} ms: failed: {message}"),
                Outcome::Unsupported { kind, .. } => println!("{elapsed} ms: unsupported {kind}"),
                Outcome::Crashed { url } => println!("{elapsed} ms: crashed at {url}"),
                Outcome::Scroll(_) | Outcome::Paused(_) => {}
            }
        }
    }
    let pids = children();
    let before: Vec<_> = pids.iter().map(|pid| status(*pid)).collect();
    std::thread::sleep(Duration::from_secs(10));
    for (pid, (memory, voluntary, involuntary)) in pids.iter().zip(before) {
        let (_, after_voluntary, after_involuntary) = status(*pid);
        println!(
            "{pid} {memory}; wake-ups in 10 s idle: {}",
            after_voluntary + after_involuntary - voluntary - involuntary
        );
    }
    if let (Some(list), Some(output)) = (last, output) {
        let height = (list.height.ceil() as u16).clamp(1, HEIGHT);
        let rgba = list.rasterize(0.0, WIDTH, height);
        let mut encoder = png::Encoder::new(
            BufWriter::new(File::create(output)?),
            u32::from(WIDTH),
            u32::from(height),
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&rgba))
            .map_err(std::io::Error::other)?;
    }
    Ok(())
}
