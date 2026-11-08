use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use jiff::Timestamp;
use rustix::fs::statvfs;
use serde_json::{Map, Value, json};

use crate::facts;

const REPORTS: &str = "reports";
const OUTBOX: &str = "outbox";
const STAGING: &str = "tmp";
const INSTALL_ID: &str = "machine";
pub const MANIFEST: &str = "manifest.json";
pub const STATE: &str = "state.json";
const UUID_SOURCE: &str = "/proc/sys/kernel/random/uuid";
const OS_RELEASE: &str = "/etc/os-release";
const KERNEL_RELEASE: &str = "/proc/sys/kernel/osrelease";
const DESKTOP_PACKAGE: &str = "bande-desktop";

const KEEP_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const KEEP_REPORTS: usize = 200;
const KEEP_BYTES: u64 = 256 * 1024 * 1024;
const REPORT_BYTES: u64 = 16 * 1024 * 1024;
const KEEP_FREE_BYTES: u64 = 512 * 1024 * 1024;
const ANY_STATE_WINDOW: Duration = Duration::from_secs(60 * 60);
const EVERY_ONE_KEPT: [&str; 3] = ["unexpected-restart", "kernel-panic", "manual"];
const UNSENT_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);

pub struct Attachment {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Local,
    Sent,
    Rejected,
}

pub struct Stored {
    pub id: String,
    pub path: PathBuf,
    pub manifest: Map<String, Value>,
    pub status: Status,
    pub occurrences: u64,
    pub stored_at: SystemTime,
    pub bytes: u64,
}

