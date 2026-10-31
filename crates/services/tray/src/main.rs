use std::process::ExitCode;

use bande_tray::Watcher;
use zbus::blocking::Connection;

fn main() -> ExitCode {
    bande_log::init("bande-tray");
    let started =
        Connection::session().and_then(|connection| Ok((Watcher::start(&connection)?, connection)));
    match started {
        Ok((_watcher, connection)) => {
            bande_bus::serve_until_closed(&connection);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
