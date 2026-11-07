use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::thread;

use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, Value};

use crate::mounts::is_connected;
use crate::udisks::{BLOCK, DRIVE, ENCRYPTED, FILESYSTEM, LOOP, Objects, PARTITION, UDISKS};

const BACKUP_MOUNT: &str = "/run/bande-backup/disk";
const BACKUP_SERVICE: &str = "org.bande.Backup";
const BACKUP_PATH: &str = "/org/bande/Backup";

const BUSY: &str = "org.freedesktop.UDisks2.Error.DeviceBusy";
pub const MOUNTS_SERVICE: &str = "org.bande.Mounts";
pub const MOUNTS_PATH: &str = "/org/bande/Mounts";
pub const MOUNTS_INTERFACE: &str = "org.bande.Mounts";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EjectFailure {
    pub mount_point: PathBuf,
    pub busy: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Unit {
    Drive(String),
    Loop(String),
    Block(String),
}

fn unit_of(objects: &Objects, block: &str) -> Unit {
    if let Some(backing) = objects.path(block, BLOCK, "CryptoBackingDevice") {
        return unit_of(objects, &backing);
    }
    if let Some(drive) = objects.path(block, BLOCK, "Drive") {
        return Unit::Drive(drive);
    }
    let whole = objects
        .path(block, PARTITION, "Table")
        .unwrap_or_else(|| block.to_owned());
    match objects.has(&whole, LOOP) {
        true => Unit::Loop(whole),
        false => Unit::Block(block.to_owned()),
    }
}

pub fn members(objects: &Objects, block: &str) -> Vec<String> {
    let unit = unit_of(objects, block);
    let mut members: Vec<String> = objects
        .paths()
        .map(|path| path.to_string())
        .filter(|path| objects.has(path, BLOCK) && unit_of(objects, path) == unit)
        .collect();
    members.sort();
    members
}

fn call(
    system: &Connection,
    path: &str,
    interface: &str,
    method: &str,
    force: bool,
) -> zbus::Result<()> {
    let options: HashMap<&str, Value<'_>> = match force {
        true => HashMap::from([("force", Value::from(true))]),
        false => HashMap::new(),
    };
    let path = ObjectPath::try_from(path)?;
    system.call_method(Some(UDISKS), &path, Some(interface), method, &(options,))?;
    Ok(())
}

fn uid() -> Option<u32> {
    std::fs::metadata("/proc/self")
        .ok()
        .map(|metadata| metadata.uid())
}

fn release_from_backups(
    system: &Connection,
    objects: &Objects,
    members: &[String],
) -> zbus::Result<()> {
    let used = members.iter().any(|member| {
        objects
            .mount_points(member)
            .iter()
            .any(|point| point == BACKUP_MOUNT)
    });
    if used {
        system.call_method(
            Some(BACKUP_SERVICE),
            BACKUP_PATH,
            Some(BACKUP_SERVICE),
            "Release",
            &(),
        )?;
    }
    Ok(())
}

fn unmount_members(
    system: &Connection,
    objects: &Objects,
    members: &[String],
    force: bool,
) -> zbus::Result<()> {
    for member in members {
        if objects
            .mount_points(member)
            .iter()
            .any(|point| point != BACKUP_MOUNT)
        {
            call(system, member, FILESYSTEM, "Unmount", force)?;
        }
    }
    Ok(())
}

pub fn eject_block_unmount_only(
    system: &Connection,
    objects: &Objects,
    block: &str,
) -> zbus::Result<()> {
    let members = members(objects, block);
    unmount_members(system, objects, &members, true)?;
    for member in &members {
        if objects.path(member, ENCRYPTED, "CleartextDevice").is_some() {
            call(system, member, ENCRYPTED, "Lock", false)?;
        }
    }
    Ok(())
}

pub fn eject_block(
    system: &Connection,
    objects: &Objects,
    block: &str,
    force: bool,
) -> zbus::Result<()> {
    let members = members(objects, block);
    release_from_backups(system, objects, &members)?;
    unmount_members(system, objects, &members, force)?;
    for member in &members {
        if objects.path(member, ENCRYPTED, "CleartextDevice").is_some() {
            call(system, member, ENCRYPTED, "Lock", false)?;
        }
    }
    match unit_of(objects, block) {
        Unit::Drive(drive) if objects.flag(&drive, DRIVE, "Ejectable") => {
            call(system, &drive, DRIVE, "Eject", false)
        }
        Unit::Drive(drive) if objects.flag(&drive, DRIVE, "CanPowerOff") => {
            call(system, &drive, DRIVE, "PowerOff", false)
        }
        Unit::Loop(device) if objects.get::<u32>(&device, LOOP, "SetupByUID") == uid() => {
            call(system, &device, LOOP, "Delete", false)
        }
        _ => Ok(()),
    }
}

fn failure(mount_point: PathBuf, error: &zbus::Error) -> EjectFailure {
    let (busy, message) = match error {
        zbus::Error::MethodError(name, message, _) => {
            (name.as_str() == BUSY, message.clone().unwrap_or_default())
        }
        _ => (false, error.to_string()),
    };
    EjectFailure {
        mount_point,
        busy,
        message,
    }
}

fn eject_all(
    mount_points: Vec<PathBuf>,
    force: bool,
    failed: &impl Fn(EjectFailure),
) -> zbus::Result<()> {
    let system = Connection::system()?;
    let objects = Objects::load(&system)?;
    let mut units: Vec<Vec<String>> = Vec::new();
    for mount_point in mount_points {
        let wanted = mount_point.to_string_lossy().into_owned();
        let block = objects
            .paths()
            .find(|path| objects.mount_points(path).contains(&wanted))
            .map(|path| path.to_string());
        let Some(block) = block else {
            let error = zbus::Error::Failure(format!("{wanted} is not a UDisks volume"));
            failed(failure(mount_point, &error));
            continue;
        };
        let unit = members(&objects, &block);
        if units.contains(&unit) {
            continue;
        }
        units.push(unit);
        if let Err(error) = eject_block(&system, &objects, &block, force) {
            tracing::warn!("cannot eject {wanted}: {error}");
            failed(failure(mount_point, &error));
        }
    }
    Ok(())
}

fn disconnect(mount_point: PathBuf, failed: &impl Fn(EjectFailure)) {
    let answer = Connection::session().and_then(|session| {
        session.call_method(
            Some(MOUNTS_SERVICE),
            MOUNTS_PATH,
            Some(MOUNTS_INTERFACE),
            "Disconnect",
            &(mount_point.to_string_lossy().as_ref(),),
        )
    });
    if let Err(error) = answer {
        tracing::warn!("cannot disconnect {}: {error}", mount_point.display());
        failed(failure(mount_point, &error));
    }
}

pub fn eject(
    mount_points: Vec<PathBuf>,
    force: bool,
    failed: impl Fn(EjectFailure) + Send + 'static,
) {
    let work = move || {
        let (connected, disks): (Vec<_>, Vec<_>) = mount_points
            .into_iter()
            .partition(|point| is_connected(point));
        for point in connected {
            disconnect(point, &failed);
        }
        if disks.is_empty() {
            return;
        }
        let first = disks.first().cloned().unwrap_or_default();
        if let Err(error) = eject_all(disks, force, &failed) {
            tracing::warn!("cannot eject: {error}");
            failed(failure(first, &error));
        }
    };
    if let Err(error) = thread::Builder::new().name("eject".to_owned()).spawn(work) {
        tracing::warn!("cannot start ejecting: {error}");
    }
}
