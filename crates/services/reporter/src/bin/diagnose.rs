use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bande_reporter::config::Config;
use bande_reporter::diagnose::{diagnose, text_of_report};
use bande_reporter::spool::Spool;
use serde_json::{Value, json};

const SOCKET: &str = "/run/bande/report.sock";
const SPOOL: &str = "/var/lib/bande/reports";
const CONFIG: &str = "/etc/bande/reports.conf";
const USAGE: &str = "usage: bande-diagnose [--send] [--describe TEXT]";

struct Asked {
    send: bool,
    description: String,
}

fn through_service(asked: &Asked) -> std::io::Result<Value> {
    let mut stream = UnixStream::connect(SOCKET)?;
    let request = json!({"op": "diagnose", "send": asked.send, "description": asked.description});
    writeln!(stream, "{request}")?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer)?;
    Ok(serde_json::from_str(&answer)?)
}

fn in_this_process(asked: &Asked) -> std::io::Result<Value> {
    let root = Path::new(SPOOL);
    let config = Config::load(Path::new(CONFIG))?;
    let id = diagnose(
        root,
        &config,
        rustix::process::getuid().as_raw(),
        &asked.description,
        asked.send,
    )?;
    Ok(
        json!({"ok": {"id": id, "queued": asked.send && config.has_server(), "text": text_of_report(&Spool::open(root)?, &id)?}}),
    )
}

fn saved_copy(id: &str, text: &str) -> std::io::Result<PathBuf> {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    let path = home.join(format!("bande-report-{id}.txt"));
    fs::write(&path, text)?;
    Ok(path)
}

fn parse() -> Option<Asked> {
    let mut asked = Asked {
        send: false,
        description: String::new(),
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--send" => asked.send = true,
            "--describe" => asked.description = arguments.next()?,
            _ => return None,
        }
    }
    Some(asked)
}

fn main() -> ExitCode {
    let Some(asked) = parse() else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let answer = through_service(&asked).or_else(|_| in_this_process(&asked));
    let answer = match answer {
        Ok(answer) => answer,
        Err(error) => {
            eprintln!("cannot make a report: {error}");
            return ExitCode::FAILURE;
        }
    };
    let Some(report) = answer.get("ok") else {
        eprintln!("{answer}");
        return ExitCode::FAILURE;
    };
    let id = report.get("id").and_then(Value::as_str).unwrap_or_default();
    println!("report {id}");
    if report.get("queued").and_then(Value::as_bool) == Some(true) {
        println!("sending; it is known to the server by this id");
    } else if let Some(text) = report.get("text").and_then(Value::as_str) {
        match saved_copy(id, text) {
            Ok(path) => println!("no server is configured; saved to {}", path.display()),
            Err(error) => eprintln!("cannot save a copy: {error}"),
        }
    } else {
        println!(
            "no server is configured; it stays in Settings ▸ Privacy & Security ▸ Diagnostics"
        );
    }
    ExitCode::SUCCESS
}
