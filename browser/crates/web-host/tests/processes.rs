use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Command as Process, ExitCode};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, poll};
use rustix::time::Timespec;
use web_host::{
    Command, Destination, Helpers, Host, LoadId, Outcome, Program, Session, Update, Url, Verdict,
    ViewId, Viewport,
};

const PAGE: &str = "<!doctype html><title>Processes</title>\
    <link rel=stylesheet href=/blocked.css><p>Hello from a sandboxed renderer</p>";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("renderer") => return web_renderer::main(),
        Some("network") => return web_netd::main(),
        _ => {}
    }
    match scenario() {
        Ok(()) => {
            println!("processes: hook, hang, crash, reload and a network restart work");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("FAIL: {error}");
            ExitCode::FAILURE
        }
    }
}

fn serve(listener: TcpListener) {
    for stream in listener.incoming().flatten() {
        let mut reader = BufReader::new(&stream);
        let mut request = String::new();
        if reader.read_line(&mut request).is_err() {
            continue;
        }
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok_and(|count| count > 2) {
            line.clear();
        }
        let (kind, body) = match request.contains("/blocked.css") {
            true => ("text/css", "p { color: red }"),
            false => ("text/html", PAGE),
        };
        let _ = write!(
            &stream,
            "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}

fn program(role: &str) -> Result<Program, String> {
    Ok(Program {
        path: std::env::current_exe().map_err(|error| error.to_string())?,
        arguments: vec![role.to_owned()],
    })
}

fn next(host: &Host, pending: &mut Vec<Update>) -> Result<Update, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while pending.is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("no update within 30 s".into());
        }
        let timeout = Timespec {
            tv_sec: left.as_secs() as i64,
            tv_nsec: 0,
        };
        let mut fds = [PollFd::from_borrowed_fd(host.fd(), PollFlags::IN)];
        poll(&mut fds, Some(&timeout)).map_err(|error| error.to_string())?;
        pending.extend(host.updates());
    }
    Ok(pending.remove(0))
}

fn scenario() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    std::thread::spawn(move || serve(listener));
    let url =
        Url::parse(&format!("http://127.0.0.1:{port}/")).map_err(|error| error.to_string())?;
    let (pids, launched) = mpsc::channel();
    let helpers = Helpers {
        renderer: program("renderer")?,
        network: program("network")?,
        unresponsive_after: Duration::from_secs(1),
    };
    let host = Host::new(
        helpers,
        Box::new(move |pid| {
            let _ = pids.send(pid);
        }),
    )
    .map_err(|error| error.to_string())?;
    let link = host.link();
    let view = ViewId(1);
    let load = |load: u64| Command::Load {
        view,
        load: LoadId(load),
        url: url.clone(),
        viewport: Viewport {
            width: 800.0,
            height: 600.0,
        },
        session: Session::Default,
    };
    let mut pending = Vec::new();
    link.send(Command::Intercept(true));
    link.send(load(1));
    let mut blocked = false;
    let title = loop {
        match next(&host, &mut pending)?.outcome {
            Outcome::Paused(paused) => {
                let verdict = match paused.destination {
                    Destination::Style => {
                        blocked = true;
                        Verdict::Cancel
                    }
                    _ => Verdict::Continue,
                };
                link.send(Command::Resume {
                    request: paused.request,
                    verdict,
                });
            }
            Outcome::Frame(frame) => break frame.title,
            Outcome::Failed { message, .. } => return Err(message),
            _ => {}
        }
    };
    if !blocked || title.as_deref() != Some("Processes") {
        return Err(format!(
            "the paused stylesheet was not seen (title {title:?})"
        ));
    }
    link.send(Command::Intercept(false));
    let pid = launched
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    signal(pid, "-STOP")?;
    link.send(Command::Resize {
        view,
        viewport: Viewport {
            width: 700.0,
            height: 600.0,
        },
    });
    expect(&host, &mut pending, |outcome| {
        matches!(outcome, Outcome::Unresponsive)
    })?;
    signal(pid, "-CONT")?;
    expect(&host, &mut pending, |outcome| {
        matches!(outcome, Outcome::Responsive)
    })?;
    Process::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status()
        .map_err(|error| error.to_string())?;
    loop {
        let update = next(&host, &mut pending)?;
        if let Outcome::Crashed { .. } = update.outcome {
            if update.load != LoadId(1) {
                return Err("the crash names the wrong load".into());
            }
            break;
        }
    }
    link.send(load(2));
    framed(&host, &mut pending, LoadId(2))?;
    for pid in helpers_named("network") {
        Process::new("kill")
            .args(["-KILL", &pid])
            .status()
            .map_err(|error| error.to_string())?;
    }
    link.send(load(3));
    if framed(&host, &mut pending, LoadId(3)).is_ok() {
        return Ok(());
    }
    link.send(load(4));
    framed(&host, &mut pending, LoadId(4))
}

fn signal(pid: u32, signal: &str) -> Result<(), String> {
    Process::new("kill")
        .args([signal, &pid.to_string()])
        .status()
        .map(drop)
        .map_err(|error| error.to_string())
}

fn expect(
    host: &Host,
    pending: &mut Vec<Update>,
    wanted: impl Fn(&Outcome) -> bool,
) -> Result<(), String> {
    loop {
        if wanted(&next(host, pending)?.outcome) {
            return Ok(());
        }
    }
}

fn framed(host: &Host, pending: &mut Vec<Update>, load: LoadId) -> Result<(), String> {
    loop {
        let update = next(host, pending)?;
        match update.outcome {
            Outcome::Frame(_) if update.load == load => return Ok(()),
            Outcome::Crashed { .. } => return Err("a renderer crashed".into()),
            Outcome::Failed { message, .. } | Outcome::Unsupported { kind: message, .. }
                if update.load == load =>
            {
                return Err(message);
            }
            _ => {}
        }
    }
}

fn helpers_named(role: &str) -> Vec<String> {
    let mut found = Vec::new();
    let tasks = std::fs::read_dir("/proc/self/task")
        .into_iter()
        .flatten()
        .flatten();
    for task in tasks {
        let children = std::fs::read_to_string(task.path().join("children")).unwrap_or_default();
        for pid in children.split_whitespace() {
            let command = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            if command
                .split(|byte| *byte == 0)
                .any(|argument| argument == role.as_bytes())
            {
                found.push(pid.to_owned());
            }
        }
    }
    found
}
