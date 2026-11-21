mod journal;
mod structured;

use std::error::Error;
use std::fs::{self, File};
use std::path::Path;
use std::process::ExitCode;

use bande_reporter::facts;
use journal::Entry;
use rustix::fs::{FlockOperation, flock};

const STATE: &str = "/var/lib/bande/crash-reports";
const REPORTED_RESULTS: [&str; 4] = ["exit-code", "timeout", "watchdog", "oom-kill"];
const UNDUMPABLE: [&str; 5] = [
    "bande-clipboard",
    "bande-greeter",
    "bande-lock",
    "bande-polkit-agent",
    "bande-secrets",
];

fn is_bande_program_in(entry: &Entry, field: &str) -> bool {
    entry
        .field(field)
        .is_some_and(|path| facts::is_bande_program(Path::new(&path)))
}

fn is_reported(entry: &Entry) -> bool {
    if entry.field("MESSAGE_ID").as_deref() == Some(journal::COREDUMP_MESSAGE) {
        return is_bande_program_in(entry, "COREDUMP_EXE");
    }
    if entry.field("TARGET").as_deref() == Some(journal::PANIC_TARGET) {
        return is_bande_program_in(entry, "_EXE")
            && entry
                .field("SYSLOG_IDENTIFIER")
                .is_some_and(|program| UNDUMPABLE.contains(&program.as_str()));
    }
    let Some(result) = entry.field("UNIT_RESULT") else {
        return false;
    };
    if !REPORTED_RESULTS.contains(&result.as_str()) {
        return false;
    }
    entry
        .field("USER_UNIT")
        .or_else(|| entry.field("UNIT"))
        .is_some_and(|unit| is_bande_service(&unit))
}

fn is_bande_service(unit: &str) -> bool {
    unit.ends_with(".service") && (unit.starts_with("bande-") || unit.contains("-bande"))
}

fn run() -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(STATE)?;
    let lock = File::create(Path::new(STATE).join("lock"))?;
    flock(&lock, FlockOperation::LockExclusive)?;
    journal::sync();
    for entry in journal::new_failures(&Path::new(STATE).join("cursor"))? {
        if is_reported(&entry)
            && let Err(error) = structured::of_entry(&entry)
        {
            tracing::warn!("cannot keep a fault report: {error}");
        }
    }
    structured::queue_checkin_if_due();
    Ok(())
}

fn main() -> ExitCode {
    bande_log::init("bande-crash-reports");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
