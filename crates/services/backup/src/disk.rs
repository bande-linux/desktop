use std::fs;
use std::io;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use bande_volumes::server::{Protocol, Server};
use bande_volumes::{NFS_SOFT_OPTIONS, SMB_SOFT_OPTIONS, force_unmount};

use crate::store::{self, Destination, SecretFile};

pub const RUN_DIR: &str = "/run/bande-backup";
pub const BROWSE_DIR: &str = "/run/bande-backup-browse";
pub const LOCAL_BROWSE_DIR: &str = "/run/bande-backup-local";
const MOUNT_DIR: &str = "/run/bande-backup/disk";
pub const REPOSITORIES: &str = "BandeBackups";
const PRIVATE: u32 = 0o700;
pub const MOUNT_OPTIONS: &str = "nosuid,nodev,noexec,noatime";
const NETWORK_TYPES: [&str; 4] = ["cifs", "smb3", "nfs", "nfs4"];
const MOUNT_LIMIT: &str = "30";
const REACH_LIMIT: Duration = Duration::from_secs(4);
const REST_SCHEME: &str = "rest:";
const REST_PORT: u16 = 8000;
const SMB_PORT: u16 = 445;
const NFS_PORT: u16 = 2049;
const MOUNT_CIFS: &str = "/usr/sbin/mount.cifs";
const MOUNT_NFS: &str = "/usr/sbin/mount.nfs";
const KIB: u64 = 1024;
const MIB: u64 = KIB * KIB;
const MOST_RESERVED: u64 = 512 * MIB;
const SHARE_RESERVED: u64 = 10;

pub fn prepare_run_dir() -> io::Result<()> {
    fs::create_dir_all(RUN_DIR)?;
    fs::set_permissions(RUN_DIR, fs::Permissions::from_mode(PRIVATE))
}

pub fn device(uuid: &str) -> PathBuf {
    Path::new("/dev/disk/by-uuid").join(uuid)
}

fn mount_type_at(path: &Path) -> Option<String> {
    let mounts = fs::read_to_string("/proc/self/mounts").ok()?;
    let wanted = path.to_string_lossy();
    mounts.lines().rev().find_map(|line| {
        let mut fields = line.split(' ').skip(1);
        (fields.next() == Some(&*wanted))
            .then(|| fields.next().map(str::to_owned))
            .flatten()
    })
}

pub fn unmount() {
    let mount_point = Path::new(MOUNT_DIR);
    let Some(kind) = mount_type_at(mount_point) else {
        return;
    };
    if NETWORK_TYPES.contains(&kind.as_str()) {
        force_unmount(mount_point);
        return;
    }
    let clean = Command::new("umount")
        .arg(mount_point)
        .status()
        .is_ok_and(|status| status.success());
    if !clean && let Err(error) = Command::new("umount").arg("-l").arg(mount_point).status() {
        tracing::warn!("cannot unmount the backup disk: {error}");
    }
}

fn run_mount(mut command: Command) -> io::Result<()> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("mount: {status}")))
    }
}

pub fn parse_network(url: &str) -> Option<NetworkPlace> {
    if let Some(rest) = rest_url(url) {
        return Some(NetworkPlace::Rest(rest));
    }
    let server = Server::parse(url)?;
    match server.protocol {
        Protocol::Smb if server.unc().is_some() => Some(NetworkPlace::Smb(server)),
        Protocol::Nfs if !server.path.is_empty() => Some(NetworkPlace::Nfs(server)),
        _ => None,
    }
}

fn rest_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix(REST_SCHEME)?;
    (rest.starts_with("http://") || rest.starts_with("https://"))
        .then(|| rest.trim_end_matches('/').to_owned())
}

pub enum NetworkPlace {
    Smb(Server),
    Nfs(Server),
    Rest(String),
}

impl NetworkPlace {
    fn address(&self) -> Option<(String, u16)> {
        match self {
            Self::Smb(server) => Some((server.host.clone(), server.port.unwrap_or(SMB_PORT))),
            Self::Nfs(server) => Some((server.host.clone(), server.port.unwrap_or(NFS_PORT))),
            Self::Rest(url) => {
                let (_, after) = url.split_once("://")?;
                let authority = after.split('/').next()?;
                match authority.rsplit_once(':') {
                    Some((host, port)) => Some((host.to_owned(), port.parse().ok()?)),
                    None => Some((authority.to_owned(), REST_PORT)),
                }
            }
        }
    }