pub struct Spool {
    root: PathBuf,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

pub fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn sync_folder(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn replace_synced(folder: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let staged = folder.join(format!(".{name}.new"));
    write_synced(&staged, bytes)?;
    fs::rename(&staged, folder.join(name))?;
    sync_folder(folder)
}

fn new_uuid() -> io::Result<String> {
    Ok(fs::read_to_string(UUID_SOURCE)?.trim().to_owned())
}

fn os_version() -> String {
    let release = fs::read_to_string(OS_RELEASE).unwrap_or_default();
    let name = release
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .map_or("unknown", |name| name.trim_matches('"'));
    let kernel = fs::read_to_string(KERNEL_RELEASE).unwrap_or_default();
    format!("{name}, kernel {}", kernel.trim())
}

fn folder_bytes(path: &Path) -> u64 {
    fs::read_dir(path).map_or(0, |entries| {
        entries
            .flatten()
            .filter_map(|entry| entry.metadata().ok())
            .map(|metadata| metadata.len())
            .sum()
    })
}

fn free_bytes(path: &Path) -> Option<u64> {
    let stats = statvfs(path).ok()?;
    Some(stats.f_bavail.saturating_mul(stats.f_frsize))
}

impl Spool {
    pub fn open(root: &Path) -> io::Result<Self> {
        for name in [REPORTS, OUTBOX, STAGING] {
            fs::create_dir_all(root.join(name))?;
        }
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    pub fn outbox(&self) -> PathBuf {
        self.root.join(OUTBOX)
    }

    fn report_path(&self, id: &str) -> io::Result<PathBuf> {
        if !is_plain_name(id) {
            return Err(invalid(format!("not a report id: {id}")));
        }
        Ok(self.root.join(REPORTS).join(id))
    }

    fn install_id(&self) -> io::Result<String> {
        let path = self.root.join(INSTALL_ID);
        match fs::read_to_string(&path) {
            Ok(id) if !id.trim().is_empty() => return Ok(id.trim().to_owned()),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let id = new_uuid()?;
        replace_synced(&self.root, INSTALL_ID, id.as_bytes())?;
        Ok(id)
    }

    fn fill_manifest(&self, manifest: &mut Map<String, Value>) -> io::Result<String> {
        let id = new_uuid()?;
        manifest.insert("id".to_owned(), json!(id));
        manifest.insert("machine".to_owned(), json!(self.install_id()?));
        manifest
            .entry("time")
            .or_insert_with(|| json!(Timestamp::now().to_string()));
        manifest
            .entry("os_version")
            .or_insert_with(|| json!(os_version()));
        if !manifest.contains_key("desktop_version")
            && let Some(version) = facts::installed_version(DESKTOP_PACKAGE)
        {
            manifest.insert("desktop_version".to_owned(), json!(version));
        }
        manifest
            .entry("build_ids")
            .or_insert_with(|| Value::Array(Vec::new()));
        manifest
            .entry("arch")
            .or_insert_with(|| json!(std::env::consts::ARCH));
        for required in ["signature", "kind"] {
            if !manifest.get(required).is_some_and(Value::is_string) {
                return Err(invalid(format!("the manifest needs {required}")));
            }
        }
        Ok(id)
    }

    fn trim_to_quota(
        &self,
        manifest: &mut Map<String, Value>,
        attachments: Vec<Attachment>,
    ) -> Vec<Attachment> {
        let manifest_size = serde_json::to_vec(manifest).map_or(0, |bytes| bytes.len() as u64);
        let mut room = REPORT_BYTES.saturating_sub(manifest_size);
        if free_bytes(&self.root).is_some_and(|free| free < KEEP_FREE_BYTES) {
            room = 0;
        }
        let mut attachments = attachments;
        attachments.sort_by_key(|attachment| attachment.bytes.len());
        let mut kept = Vec::new();
        let mut dropped = Vec::new();
        for attachment in attachments {
            let size = attachment.bytes.len() as u64;
            if size <= room {
                room -= size;
                kept.push(attachment);
            } else {
                dropped.push(attachment.name);
            }
        }
        if !dropped.is_empty() {
            manifest.insert("attachments_dropped".to_owned(), json!(dropped));
        }
        kept
    }

    pub fn store(
        &self,
        mut manifest: Map<String, Value>,
        attachments: Vec<Attachment>,
        automatic: bool,
    ) -> io::Result<String> {
        let collapses = manifest
            .get("kind")
            .and_then(Value::as_str)
            .is_none_or(|kind| !EVERY_ONE_KEPT.contains(&kind));
        if let Some(existing) = self
            .repeat_of(manifest.get("signature"))
            .filter(|_| collapses)
        {
            self.count_repeat(&existing)?;
            if automatic && existing.status == Status::Sent {
                self.queue(&existing.id)?;
            }
            return Ok(existing.id);
        }
        let id = self.fill_manifest(&mut manifest)?;
        let attachments = self.trim_to_quota(&mut manifest, attachments);
        let staged = self.root.join(STAGING).join(&id);
        fs::create_dir(&staged)?;
        write_synced(&staged.join(MANIFEST), &serde_json::to_vec(&manifest)?)?;
        for attachment in &attachments {
            if !is_plain_name(&attachment.name) || attachment.name == MANIFEST {
                return Err(invalid(format!("not an attachment: {}", attachment.name)));
            }
            write_synced(&staged.join(&attachment.name), &attachment.bytes)?;
        }
        sync_folder(&staged)?;
        let reports = self.root.join(REPORTS);
        fs::rename(&staged, reports.join(&id))?;
        sync_folder(&reports)?;
        if automatic {
            self.queue(&id)?;
        }
        self.prune();
        Ok(id)
    }

    pub fn queue(&self, id: &str) -> io::Result<()> {
        if !self.report_path(id)?.is_dir() {
            return Err(invalid(format!("no such report: {id}")));
        }
        write_synced(&self.outbox().join(id), b"")?;
        sync_folder(&self.outbox())
    }

    pub fn waiting(&self) -> io::Result<Vec<String>> {
        let mut waiting: Vec<(SystemTime, String)> = Vec::new();
        for entry in fs::read_dir(self.outbox())?.flatten() {
            let name = entry.file_name();
            match name.to_str().filter(|name| is_plain_name(name)) {
                Some(name) => {
                    if let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified())
                    {
                        waiting.push((modified, name.to_owned()));
                    }
                }
                None => {
                    if let Err(error) = fs::remove_file(entry.path()) {
                        tracing::warn!("cannot remove a stray file from the outbox: {error}");
                    }
                }
            }
        }
        waiting.sort();
        Ok(waiting.into_iter().map(|(_, name)| name).collect())
    }

    fn read_state(path: &Path) -> Map<String, Value> {
        fs::read(path.join(STATE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn read(&self, id: &str) -> io::Result<Stored> {
        let path = self.report_path(id)?;
        let manifest: Map<String, Value> = serde_json::from_slice(&fs::read(path.join(MANIFEST))?)?;
        let state = Self::read_state(&path);
        let status = match state.get("status").and_then(Value::as_str) {
            Some("sent") => Status::Sent,
            Some("rejected") => Status::Rejected,
            _ => Status::Local,
        };
        Ok(Stored {
            id: id.to_owned(),
            manifest,
            status,
            occurrences: state
                .get("occurrences")
                .and_then(Value::as_u64)
                .unwrap_or(1),
            stored_at: fs::metadata(path.join(MANIFEST))?.modified()?,
            bytes: folder_bytes(&path),
            path,
        })
    }

    pub fn all(&self) -> Vec<Stored> {
        let Ok(entries) = fs::read_dir(self.root.join(REPORTS)) else {
            return Vec::new();
        };
        let mut all: Vec<Stored> = entries
            .flatten()
            .filter_map(|entry| self.read(entry.file_name().to_str()?).ok())
            .collect();
        all.sort_by_key(|report| report.stored_at);
        all
    }

    fn repeat_of(&self, signature: Option<&Value>) -> Option<Stored> {
        let signature = signature?;
        let now = SystemTime::now();
        self.all().into_iter().rev().find(|report| {
            let age = now
                .duration_since(report.stored_at)
                .unwrap_or(Duration::ZERO);
            report.manifest.get("signature") == Some(signature)
                && report.status != Status::Rejected
                && (age < ANY_STATE_WINDOW
                    || (age < UNSENT_WINDOW && report.status == Status::Local))
        })
    }

    fn count_repeat(&self, report: &Stored) -> io::Result<()> {
        let mut state = Self::read_state(&report.path);
        state.insert("occurrences".to_owned(), json!(report.occurrences + 1));
        state.insert("last_seen".to_owned(), json!(Timestamp::now().to_string()));
        replace_synced(&report.path, STATE, &serde_json::to_vec(&state)?)
    }

    pub fn settle(&self, id: &str, status: Status) -> io::Result<()> {
        let path = self.report_path(id)?;
        if !path.is_dir() {
            return self.unqueue(id);
        }
        let mut state = Self::read_state(&path);
        let name = match status {
            Status::Sent => "sent",
            Status::Rejected => "rejected",
            Status::Local => "local",
        };
        state.insert("status".to_owned(), json!(name));
        state.insert("settled".to_owned(), json!(Timestamp::now().to_string()));
        replace_synced(&path, STATE, &serde_json::to_vec(&state)?)?;
        self.unqueue(id)
    }

    pub fn unqueue(&self, id: &str) -> io::Result<()> {
        match fs::remove_file(self.outbox().join(id)) {
            Ok(()) => sync_folder(&self.outbox()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub fn delete(&self, id: &str) -> io::Result<()> {
        self.unqueue(id)?;
        let path = self.report_path(id)?;
        fs::remove_dir_all(&path)?;
        sync_folder(&self.root.join(REPORTS))
    }

    pub fn clear_outbox(&self) -> io::Result<()> {
        for id in self.waiting()? {
            self.unqueue(&id)?;
        }
        Ok(())
    }

    pub fn prune(&self) {
        self.remove_leftovers();
        let mut reports = self.all();
        let now = SystemTime::now();
        reports.retain(|report| {
            let expired = now
                .duration_since(report.stored_at)
                .is_ok_and(|age| age > KEEP_AGE);
            if expired {
                self.remove(&report.id);
            }
            !expired
        });
        let mut total: u64 = reports.iter().map(|report| report.bytes).sum();
        let mut count = reports.len();
        for settled_only in [true, false] {
            for report in &reports {
                if count <= KEEP_REPORTS && total <= KEEP_BYTES {
                    return;
                }
                if (report.status != Status::Local) == settled_only {
                    self.remove(&report.id);
                    count -= 1;
                    total = total.saturating_sub(report.bytes);
                }
            }
        }
    }

    fn remove(&self, id: &str) {
        if let Err(error) = self.delete(id) {
            tracing::warn!("cannot remove report {id}: {error}");
        }
    }

    fn remove_leftovers(&self) {
        let Ok(staged) = fs::read_dir(self.root.join(STAGING)) else {
            return;
        };
        for entry in staged.flatten() {
            if let Err(error) = fs::remove_dir_all(entry.path()) {
                tracing::warn!("cannot remove an unfinished report: {error}");
            }
        }
    }
}
