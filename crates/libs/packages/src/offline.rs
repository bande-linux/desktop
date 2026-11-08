use std::path::Path;

use zbus::blocking::Connection;

use crate::{Error, ErrorCode, Failure};

const RESULTS: &str = "/var/lib/PackageKit/offline-update-competed";

#[zbus::proxy(
    interface = "org.freedesktop.PackageKit.Offline",
    default_service = "org.freedesktop.PackageKit",
    default_path = "/org/freedesktop/PackageKit"
)]
trait Offline {
    fn get_prepared(&self) -> zbus::Result<Vec<String>>;
    fn get_results(&self) -> zbus::Result<(bool, Vec<String>, u32, u64, u32, String)>;
    fn clear_results(&self) -> zbus::Result<()>;
    fn trigger(&self, action: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn update_triggered(&self) -> zbus::Result<bool>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Results {
    Installed(Vec<String>),
    Failed(Failure),
}

pub fn prepared(system: &Connection) -> Result<Vec<String>, Error> {
    Ok(OfflineProxyBlocking::new(system)?.get_prepared()?)
}

pub fn triggered(system: &Connection) -> Result<bool, Error> {
    Ok(OfflineProxyBlocking::new(system)?.update_triggered()?)
}

pub fn trigger(system: &Connection) -> Result<(), Error> {
    Ok(OfflineProxyBlocking::new(system)?.trigger("reboot")?)
}

pub fn results(system: &Connection) -> Result<Option<Results>, Error> {
    if !Path::new(RESULTS).exists() {
        return Ok(None);
    }
    Ok(Some(
        match OfflineProxyBlocking::new(system)?.get_results()? {
            (true, ids, ..) => Results::Installed(ids),
            (false, _, _, _, code, details) => Results::Failed(Failure {
                code: ErrorCode::from_raw(code),
                details,
            }),
        },
    ))
}

pub fn clear_results(system: &Connection) -> Result<(), Error> {
    Ok(OfflineProxyBlocking::new(system)?.clear_results()?)
}