    pub fn is_reachable(&self) -> bool {
        let Some(address) = self.address() else {
            return false;
        };
        let (sender, answer) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let addresses: Vec<SocketAddr> = address
                .to_socket_addrs()
                .map(Iterator::collect)
                .unwrap_or_default();
            let reached = addresses
                .iter()
                .any(|address| TcpStream::connect_timeout(address, REACH_LIMIT).is_ok());
            let _ = sender.send(reached);
        });
        answer.recv_timeout(REACH_LIMIT * 2).unwrap_or(false)
    }

    pub fn missing_package(&self) -> Option<&'static str> {
        match self {
            Self::Smb(_) if !Path::new(MOUNT_CIFS).exists() => Some("cifs-utils"),
            Self::Nfs(_) if !Path::new(MOUNT_NFS).exists() => Some("nfs-common"),
            _ => None,
        }
    }

    pub fn mount_at(&self, point: &Path, user: &str, password: &str) -> io::Result<()> {
        match self {
            Self::Smb(server) => {
                let unc = server
                    .unc()
                    .ok_or_else(|| io::Error::other("no share in the address"))?;
                let name = user.rsplit('\\').next().unwrap_or(user);
                let credentials =
                    SecretFile::new(&format!("username={name}\npassword={password}\n"))?;
                let mut options = format!(
                    "{SMB_SOFT_OPTIONS},noexec,noperm,credentials={}",
                    credentials.path().display()
                );
                if user.is_empty() {
                    options.push_str(",guest");
                }
                if let Some((domain, _)) = user.split_once('\\') {
                    options.push_str(&format!(",domain={domain}"));
                }
                if let Some(port) = server.port {
                    options.push_str(&format!(",port={port}"));
                }
                let mut command = Command::new("timeout");
                command
                    .args(["-k", "2", MOUNT_LIMIT, MOUNT_CIFS])
                    .arg(unc)
                    .arg(point)
                    .args(["-o", &options]);
                run_mount(command)
            }
            Self::Nfs(server) => {
                let port = server
                    .port
                    .map(|port| format!(",port={port}"))
                    .unwrap_or_default();
                let attempt = |extra: &str| {
                    let mut command = Command::new("timeout");
                    command
                        .args(["-k", "2", MOUNT_LIMIT, MOUNT_NFS])
                        .arg(server.nfs_source())
                        .arg(point)
                        .arg("-o")
                        .arg(format!("{NFS_SOFT_OPTIONS},noexec{port},{extra}"));
                    run_mount(command)
                };
                attempt("vers=4").or_else(|_| attempt("vers=3,nolock"))
            }
            Self::Rest(_) => Ok(()),
        }
    }
}

impl Destination {
    pub fn network_place(&self) -> Option<NetworkPlace> {
        match self {
            Self::Disk { .. } => None,
            Self::Network { url, .. } => parse_network(url),
        }
    }

    pub fn is_present(&self) -> bool {
        match self {
            Self::Disk { uuid, .. } => device(uuid).exists(),
            Self::Network { .. } => self
                .network_place()
                .is_some_and(|place| place.is_reachable()),
        }
    }

    pub fn is_rest(&self) -> bool {
        matches!(self.network_place(), Some(NetworkPlace::Rest(_)))
    }

    pub fn mount(&self) -> io::Result<PathBuf> {
        prepare_run_dir()?;
        let point = PathBuf::from(MOUNT_DIR);
        fs::create_dir_all(&point)?;
        if self.is_rest() || mount_type_at(&point).is_some() {
            return Ok(point);
        }
        match self {
            Self::Disk {
                uuid, filesystem, ..
            } => {
                let mut command = Command::new("mount");
                command
                    .args(["-t", filesystem.as_str(), "-o", MOUNT_OPTIONS])
                    .arg(device(uuid))
                    .arg(&point);
                run_mount(command)?;
            }
            Self::Network { user, .. } => {
                let place = self
                    .network_place()
                    .ok_or_else(|| io::Error::other("not a network address"))?;
                place.mount_at(&point, user, &store::share_password())?;
            }
        }
        Ok(point)
    }

    pub fn repository_location(&self, mount_point: &Path, id: &str) -> PathBuf {
        match self.network_place() {
            Some(NetworkPlace::Rest(url)) => PathBuf::from(format!("{REST_SCHEME}{url}/{id}")),
            _ => repository_path(mount_point, id),
        }
    }
}

pub fn peek(uuid: &str, filesystem: &str) -> io::Result<bool> {
    prepare_run_dir()?;
    let mount_point = Path::new(RUN_DIR).join("peek");
    fs::create_dir_all(&mount_point)?;
    let status = Command::new("mount")
        .args(["-t", filesystem, "-o", MOUNT_OPTIONS])
        .arg(device(uuid))
        .arg(&mount_point)
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!("mount: {status}")));
    }
    let found = store::machine_id().map(|id| has_repository(&repository_path(&mount_point, &id)));
    let unmounted = Command::new("umount").arg(&mount_point).status();
    if let Err(error) = unmounted {
        tracing::warn!("cannot unmount {}: {error}", mount_point.display());
    }
    found
}

pub fn peek_share(place: &NetworkPlace, user: &str, password: &str) -> io::Result<bool> {
    prepare_run_dir()?;
    let mount_point = Path::new(RUN_DIR).join("peek");
    fs::create_dir_all(&mount_point)?;
    place.mount_at(&mount_point, user, password)?;
    let found = store::machine_id().map(|id| has_repository(&repository_path(&mount_point, &id)));
    force_unmount(&mount_point);
    found
}

pub fn repository_path(mount_point: &Path, machine_id: &str) -> PathBuf {
    mount_point.join(REPOSITORIES).join(machine_id)
}

pub fn has_repository(path: &Path) -> bool {
    path.join("config").is_file()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Space {
    pub total: u64,
    pub available: u64,
}

pub fn space(path: &Path) -> io::Result<Space> {
    let stats = rustix::fs::statvfs(path)?;
    Ok(Space {
        total: stats.f_blocks * stats.f_frsize,
        available: stats.f_bavail * stats.f_frsize,
    })
}

pub fn reserved_for(total: u64) -> u64 {
    MOST_RESERVED.min(total / SHARE_RESERVED)
}
