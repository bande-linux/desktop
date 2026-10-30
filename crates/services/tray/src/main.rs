use std::process::ExitCode;

use unbound_tray::Watcher;
use zbus::blocking::Connection;

fn main() -> ExitCode {
    unbound_log::init("unbound-tray");
    let started =
        Connection::session().and_then(|connection| Ok((Watcher::start(&connection)?, connection)));
    match started {
        Ok((_watcher, connection)) => {
            unbound_bus::serve_until_closed(&connection);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
