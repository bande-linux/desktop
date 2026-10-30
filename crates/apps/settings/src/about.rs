use std::path::Path;
use std::sync::mpsc::{self, Sender};

use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

use crate::follow::{Update, Updates, spawn};
use crate::storage::disk;

const HOSTNAME: &str = "org.freedesktop.hostname1";
const HOSTNAME_PATH: &str = "/org/freedesktop/hostname1";
const MAX_HOSTNAME: usize = 63;
const GIBIBYTE: f64 = 1024.0 * 1024.0 * 1024.0;
const APPLE_IMPLEMENTER: &str = "0x61";

#[zbus::proxy(
    interface = "org.freedesktop.hostname1",
    default_service = "org.freedesktop.hostname1",
    default_path = "/org/freedesktop/hostname1"
)]
trait Hostname {
    #[zbus(property)]
    fn hostname(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn pretty_hostname(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn hardware_vendor(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn hardware_model(&self) -> zbus::Result<String>;
    fn set_pretty_hostname(&self, name: &str, interactive: bool) -> zbus::Result<()>;
    fn set_static_hostname(&self, name: &str, interactive: bool) -> zbus::Result<()>;
    fn get_hardware_serial(&self) -> zbus::Result<String>;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct About {
    pub name: String,
    pub model: String,
    pub chip: String,
    pub memory: u64,
    pub capacity: u64,
    pub available: u64,
    pub unbound: String,
    pub system: String,
    pub serial: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AboutCommand {
    Rename(String),
}

pub fn chip_name(compatible: &str, cpuinfo: &str) -> String {
    let soc = compatible
        .split('\0')
        .find_map(|entry| entry.strip_prefix("apple,t"));
    let name = match soc {
        Some("8103") => Some("Apple M1"),
        Some("6000") => Some("Apple M1 Pro"),
        Some("6001") => Some("Apple M1 Max"),
        Some("6002") => Some("Apple M1 Ultra"),
        Some("8112") => Some("Apple M2"),
        Some("6020") => Some("Apple M2 Pro"),
        Some("6021") => Some("Apple M2 Max"),
        Some("6022") => Some("Apple M2 Ultra"),
        _ => None,
    };
    if let Some(name) = name {
        return name.to_owned();
    }
    let field = |name: &str| {
        cpuinfo.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == name).then(|| value.trim().to_owned())
        })
    };
    let cores = cpuinfo
        .lines()
        .filter(|line| {
            line.split(':')
                .next()
                .is_some_and(|key| key.trim() == "processor")
        })
        .count();
    let family = match field("model name") {
        Some(model) => model,
        None if field("CPU implementer").as_deref() == Some(APPLE_IMPLEMENTER) => {
            "Apple silicon".to_owned()
        }
        None => std::env::consts::ARCH.to_owned(),
    };
    if cores > 1 {
        format!("{family} ({cores} cores)")
    } else {
        family
    }
}

pub fn memory_bytes(meminfo: &str) -> u64 {
    let kilobytes = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|value| {
            value
                .trim()
                .trim_end_matches("kB")
                .trim()
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0);
    let gibibytes = (kilobytes as f64 * 1024.0 / GIBIBYTE).ceil();
    (gibibytes * 1e9) as u64
}

pub fn system_version(os_release: &str, debian_version: Option<&str>) -> String {
    let field = |name: &str| {
        os_release.lines().find_map(|line| {
            let value = line.strip_prefix(name)?.strip_prefix('=')?;
            Some(value.trim().trim_matches('"').to_owned())
        })
    };
    let point = debian_version
        .map(str::trim)
        .filter(|version| version.starts_with(|c: char| c.is_ascii_digit()));
    match (field("ID").as_deref(), point, field("VERSION_CODENAME")) {
        (Some("debian"), Some(version), Some(codename)) => format!("Debian {version} ({codename})"),
        _ => field("PRETTY_NAME").unwrap_or_default(),
    }
}

pub fn static_hostname(name: &str) -> String {
    let mut hostname = String::new();
    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            hostname.push(character);
        } else if (character.is_whitespace() || character == '-' || character == '_')
            && !hostname.ends_with('-')
        {
            hostname.push('-');
        }
    }
    let hostname = hostname.trim_matches('-');
    let hostname: String = hostname.chars().take(MAX_HOSTNAME).collect();
    hostname.trim_end_matches('-').to_owned()
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn model(hostname: &HostnameProxyBlocking<'_>) -> String {
    if let Some(model) = read("/proc/device-tree/model") {
        return model.trim_end_matches('\0').to_owned();
    }
    let vendor = hostname.hardware_vendor().unwrap_or_default();
    let model = hostname.hardware_model().unwrap_or_default();
    if model.starts_with(&vendor) {
        model
    } else {
        format!("{vendor} {model}").trim().to_owned()
    }
}

fn serial(hostname: &HostnameProxyBlocking<'_>) -> Option<String> {
    read("/proc/device-tree/serial-number")
        .map(|serial| serial.trim_end_matches('\0').trim().to_owned())
        .or_else(|| hostname.get_hardware_serial().ok())
        .filter(|serial| !serial.is_empty())
}

fn read_about(hostname: &HostnameProxyBlocking<'_>) -> About {
    let (capacity, available) = disk(Path::new("/"));
    let pretty = hostname.pretty_hostname().unwrap_or_default();
    About {
        name: if pretty.is_empty() {
            hostname.hostname().unwrap_or_default()
        } else {
            pretty
        },
        model: model(hostname),
        chip: chip_name(
            &read("/proc/device-tree/compatible").unwrap_or_default(),
            &read("/proc/cpuinfo").unwrap_or_default(),
        ),
        memory: memory_bytes(&read("/proc/meminfo").unwrap_or_default()),
        capacity,
        available,
        unbound: env!("CARGO_PKG_VERSION").to_owned(),
        system: system_version(
            &read("/etc/os-release").unwrap_or_default(),
            read("/etc/debian_version").as_deref(),
        ),
        serial: serial(hostname),
    }
}

fn rename(hostname: &HostnameProxyBlocking<'_>, name: &str) -> zbus::Result<()> {
    hostname.set_pretty_hostname(name, true)?;
    let fallback = static_hostname(name);
    if fallback.is_empty() {
        return Ok(());
    }
    hostname.set_static_hostname(&fallback, true)
}

pub fn follow_about(system: &Connection, updates: Updates) -> Sender<AboutCommand> {
    let (sender, receiver) = mpsc::channel();
    let system = system.clone();
    let reader = system.clone();
    let feed = updates.clone();
    spawn("about", move || {
        let follow = || -> zbus::Result<()> {
            let hostname = HostnameProxyBlocking::builder(&reader)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()?;
            let changes = PropertiesProxy::builder(&reader)
                .destination(HOSTNAME)?
                .path(HOSTNAME_PATH)?
                .build()?
                .receive_properties_changed()?;
            if !feed.send(Update::About(read_about(&hostname))) {
                return Ok(());
            }
            for _ in changes {
                if !feed.send(Update::About(read_about(&hostname))) {
                    break;
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow hostnamed: {error}");
        }
    });
    spawn("about-worker", move || {
        let Ok(hostname) = HostnameProxyBlocking::new(&system) else {
            return;
        };
        for command in receiver {
            match command {
                AboutCommand::Rename(name) => {
                    if let Err(error) = rename(&hostname, &name) {
                        updates.send(Update::Failed(error.to_string()));
                        updates.send(Update::About(read_about(&hostname)));
                    }
                }
            }
        }
    });
    sender
}
