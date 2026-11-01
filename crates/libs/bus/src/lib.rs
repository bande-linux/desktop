pub mod portal_settings;

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

pub fn serve_until_closed(connection: &Connection) {
    for _ in MessageIterator::from(connection) {}
}
