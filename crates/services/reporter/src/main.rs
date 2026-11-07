use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bande_reporter::config::Config;
use bande_reporter::send::{self, Outcome};
use bande_reporter::spool::{Attachment, Spool, Status};
use bande_reporter::wait::{Waiter, Woken};
use serde_json::{Map, Value};

const SPOOL: &str = "/var/lib/bande/reports";
const CONFIG: &str = "/etc/bande/reports.conf";
const FIRST_RETRY: Duration = Duration::from_secs(60);
const LONGEST_RETRY: Duration = Duration::from_secs(6 * 60 * 60);
const USAGE: &str = "usage: bande-reporter [--spool DIR] [--config FILE] (send | serve | watch | list | store MANIFEST [ATTACHMENT...] | queue ID | delete ID)";

struct Paths {
    spool: PathBuf,
    config: PathBuf,
}

enum Pause {
    None,
    Network,
    Server(Duration),
    UntilConfigChanges,
}

fn attempt(spool: &Spool, config: &Config, waiting: &[String], retry: Duration) -> Pause {
    for id in waiting {
        let Ok(report) = spool.read(id) else {
            settle(spool, id, Status::Rejected);
            continue;
        };
        match send::send(config, &report) {
            Outcome::Stored => settle(spool, id, Status::Sent),
            Outcome::BadFormat | Outcome::TooLarge => {
                tracing::warn!("the server refused report {id}");
                settle(spool, id, Status::Rejected);
            }
            Outcome::Unauthorized => {
                tracing::warn!("the report server refuses the configured key");
                return Pause::UntilConfigChanges;
            }
            Outcome::Later(wait) => return Pause::Server(wait.unwrap_or(retry)),
            Outcome::NoRoute(error) => {
                if matches!(
                    error.kind(),
                    io::ErrorKind::InvalidData | io::ErrorKind::Other
                ) {
                    tracing::warn!("cannot talk to the report server: {error}");
                } else {
                    tracing::debug!("cannot reach the report server: {error}");
                }
                return Pause::Network;
            }
        }
    }
    Pause::None
}

fn settle(spool: &Spool, id: &str, status: Status) {
    if let Err(error) = spool.settle(id, status) {
        tracing::warn!("cannot record the state of report {id}: {error}");
        if let Err(error) = spool.unqueue(id) {
            tracing::warn!("cannot take report {id} out of the outbox: {error}");
        }
    }
}

fn run_sender(paths: &Paths) -> Result<(), Box<dyn Error>> {
    let spool = Spool::open(&paths.spool)?;
    let waiter = Waiter::new(&paths.config);
    let mut retry = FIRST_RETRY;
    loop {
        let waiting = spool.waiting()?;
        if waiting.is_empty() {
            return Ok(());
        }
        let config = match Config::load(&paths.config) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!("cannot read the report settings: {error}");
                Config::default()
            }
        };
        let pause = if config.server().is_some() {
            attempt(&spool, &config, &waiting, retry)
        } else {
            Pause::UntilConfigChanges
        };
        let woken = match pause {
            Pause::None => {
                retry = FIRST_RETRY;
                continue;
            }
            Pause::UntilConfigChanges => waiter.wait(None, false)?,
            Pause::Server(wait) => waiter.wait(Some(Instant::now() + wait), false)?,
            Pause::Network => waiter.wait(Some(Instant::now() + retry), true)?,
        };
        retry = match woken {
            Woken::Deadline => (retry * 2).min(LONGEST_RETRY),
            Woken::Network | Woken::Config => FIRST_RETRY,
        };
    }
}

fn list(paths: &Paths) -> Result<(), Box<dyn Error>> {
    let spool = Spool::open(&paths.spool)?;
    let waiting = spool.waiting()?;
    for report in spool.all() {
        let field = |name| {
            report
                .manifest
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or("-")
        };
        let status = match report.status {
            Status::Sent => "sent",
            Status::Rejected => "rejected",
            Status::Local if waiting.contains(&report.id) => "waiting",
            Status::Local => "local",
        };
        println!(
            "{} {} {} {status} x{} {} bytes",
            report.id,
            field("time"),
            field("kind"),
            report.occurrences,
            report.bytes,
        );
    }
    Ok(())
}

fn store(paths: &Paths, manifest: &Path, attachments: &[String]) -> Result<(), Box<dyn Error>> {
    let spool = Spool::open(&paths.spool)?;
    let manifest: Map<String, Value> = serde_json::from_slice(&fs::read(manifest)?)?;
    let mut read = Vec::with_capacity(attachments.len());
    for file in attachments {
        let name = Path::new(file)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, file.clone()))?;
        read.push(Attachment {
            name: name.to_owned(),
            bytes: fs::read(file)?,
        });
    }
    let automatic = Config::load(&paths.config)?.sends_automatically();
    println!("{}", spool.store(manifest, read, automatic)?);
    Ok(())
}

fn run(arguments: &[String]) -> Result<(), Box<dyn Error>> {
    let mut paths = Paths {
        spool: PathBuf::from(SPOOL),
        config: PathBuf::from(CONFIG),
    };
    let mut rest = arguments;
    while let [flag, value, tail @ ..] = rest {
        match flag.as_str() {
            "--spool" => paths.spool = PathBuf::from(value),
            "--config" => paths.config = PathBuf::from(value),
            _ => break,
        }
        rest = tail;
    }
    match rest {
        [command] if command == "send" => run_sender(&paths),
        [command] if command == "list" => list(&paths),
        [command] if command == "collect-failures" => {
            Ok(bande_reporter::diagnose::ask_for_collection()?)
        }
        [command] if command == "serve" => bande_reporter::serve::run(&paths.spool, &paths.config),
        [command] if command == "watch" => bande_reporter::watch::run(&paths.spool, &paths.config),
        [command, manifest, attachments @ ..] if command == "store" => {
            store(&paths, Path::new(manifest), attachments)
        }
        [command, id] if command == "queue" => Ok(Spool::open(&paths.spool)?.queue(id)?),
        [command, id] if command == "delete" => Ok(Spool::open(&paths.spool)?.delete(id)?),
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    bande_log::init("bande-reporter");
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
