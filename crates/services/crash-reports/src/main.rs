mod journal;
mod report;
mod store;

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use jiff::Timestamp;
use journal::Entry;
use report::{Kind, Owner, Report};
use store::Store;

const STATE: &str = "/var/lib/bande/crash-reports";
const BOOT_ID: &str = "/proc/sys/kernel/random/boot_id";
const PSTORE_ARCHIVE: &str = "/var/lib/systemd/pstore";
const CHECKED_BOOT: &str = "checked-boot";
const PSTORE_SEEN: &str = "pstore-seen";
const PSTORE_FILE_LIMIT: u64 = 256 * 1024;
const FIRST_USER: u32 = 1000;
const NOBODY: u32 = 65534;
const QUIT_SIGNAL: &str = "SIGQUIT";
const REPORTED_RESULTS: [&str; 4] = ["exit-code", "timeout", "watchdog", "oom-kill"];
const WATCHED_SYSTEM_UNITS: [&str; 1] = ["greetd.service"];
const UNDUMPABLE: [&str; 5] = [
    "bande-clipboard",
    "bande-greeter",
    "bande-lock",
    "bande-polkit-agent",
    "bande-secrets",
];

fn owner_of(uid: u32) -> Owner {
    if (FIRST_USER..NOBODY).contains(&uid) {
        Owner::User(uid)
    } else {
        Owner::System
    }
}

fn crash(entry: &Entry) -> Option<Report> {
    let path = entry.field("COREDUMP_EXE");
    let program = entry
        .field("COREDUMP_COMM")
        .or_else(|| program_name(path.as_deref()?))?;
    let uid = entry.number("COREDUMP_UID")?;
    let pid = entry.field("COREDUMP_PID")?;
    let signal = entry.field("COREDUMP_SIGNAL_NAME").unwrap_or_default();
    let time = entry
        .timestamp("COREDUMP_TIMESTAMP")
        .unwrap_or_else(|| entry.time());
    let kind = if signal == QUIT_SIGNAL {
        Kind::NotResponding
    } else {
        Kind::Crash
    };
    let mut report = Report::new(kind, owner_of(uid), program, time, &pid);
    report.detail("Path", path.as_deref());
    report.detail("Command", entry.field("COREDUMP_CMDLINE").as_deref());
    report.detail(
        "Package",
        path.as_deref().and_then(journal::package_of).as_deref(),
    );
    report.detail("Signal", Some(&signal));
    report.detail("Panic", journal::panic_of(&pid, time).as_deref());
    report.detail(
        "Unit",
        entry
            .field("COREDUMP_USER_UNIT")
            .or_else(|| entry.field("COREDUMP_UNIT"))
            .as_deref(),
    );
    match entry.field("COREDUMP_FILENAME") {
        Some(core) => report.detail(
            "Core dump",
            Some(&format!("{core} (coredumpctl debug {pid})")),
        ),
        None => report.detail("Core dump", Some("not kept")),
    }
    report.section(
        "Stack trace",
        entry
            .field("MESSAGE")
            .as_deref()
            .map_or_else(String::new, report::stack_trace),
    );
    report.section(
        "Last log lines",
        journal::recent_lines(owner_of(uid), &pid, time),
    );
    Some(report)
}

fn program_name(path: &str) -> Option<String> {
    Path::new(path).file_name()?.to_str().map(str::to_owned)
}

fn failed_unit(entry: &Entry) -> Option<Report> {
    let result = entry.field("UNIT_RESULT")?;
    if !REPORTED_RESULTS.contains(&result.as_str()) {
        return None;
    }
    let (unit, owner) = match (entry.field("USER_UNIT"), entry.field("UNIT")) {
        (Some(unit), _) if is_bande_service(&unit) => (unit, owner_of(entry.number("_UID")?)),
        (None, Some(unit))
            if is_bande_service(&unit) || WATCHED_SYSTEM_UNITS.contains(&unit.as_str()) =>
        {
            (unit, Owner::System)
        }
        _ => return None,
    };
    let time = entry.time();
    let pid = entry.field("_PID").unwrap_or_default();
    let mut report = Report::new(Kind::Failed, owner, unit.clone(), time, &pid);
    report.detail("Unit", Some(&unit));
    report.detail("Result", Some(&result));
    report.section("Last log lines", journal::unit_lines(owner, &unit, time));
    Some(report)
}

