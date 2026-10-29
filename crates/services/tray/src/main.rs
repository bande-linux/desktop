use std::process::ExitCode;

use unbound_tray::Watcher;
use zbus::blocking::Connection;

fn main() -> ExitCode {
    unbound_log::init("unbound-tray");
    match Connection::session().and_then(|connection| Watcher::start(&connection)) {
        Ok(_watcher) => loop {
            std::thread::park();
        },
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
