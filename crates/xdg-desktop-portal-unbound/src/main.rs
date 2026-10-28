use std::process::ExitCode;

use xdg_desktop_portal_unbound::BUS_NAME;
use xdg_desktop_portal_unbound::settings::Settings;
use zbus::blocking::Connection;

fn serve() -> zbus::Result<Connection> {
    let connection = Connection::session()?;
    Settings::serve(&connection)?;
    connection.request_name(BUS_NAME)?;
    Ok(connection)
}

fn main() -> ExitCode {
    match serve() {
        Ok(_connection) => loop {
            std::thread::park();
        },
        Err(error) => {
            eprintln!("xdg-desktop-portal-unbound: {error}");
            ExitCode::FAILURE
        }
    }
}
