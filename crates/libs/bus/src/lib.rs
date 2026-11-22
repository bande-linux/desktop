pub mod accounts;
pub mod logind;
pub mod portal_settings;
pub mod power_profiles;
mod reading;
pub mod systemd;
pub mod systemd_config;

pub use reading::Reading;

use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::time::Duration;

use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator, connection};
use zbus::message::Message;
use zbus::names::BusName;

pub const CALL_TIMEOUT: Duration = Duration::from_secs(25);

pub fn session() -> zbus::Result<Connection> {
    connection::Builder::session()?
        .method_timeout(CALL_TIMEOUT)
        .build()
}

pub fn system() -> zbus::Result<Connection> {
    connection::Builder::system()?
        .method_timeout(CALL_TIMEOUT)
        .build()
}

/// Logs a signal that could not be sent; listeners miss that one change.
pub fn warn_unsent(signal: &str, sent: zbus::Result<()>) {
    if let Err(error) = sent {
        tracing::warn!("cannot send {signal}: {error}");
    }
}

pub fn serve_until_closed(connection: &Connection) {
    for _ in MessageIterator::from(connection) {}
}

/// Whether `error` says that no program owns the called bus name.
pub fn is_unowned(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::FDO(error) => matches!(
            **error,
            zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_)
        ),
        zbus::Error::MethodError(name, _, _) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
        ),
        _ => false,
    }
}

fn pid_of_handle(handle: BorrowedFd<'_>) -> Option<u32> {
    std::fs::read_to_string(format!("/proc/self/fdinfo/{}", handle.as_raw_fd()))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("Pid:"))?
        .trim()
        .parse()
        .ok()
}

/// Whether the connection `caller` on `bus` belongs to the main process of the service manager's
/// `unit` (on the same bus), checked through the process handle the bus took when it connected,
/// so a process that only shares that program, or that later took the number, does not pass.
pub fn is_main_process_of(bus: &Connection, caller: BusName<'_>, unit: &str) -> zbus::Result<bool> {
    let credentials = DBusProxy::new(bus)?.get_connection_credentials(caller)?;
    let (Some(process), Some(handle)) = (credentials.process_id(), credentials.process_fd()) else {
        return Ok(false);
    };
    Ok(pid_of_handle(handle.as_fd()) == Some(process)
        && systemd::main_pid(bus, unit)? == Some(process))
}

/// Whether `message` came from the program that owns `name` now.
pub fn sent_by(bus: &Connection, message: &Message, name: &str) -> bool {
    let header = message.header();
    let Some(sender) = header.sender() else {
        return false;
    };
    let owner = DBusProxy::new(bus).and_then(|proxy| {
        proxy
            .get_name_owner(BusName::try_from(name.to_owned())?)
            .map_err(zbus::Error::from)
    });
    owner.is_ok_and(|owner| owner.as_str() == sender.as_str())
}
