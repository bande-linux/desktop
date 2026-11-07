use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

pub const STATE_DIR: &str = "/var/lib/bande-backup";
const STATE_FILE: &str = "state.json";
const KEY_FILE: &str = "key";
const SHARE_PASSWORD_FILE: &str = "share-password";
const KEY_BYTES: usize = 32;
const PRIVATE: u32 = 0o700;
const SECRET: u32 = 0o400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Destination {
    Disk {
        uuid: String,
        name: String,
        filesystem: String,
    },
    Network {
        url: String,
        user: String,
        name: String,
    },
}

impl Destination {
    pub fn name(&self) -> &str {
        match self {
            Self::Disk { name, .. } | Self::Network { name, .. } => name,
        }
    }

    pub fn is_network(&self) -> bool {
        matches!(self, Self::Network { .. })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub destination: Option<Destination>,
    pub excluded: Vec<String>,
    pub on_battery: bool,
    pub chosen_at: u64,
    pub last_success: u64,
    pub last_prune: u64,
    pub last_check: u64,
    pub check_slice: u32,
    pub paused_until: u64,
    pub failures_in_a_row: u32,
    pub interrupted: bool,
    pub problem: String,
    pub damaged_at: u64,
    pub full_at: u64,
    pub oldest: u64,
    pub versions: u32,
}

pub fn load() -> State {
    let path = Path::new(STATE_DIR).join(STATE_FILE);
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            tracing::warn!("cannot read {}: {error}", path.display());
            State::default()
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => State::default(),
        Err(error) => {
            tracing::warn!("cannot read {}: {error}", path.display());
            State::default()
        }
    }
}

pub fn save(state: &State) -> io::Result<()> {
    fs::create_dir_all(STATE_DIR)?;
    fs::set_permissions(STATE_DIR, fs::Permissions::from_mode(PRIVATE))?;
    let path = Path::new(STATE_DIR).join(STATE_FILE);
    let temporary = path.with_extension("new");
    let mut file = File::create(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(state).map_err(io::Error::other)?)?;
    file.sync_all()?;
    fs::rename(&temporary, &path)?;
    File::open(STATE_DIR)?.sync_all()
}

pub fn key_path() -> PathBuf {
    Path::new(STATE_DIR).join(KEY_FILE)
}

pub fn ensure_key() -> io::Result<PathBuf> {
    let path = key_path();
    if path.exists() {
        return Ok(path);
    }
    fs::create_dir_all(STATE_DIR)?;
    fs::set_permissions(STATE_DIR, fs::Permissions::from_mode(PRIVATE))?;
    let mut random = [0_u8; KEY_BYTES];
    getrandom_fill(&mut random)?;
    let text: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(SECRET)
        .open(&path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    File::open(STATE_DIR)?.sync_all()?;
    Ok(path)
}

fn getrandom_fill(buffer: &mut [u8]) -> io::Result<()> {
    let mut file = File::open("/dev/urandom")?;
    io::Read::read_exact(&mut file, buffer)
}

pub fn share_password() -> String {
    fs::read_to_string(Path::new(STATE_DIR).join(SHARE_PASSWORD_FILE)).unwrap_or_default()
}

pub fn save_share_password(password: &str) -> io::Result<()> {
    fs::create_dir_all(STATE_DIR)?;
    fs::set_permissions(STATE_DIR, fs::Permissions::from_mode(PRIVATE))?;
    let path = Path::new(STATE_DIR).join(SHARE_PASSWORD_FILE);
    if let Err(error) = fs::remove_file(&path)
        && error.kind() != io::ErrorKind::NotFound
    {
        return Err(error);
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(SECRET)
        .open(&path)?;
    file.write_all(password.as_bytes())?;
    file.sync_all()
}

pub fn machine_id() -> io::Result<String> {
    Ok(fs::read_to_string("/etc/machine-id")?.trim().to_owned())
}

pub fn set_timer(enabled: bool) {
    let verb = if enabled { "enable" } else { "disable" };
    match Command::new("systemctl")
        .args([verb, "--now", "bande-backup.timer"])
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::warn!("systemctl {verb} bande-backup.timer: {status}"),
        Err(error) => tracing::warn!("cannot run systemctl: {error}"),
    }
}

pub struct SecretFile(PathBuf);

impl SecretFile {
    pub fn new(contents: &str) -> io::Result<Self> {
        let directory = Path::new(crate::disk::RUN_DIR);
        crate::disk::prepare_run_dir()?;
        let mut random = [0_u8; 8];
        getrandom_fill(&mut random)?;
        let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = directory.join(format!(".secret-{name}"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(SECRET)
            .open(&path)?;
        file.write_all(contents.as_bytes())?;
        Ok(Self(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for SecretFile {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.0) {
            tracing::warn!("cannot remove {}: {error}", self.0.display());
        }
    }
}
