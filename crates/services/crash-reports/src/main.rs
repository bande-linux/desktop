mod journal;
mod structured;

use std::error::Error;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use jiff::Timestamp;
use journal::Entry;
use rustix::fs::{FlockOperation, flock};

const STATE: &str = "/var/lib/bande/crash-reports";
const BOOT_ID: &str = "/proc/sys/kernel/random/boot_id";
const PSTORE_ARCHIVE: &str = "/var/lib/systemd/pstore";
const CHECKED_BOOT: &str = "checked-boot";
const PSTORE_SEEN: &str = "pstore-seen";
const PSTORE_FILE_LIMIT: u64 = 256 * 1024;
const ESP_FOLDERS: [&str; 3] = ["/boot/efi", "/efi", "/boot"];
const FAILED_BOOTS_SEEN: &str = "failed-boots-seen";
const LOADER_ENTRY_SELECTED: &str =
    "/sys/firmware/efi/efivars/LoaderEntrySelected-4a67b082-0a4c-41cf-b6c7-440b29bb8c4f";
const EFIVAR_ATTRIBUTES: usize = 4;
const REPORTED_RESULTS: [&str; 4] = ["exit-code", "timeout", "watchdog", "oom-kill"];
const WATCHED_SYSTEM_UNITS: [&str; 2] = ["greetd.service", "systemd-suspend.service"];
const UNDUMPABLE: [&str; 5] = [
    "bande-clipboard",
    "bande-greeter",
    "bande-lock",
    "bande-polkit-agent",
    "bande-secrets",
];

fn is_installed(entry: &Entry, field: &str) -> bool {
    entry
        .field(field)
        .is_some_and(|path| bande_log::is_installed_program(Path::new(&path)))
}

fn is_reported(entry: &Entry) -> bool {
    if entry.field("MESSAGE_ID").as_deref() == Some(journal::COREDUMP_MESSAGE) {
        return is_installed(entry, "COREDUMP_EXE");
    }
    if entry.field("TARGET").as_deref() == Some(journal::PANIC_TARGET) {
        return is_installed(entry, "_EXE")
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
    match (entry.field("USER_UNIT"), entry.field("UNIT")) {
        (Some(unit), _) => is_bande_service(&unit),
        (None, Some(unit)) => {
            is_bande_service(&unit) || WATCHED_SYSTEM_UNITS.contains(&unit.as_str())
        }
        _ => false,
    }
}

fn is_bande_service(unit: &str) -> bool {
    unit.ends_with(".service") && (unit.starts_with("bande-") || unit.contains("-bande"))
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

fn read_state(name: &str) -> Option<String> {
    fs::read_to_string(Path::new(STATE).join(name)).ok()
}

fn replace_state(name: &str, contents: &str) -> Result<(), Box<dyn Error>> {
    let staged = Path::new(STATE).join(format!("{name}.new"));
    fs::write(&staged, contents)?;
    fs::rename(staged, Path::new(STATE).join(name))?;
    Ok(())
}

fn counted_entry(name: &str) -> Option<(&str, u32, u32)> {
    let (entry, counter) = name.strip_suffix(".conf")?.rsplit_once('+')?;
    let (left, done) = counter.split_once('-')?;
    Some((entry, left.parse().ok()?, done.parse().ok()?))
}

fn selected_entry() -> String {
    let bytes = fs::read(LOADER_ENTRY_SELECTED).unwrap_or_default();
    let units: Vec<u16> = bytes
        .get(EFIVAR_ATTRIBUTES..)
        .unwrap_or_default()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();
    let id = String::from_utf16_lossy(&units);
    let file = format!("{}.conf", id.trim_end_matches(".conf"));
    counted_entry(&file).map_or_else(
        || file.trim_end_matches(".conf").to_owned(),
        |(entry, ..)| entry.to_owned(),
    )
}

fn check_failed_boots() -> Result<(), Box<dyn Error>> {
    let Some(folder) = ESP_FOLDERS
        .iter()
        .map(|esp| Path::new(esp).join("loader/entries"))
        .find(|folder| folder.is_dir())
    else {
        return Ok(());
    };
    let selected = selected_entry();
    let seen = read_state(FAILED_BOOTS_SEEN).unwrap_or_default();
    let mut names: Vec<String> = fs::read_dir(&folder)?
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    let mut failed = String::new();
    for name in names {
        let Some((entry, left, done)) = counted_entry(&name) else {
            continue;
        };
        let failed_tries = if entry == selected {
            done.saturating_sub(1)
        } else if left == 0 {
            done
        } else {
            0
        };
        if failed_tries == 0 {
            continue;
        }
        failed.push_str(&format!("{name}\n"));
        if !seen.lines().any(|line| line == name) {
            structured::failed_boot(entry, failed_tries, &selected)?;
        }
    }
    replace_state(FAILED_BOOTS_SEEN, &failed)
}

fn check_previous_boot() -> Result<(), Box<dyn Error>> {
    let boot = fs::read_to_string(BOOT_ID)?;
    if read_state(CHECKED_BOOT).as_deref() == Some(boot.as_str()) {
        return Ok(());
    }
    let seen = read_state(PSTORE_SEEN).unwrap_or_default();
    let panics = new_pstore_files(Path::new(PSTORE_ARCHIVE), &seen);
    if !panics.is_empty() || !journal::previous_boot_ended_cleanly() {
        let time = journal::previous_boot_end().unwrap_or_else(Timestamp::now);
        structured::of_previous_boot(&kernel_log(&panics), time)?;
    }
    if let Err(error) = check_failed_boots() {
        tracing::warn!("cannot check for failed boots: {error}");
    }
    if !panics.is_empty() {
        let mut seen = seen;
        for file in &panics {
            seen.push_str(&format!("{}\n", file.display()));
        }
        replace_state(PSTORE_SEEN, &seen)?;
    }
    replace_state(CHECKED_BOOT, &boot)
}

fn run() -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(STATE)?;
    let lock = File::create(Path::new(STATE).join("lock"))?;
    flock(&lock, FlockOperation::LockExclusive)?;
    journal::sync();
    if let Err(error) = check_previous_boot() {
        tracing::warn!("cannot check how the last boot ended: {error}");
    }
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
