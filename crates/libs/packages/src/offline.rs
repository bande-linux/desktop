use zbus::blocking::Connection;

use crate::{Error, ErrorCode, Failure};

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
    match OfflineProxyBlocking::new(system)?.get_results() {
        Ok((true, ids, ..)) => Ok(Some(Results::Installed(ids))),
        Ok((false, _, _, _, code, details)) => Ok(Some(Results::Failed(Failure {
            code: ErrorCode::from_raw(code),
            details,
        }))),
        Err(error) => match Error::from(error) {
            Error::Bus(zbus::Error::MethodError(..)) => Ok(None),
            other => Err(other),
        },
    }
}

pub fn clear_results(system: &Connection) -> Result<(), Error> {
    Ok(OfflineProxyBlocking::new(system)?.clear_results()?)
}
