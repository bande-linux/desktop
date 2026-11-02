use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::thread;

use zbus::blocking::Connection;
use zbus::blocking::fdo::ObjectManagerProxy;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const UDISKS: &str = "org.freedesktop.UDisks2";
const UDISKS_PATH: &str = "/org/freedesktop/UDisks2";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";
const BUSY: &str = "org.freedesktop.UDisks2.Error.DeviceBusy";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EjectFailure {
    pub mount_point: PathBuf,
    pub busy: bool,
    pub message: String,
}

fn mounted_at(points: &[Vec<u8>], mount_point: &Path) -> bool {
    let wanted = mount_point.as_os_str().as_bytes();
    points
        .iter()
        .any(|point| point.strip_suffix(&[0]).unwrap_or(point) == wanted)
}

fn eject_now(mount_point: &Path, force: bool) -> zbus::Result<()> {
    let system = Connection::system()?;
    let manager = ObjectManagerProxy::builder(&system)
        .destination(UDISKS)?
        .path(UDISKS_PATH)?
        .build()?;
    let options: HashMap<&str, Value<'_>> = match force {
        true => HashMap::from([("force", Value::from(true))]),
        false => HashMap::new(),
    };
    for (object, interfaces) in manager.get_managed_objects()? {
        let property = |interface: &str, name: &str| -> Option<OwnedValue> {
            interfaces
                .iter()
                .find(|(candidate, _)| candidate.as_str() == interface)
                .and_then(|(_, properties)| properties.get(name))
                .and_then(|value| value.try_clone().ok())
        };
        let points = property(FILESYSTEM, "MountPoints")
            .and_then(|value| Vec::<Vec<u8>>::try_from(value).ok())
            .unwrap_or_default();
        if !mounted_at(&points, mount_point) {
            continue;
        }
        system.call_method(
            Some(UDISKS),
            &object,
            Some(FILESYSTEM),
            "Unmount",
            &(&options,),
        )?;
        let drive =
            property(BLOCK, "Drive").and_then(|value| OwnedObjectPath::try_from(value).ok());
        if let Some(drive) = drive.filter(|drive| drive.as_str() != "/")
            && let Err(error) =
                system.call_method(Some(UDISKS), &drive, Some(DRIVE), "PowerOff", &(&options,))
        {
            tracing::info!(
                "unmounted {} but cannot power it off: {error}",
                mount_point.display()
            );
        }
        return Ok(());
    }
    Err(zbus::Error::Failure(format!(
        "{} is not a UDisks volume",
        mount_point.display()
    )))
}

pub fn eject(
    mount_points: Vec<PathBuf>,
    force: bool,
    failed: impl Fn(EjectFailure) + Send + 'static,
) {
    let work = move || {
        for mount_point in mount_points {
            if let Err(error) = eject_now(&mount_point, force) {
                tracing::warn!("cannot eject {}: {error}", mount_point.display());
                let (busy, message) = match &error {
                    zbus::Error::MethodError(name, message, _) => {
                        (name.as_str() == BUSY, message.clone().unwrap_or_default())
                    }
                    _ => (false, error.to_string()),
                };
                failed(EjectFailure {
                    mount_point,
                    busy,
                    message,
                });
            }
        }
    };
    if let Err(error) = thread::Builder::new().name("eject".to_owned()).spawn(work) {
        tracing::warn!("cannot start ejecting: {error}");
    }
}
