use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bande_reporter::config::Config;
use bande_reporter::spool::{Attachment, Spool, Status};
use serde_json::{Map, Value};

const SPOOL: &str = "/var/lib/bande/reports";
const CONFIG: &str = "/etc/bande/reports.conf";
const USAGE: &str = "usage: bande-reporter [--spool DIR] [--config FILE] (send | serve | resources | list | sending on|off | store MANIFEST [ATTACHMENT...] | queue ID | delete ID)";

struct Paths {
    spool: PathBuf,
    config: PathBuf,
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
    let automatic = Config::load(&paths.config)?.send;
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
        [command] if command == "send" => {
            bande_reporter::dispatch::run(&paths.spool, &paths.config)
        }
        [command] if command == "list" => list(&paths),
        [command, unit] if command == "collect-failures" => {
            Ok(bande_reporter::diagnose::ask_for_collection(unit)?)
        }
        [command] if command == "serve" => bande_reporter::serve::run(&paths.spool, &paths.config),
        [command] if command == "resources" => {
            bande_reporter::resources::run(&paths.spool, &paths.config)
        }
        [command, manifest, attachments @ ..] if command == "store" => {
            store(&paths, Path::new(manifest), attachments)
        }
        [command, on] if command == "sending" && (on == "on" || on == "off") => Ok(
            bande_reporter::serve::set_sending(&paths.spool, &paths.config, on == "on")?,
        ),
        [command, id] if command == "queue" => Ok(Spool::open(&paths.spool)?.queue_by_hand(id)?),
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
