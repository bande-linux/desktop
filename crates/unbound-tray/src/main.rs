use std::process::ExitCode;

use unbound_tray::Watcher;
use zbus::blocking::Connection;

fn main() -> ExitCode {
    match Connection::session().and_then(|connection| Watcher::start(&connection)) {
        Ok(_watcher) => loop {
            std::thread::park();
        },
        Err(error) => {
            eprintln!("unbound-tray: {error}");
            ExitCode::FAILURE
        }
    }
}
