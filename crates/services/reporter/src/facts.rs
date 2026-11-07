use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rustix::fs::{Mode, OFlags};
use serde_json::{Map, Value, json};

use crate::scrub::Scrubber;

const RING_LIMIT: u64 = 256 * 1024;
const RING_HEADER: usize = 64;
const SESSION_LIMIT: u64 = 64 * 1024;
const MEMORY_FIELDS: [&str; 6] = [
    "MemTotal",
    "MemAvailable",
    "SwapTotal",
    "SwapFree",
    "Shmem",
    "Slab",
];
const PRESSURES: [&str; 3] = ["cpu", "memory", "io"];
const OTHER_PACKAGES: [&str; 5] = [
    "linux-image-*",
    "mesa-vulkan-drivers",
    "libgl1-mesa-dri",
    "firmware-*",
    "systemd",
];
const MODEL_FILES: [&str; 2] = ["/proc/device-tree/model", "/sys/class/dmi/id/product_name"];
const FIRMWARE_FILES: [&str; 3] = [
    "/proc/device-tree/chosen/asahi,os-fw-version",
    "/proc/device-tree/chosen/asahi,system-fw-version",
    "/sys/class/dmi/id/bios_version",
];
const DRM: &str = "/sys/class/drm";
const JOURNAL_LINES: &str = "200";
const OOM_TOP: usize = 12;

fn read_trimmed(path: &str) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_matches(['\0', '\n', ' ']);
    (!text.is_empty()).then(|| text.to_owned())
}

fn first_of(paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| read_trimmed(path))
}

fn command_text(program: &str, arguments: &[&str]) -> String {
    Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

fn package_versions() -> Value {
    let mut packages = Map::new();
    let mut arguments = vec![
        "-W",
        "-f",
        "${db:Status-Abbrev} ${Package} ${Version}\\n",
        "bande*",
    ];
    arguments.extend(OTHER_PACKAGES);
    for line in command_text("dpkg-query", &arguments).lines() {
        let mut words = line.split_whitespace();
        if words.next() == Some("ii")
            && let (Some(name), Some(version)) = (words.next(), words.next())
        {
            packages.insert(name.to_owned(), json!(version));
        }
    }
    Value::Object(packages)
}

fn memory() -> Value {
    let text = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut memory = Map::new();
    for line in text.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if MEMORY_FIELDS.contains(&name) {
            memory.insert(name.to_owned(), json!(value.trim()));
        }
    }
    Value::Object(memory)
}

fn pressure() -> Value {
    let mut pressure = Map::new();
    for name in PRESSURES {
        if let Ok(text) = fs::read_to_string(format!("/proc/pressure/{name}")) {
            pressure.insert(name.to_owned(), json!(text.trim()));
        }
    }
    Value::Object(pressure)
}

fn displays() -> Value {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(DRM) else {
        return Value::Array(found);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(status) = fs::read_to_string(path.join("status")) else {
            continue;
        };
        if status.trim() == "connected" {
            let modes = fs::read_to_string(path.join("modes")).unwrap_or_default();
            found.push(json!({
                "connector": name,
                "preferred_mode": modes.lines().next(),
            }));
        }
    }
    Value::Array(found)
}

pub fn boot_id() -> String {
    read_trimmed("/proc/sys/kernel/random/boot_id")
        .unwrap_or_default()
        .replace('-', "")
}

pub fn boot_id_of(boot: &str) -> String {
    let offset: i32 = boot.parse().unwrap_or(0);
    command_text("journalctl", &["--list-boots", "--no-pager", "--no-legend"])
        .lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next()?.parse() == Ok(offset)).then(|| words.next().map(str::to_owned))?
        })
        .unwrap_or_default()
        .replace('-', "")
}

pub fn uptime_seconds() -> Option<u64> {
    let text = fs::read_to_string("/proc/uptime").ok()?;
    let seconds: f64 = text.split_whitespace().next()?.parse().ok()?;
    Some(seconds as u64)
}

pub fn machine() -> Map<String, Value> {
    let mut facts = Map::new();
    facts.insert("packages".to_owned(), package_versions());
    facts.insert(
        "kernel".to_owned(),
        json!(read_trimmed("/proc/sys/kernel/osrelease")),
    );
    facts.insert("model".to_owned(), json!(first_of(&MODEL_FILES)));
    facts.insert("firmware".to_owned(), json!(first_of(&FIRMWARE_FILES)));
    facts.insert("displays".to_owned(), displays());
    facts.insert("memory".to_owned(), memory());
    facts.insert("pressure".to_owned(), pressure());
    facts.insert("uptime_seconds".to_owned(), json!(uptime_seconds()));
    facts.insert(
        "failed_units".to_owned(),
        json!(
            command_text(
                "systemctl",
                &["--failed", "--plain", "--no-legend", "--full"]
            )
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .collect::<Vec<_>>()
        ),
    );
    facts
}