fn is_bande_service(unit: &str) -> bool {
    unit.ends_with(".service") && (unit.starts_with("bande-") || unit.contains("-bande"))
}

fn undumpable_panic(entry: &Entry) -> Option<Report> {
    let program = entry.field("SYSLOG_IDENTIFIER")?;
    if !UNDUMPABLE.contains(&program.as_str()) {
        return None;
    }
    let owner = owner_of(entry.number("_UID")?);
    let time = entry.time();
    let pid = entry.field("_PID").unwrap_or_default();
    let mut report = Report::new(Kind::Crash, owner, program, time, &pid);
    report.detail("Panic", entry.field("MESSAGE").as_deref());
    report.detail("Core dump", Some("none: this program holds passwords"));
    report.section("Last log lines", journal::recent_lines(owner, &pid, time));
    Some(report)
}

fn report_of(entry: &Entry) -> Option<Report> {
    if entry.field("MESSAGE_ID").as_deref() == Some(journal::COREDUMP_MESSAGE) {
        crash(entry)
    } else if entry.field("TARGET").as_deref() == Some(journal::PANIC_TARGET) {
        undumpable_panic(entry)
    } else {
        failed_unit(entry)
    }
}

fn new_pstore_files(root: &Path, seen: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => folders.push(path),
                Ok(kind) if kind.is_file() && !seen.lines().any(|line| Path::new(line) == path) => {
                    found.push(path);
                }
                _ => {}
            }
        }
    }
    found.sort();
    found
}

fn kernel_log(files: &[PathBuf]) -> String {
    let mut log = String::new();
    for file in files {
        let Ok(size) = fs::metadata(file).map(|metadata| metadata.len()) else {
            continue;
        };
        if size > PSTORE_FILE_LIMIT {
            continue;
        }
        if let Ok(bytes) = fs::read(file) {
            log.push_str(&format!("--- {}\n", file.display()));
            log.push_str(&String::from_utf8_lossy(&bytes));
            log.push('\n');
        }
    }
    log
}

fn check_previous_boot(store: &Store) -> Result<(), Box<dyn Error>> {
    let boot = fs::read_to_string(BOOT_ID)?;
    if store.read(CHECKED_BOOT).as_deref() == Some(boot.as_str()) {
        return Ok(());
    }
    let seen = store.read(PSTORE_SEEN).unwrap_or_default();
    let panics = new_pstore_files(Path::new(PSTORE_ARCHIVE), &seen);
    let ended_cleanly = journal::previous_boot_ended_cleanly();
    if !panics.is_empty() || !ended_cleanly {
        let time = journal::previous_boot_end().unwrap_or_else(Timestamp::now);
        let kind = if panics.is_empty() {
            Kind::UnexpectedRestart
        } else {
            Kind::KernelPanic
        };
        let mut report = Report::new(kind, Owner::System, "kernel".to_owned(), time, "0");
        if !panics.is_empty() {
            report.section("Kernel log saved across the restart", kernel_log(&panics));
        }
        report.section(
            "Last log lines of that boot",
            journal::previous_boot_lines(),
        );
        store.write(&report)?;
    }
    if !panics.is_empty() {
        let mut seen = seen;
        for file in &panics {
            seen.push_str(&format!("{}\n", file.display()));
        }
        store.replace(PSTORE_SEEN, &seen)?;
    }
    store.replace(CHECKED_BOOT, &boot)
}

fn run() -> Result<(), Box<dyn Error>> {
    let store = Store::open(Path::new(STATE))?;
    let _lock = store.lock()?;
    journal::sync();
    if let Err(error) = check_previous_boot(&store) {
        tracing::warn!("cannot check how the last boot ended: {error}");
    }
    for entry in journal::new_failures(&store.path("cursor"))? {
        let Some(report) = report_of(&entry) else {
            continue;
        };
        if let Err(error) = store.write(&report) {
            tracing::warn!("cannot write a report of {}: {error}", report.program());
        }
    }
    store.prune();
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
