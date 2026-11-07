use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::store::ChosenDisk;

pub const RUN_DIR: &str = "/run/bande-backup";
pub const BROWSE_DIR: &str = "/run/bande-backup-browse";
const MOUNT_DIR: &str = "/run/bande-backup/disk";
pub const REPOSITORIES: &str = "BandeBackups";
const PRIVATE: u32 = 0o700;
const MOUNT_OPTIONS: &str = "nosuid,nodev,noexec,noatime";
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

pub fn is_present(uuid: &str) -> bool {
    device(uuid).exists()
}

pub fn mount_type(filesystem: &str) -> &str {
    match filesystem {
        "ntfs" => "ntfs3",
        other => other,
    }
}

fn is_mounted(path: &Path) -> bool {
    let Ok(mounts) = fs::read_to_string("/proc/self/mounts") else {
        return false;
    };
    let wanted = path.to_string_lossy();
    mounts
        .lines()
        .filter_map(|line| line.split(' ').nth(1))
        .any(|mount| mount == wanted)
}

pub fn mount(disk: &ChosenDisk) -> io::Result<PathBuf> {
    prepare_run_dir()?;
    let mount_point = PathBuf::from(MOUNT_DIR);
    fs::create_dir_all(&mount_point)?;
    if is_mounted(&mount_point) {
        return Ok(mount_point);
    }
    let status = Command::new("mount")
        .args(["-t", mount_type(&disk.filesystem), "-o", MOUNT_OPTIONS])
        .arg(device(&disk.uuid))
        .arg(&mount_point)
        .status()?;
    if status.success() {
        Ok(mount_point)
    } else {
        Err(io::Error::other(format!("mount: {status}")))
    }
}

pub fn unmount() {
    let mount_point = Path::new(MOUNT_DIR);
    if !is_mounted(mount_point) {
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

pub fn peek(uuid: &str, filesystem: &str) -> io::Result<bool> {
    prepare_run_dir()?;
    let mount_point = Path::new(RUN_DIR).join("peek");
    fs::create_dir_all(&mount_point)?;
    let status = Command::new("mount")
        .args([
            "-t",
            mount_type(filesystem),
            "-o",
            "ro,nosuid,nodev,noexec,noatime",
        ])
        .arg(device(uuid))
        .arg(&mount_point)
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!("mount: {status}")));
    }
    let found =
        crate::store::machine_id().map(|id| has_repository(&repository_path(&mount_point, &id)));
    let unmounted = Command::new("umount").arg(&mount_point).status();
    if let Err(error) = unmounted {
        tracing::warn!("cannot unmount {}: {error}", mount_point.display());
    }
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
