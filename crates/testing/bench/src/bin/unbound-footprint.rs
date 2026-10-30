use std::error::Error;
use std::fs;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use smithay::utils::{Point, Size};
use unbound_compositor::{FrameClock, FrameScheduler, Headless, REFRESH, Renderer};

const OUTPUT: (i32, i32) = (2928, 1818);
const SETTLE: Duration = Duration::from_secs(4);
const HOVER_CHANGES: u32 = 40;
const HOVER_SETTLE: Duration = Duration::from_millis(120);
const DEVICE_MEMORY: &str = "/memfd:allocation";

struct Hover {
    from: (f64, f64),
    to: (f64, f64),
}

struct Footprint {
    resident_mib: f64,
    device_mib: f64,
    threads: u32,
    hover_cpu_ms: Option<f64>,
}

fn main() -> ExitCode {
    unbound_log::init("unbound-footprint");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut hover = None;
    let mut clients = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--hover" {
            let value = args.next().ok_or("--hover needs x1,y1,x2,y2")?;
            let numbers = value
                .split(',')
                .map(str::parse)
                .collect::<Result<Vec<f64>, _>>()?;
            let [x1, y1, x2, y2] = numbers[..] else {
                return Err("--hover needs x1,y1,x2,y2".into());
            };
            hover = Some(Hover {
                from: (x1, y1),
                to: (x2, y2),
            });
        } else {
            clients.push((arg, hover.take()));
        }
    }
    if clients.is_empty() {
        return Err("usage: unbound-footprint [--hover x1,y1,x2,y2] <client>...".into());
    }
    println!("| Client | RSS+swap MiB | Device memory MiB | Threads | CPU per hover change ms |");
    println!("| :--- | ---: | ---: | ---: | ---: |");
    for (client, hover) in clients {
        let footprint = measure(&client, hover.as_ref())?;
        println!(
            "| {client} | {:.1} | {:.1} | {} | {} |",
            footprint.resident_mib,
            footprint.device_mib,
            footprint.threads,
            footprint
                .hover_cpu_ms
                .map_or_else(|| "-".to_owned(), |ms| format!("{ms:.2}")),
        );
    }
    Ok(())
}

struct Session {
    server: Headless,
    renderer: Renderer,
    scheduler: FrameScheduler,
    epoch: Instant,
}

impl Session {
    fn run_for(&mut self, duration: Duration) -> Result<(), Box<dyn Error>> {
        let started = Instant::now();
        while started.elapsed() < duration {
            self.server.dispatch()?;
            let epoch = self.epoch;
            self.scheduler
                .poll(|| epoch.elapsed(), &mut self.server, &mut self.renderer)?;
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }
}

fn measure(client: &str, hover: Option<&Hover>) -> Result<Footprint, Box<dyn Error>> {
    let mut session = Session {
        server: Headless::new(Size::from(OUTPUT))?,
        renderer: Renderer::new()?,
        scheduler: FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH)),
        epoch: Instant::now(),
    };
    let (client_end, server_end) = UnixStream::pair()?;
    session.server.insert_client(server_end)?;
    let mut child = Command::new(client)
        .env("WAYLAND_SOCKET", "0")
        .env_remove("WAYLAND_DISPLAY")
        .stdin(Stdio::from(OwnedFd::from(client_end)))
        .stdout(Stdio::null())
        .spawn()?;
    let result = observe(&mut session, &mut child, hover);
    child.kill()?;
    child.wait()?;
    result
}

fn observe(
    session: &mut Session,
    child: &mut Child,
    hover: Option<&Hover>,
) -> Result<Footprint, Box<dyn Error>> {
    session.run_for(SETTLE)?;
    if let Some(status) = child.try_wait()? {
        return Err(format!("the client exited with {status}").into());
    }
    let pid = child.id();
    let hover_cpu_ms = match hover {
        Some(hover) => {
            let before = cpu_ticks(pid)?;
            for change in 0..HOVER_CHANGES {
                let (x, y) = if change % 2 == 0 {
                    hover.to
                } else {
                    hover.from
                };
                session
                    .server
                    .move_pointer_to(Point::from((x, y)), change * 100);
                session.run_for(HOVER_SETTLE)?;
            }
            let ticks = cpu_ticks(pid)? - before;
            Some(ticks as f64 * 10.0 / f64::from(HOVER_CHANGES))
        }
        None => None,
    };
    let (resident_mib, device_mib) = memory(pid)?;
    Ok(Footprint {
        resident_mib,
        device_mib,
        threads: fs::read_dir(format!("/proc/{pid}/task"))?.count() as u32,
        hover_cpu_ms,
    })
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

fn memory(pid: u32) -> Result<(f64, f64), Box<dyn Error>> {
    let smaps = fs::read_to_string(format!("/proc/{pid}/smaps"))?;
    let mut total = 0;
    let mut device = 0;
    let mut in_device = false;
    for line in smaps.lines() {
        let mut words = line.split_whitespace();
        let Some(first) = words.next() else {
            continue;
        };
        if first.contains('-') && !first.ends_with(':') {
            in_device = line.contains(DEVICE_MEMORY);
            continue;
        }
        if first == "Rss:" || first == "Swap:" {
            let kib: u64 = words.next().ok_or("malformed smaps")?.parse()?;
            total += kib;
            if in_device {
                device += kib;
            }
        }
    }
    Ok((total as f64 / 1024.0, device as f64 / 1024.0))
}
