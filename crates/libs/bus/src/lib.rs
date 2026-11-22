pub mod accounts;
pub mod logind;
pub mod portal_settings;
pub mod power_profiles;
mod reading;
pub mod systemd;

pub use reading::Reading;

use std::time::Duration;

use zbus::blocking::{Connection, MessageIterator, connection};

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