fn runtime_dir(uid: u32) -> PathBuf {
    PathBuf::from(format!("/run/user/{uid}/bande"))
}

fn read_user_bytes(path: &Path, uid: u32, limit: u64) -> Option<Vec<u8>> {
    let file = File::options()
        .read(true)
        .custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC).bits() as i32)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.uid() != uid || metadata.mode() & Mode::RWXO.bits() != 0 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn read_user_file(path: &Path, uid: u32, limit: u64) -> Option<String> {
    let bytes = read_user_bytes(path, uid, limit)?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_ring(path: &Path, uid: u32) -> Option<String> {
    let bytes = read_user_bytes(path, uid, RING_LIMIT)?;
    Some(String::from_utf8_lossy(bytes.get(RING_HEADER..)?).into_owned())
}

pub fn ring_lines(uid: u32, program: &str, pid: Option<&str>) -> String {
    let path = runtime_dir(uid).join("ring").join(program);
    let Some(text) = read_ring(&path, uid) else {
        return String::new();
    };
    let wanted = pid.map(|pid| format!("{pid} "));
    let mut lines: Vec<&str> = text
        .split(['\n', '\0'])
        .filter(|line| {
            wanted
                .as_ref()
                .is_none_or(|wanted| line.starts_with(wanted))
        })
        .filter(|line| !line.is_empty())
        .collect();
    lines.sort_by_key(|line| {
        line.split_whitespace()
            .nth(1)
            .and_then(|time| time.parse::<u64>().ok())
            .unwrap_or_default()
    });
    lines.join("\n")
}

pub fn session(uid: u32, facts: &mut Map<String, Value>) {
    let path = runtime_dir(uid).join("session.json");
    if let Some(session) = read_user_file(&path, uid, SESSION_LIMIT)
        .and_then(|text| serde_json::from_str::<Map<String, Value>>(&text).ok())
    {
        for (name, value) in session {
            if value.is_number() || value.is_boolean() || value.is_string() || value.is_array() {
                facts.insert(format!("session_{name}"), value);
            }
        }
    }
    if let Some(actions) = read_ring(&runtime_dir(uid).join("ring/actions"), uid) {
        let mut actions: Vec<&str> = actions
            .split(['\n', '\0'])
            .filter(|line| !line.is_empty())
            .collect();
        actions.sort_by_key(|line| {
            line.split_whitespace()
                .nth(1)
                .and_then(|time| time.parse::<u64>().ok())
                .unwrap_or_default()
        });
        let names: Vec<&str> = actions
            .iter()
            .filter_map(|line| line.split_whitespace().last())
            .collect();
        facts.insert(
            "last_actions".to_owned(),
            json!(names[names.len().saturating_sub(32)..]),
        );
    }
}

pub fn journal_around(boot: Option<&str>, around: Option<i64>, scrubber: &Scrubber) -> String {
    let mut arguments: Vec<String> = ["--no-pager", "--quiet", "--output=short-iso", "--all"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    arguments.extend([
        "--priority=warning".to_owned(),
        "-n".to_owned(),
        JOURNAL_LINES.to_owned(),
    ]);
    arguments.push(format!("--boot={}", boot.unwrap_or("0")));
    if let Some(second) = around {
        arguments.push(format!("--since=@{}", second - 300));
        arguments.push(format!("--until=@{}", second + 5));
    }
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let lines: Vec<String> = command_text("journalctl", &arguments)
        .lines()
        .filter(|line| !line.starts_with(char::is_whitespace))
        .map(|line| scrubber.line(line))
        .collect();
    lines.join("\n")
}

pub fn largest_processes() -> Value {
    let mut sizes: Vec<(u64, String)> = Vec::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Value::Array(Vec::new());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(pid) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(status) = fs::read_to_string(path.join("status")) else {
            continue;
        };
        let name = status
            .lines()
            .find_map(|line| line.strip_prefix("Name:"))
            .map_or("", str::trim);
        let resident = status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|size| size.split_whitespace().next())
            .and_then(|size| size.parse().ok())
            .unwrap_or(0);
        sizes.push((resident, name.to_owned()));
    }
    sizes.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
    Value::Array(
        sizes
            .into_iter()
            .take(OOM_TOP)
            .map(|(size, name)| json!({"name": name, "resident_kib": size}))
            .collect(),
    )
}

pub fn counters_of(spool: &Path) -> Value {
    fs::read(spool.join("counters.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| json!({}))
}
