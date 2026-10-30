use std::process::ExitCode;

use xdg_desktop_portal_unbound::BUS_NAME;
use xdg_desktop_portal_unbound::settings::{CursorSettings, Settings};
use zbus::blocking::Connection;

fn serve() -> zbus::Result<Connection> {
    let connection = Connection::session()?;
    Settings::serve(&connection, CursorSettings::from_environment())?;
    connection.request_name(BUS_NAME)?;
    Ok(connection)
}

fn main() -> ExitCode {
    unbound_log::init("xdg-desktop-portal-unbound");
    match serve() {
        Ok(connection) => {
            unbound_bus::serve_until_closed(&connection);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
