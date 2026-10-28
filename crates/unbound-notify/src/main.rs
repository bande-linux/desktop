use std::process::ExitCode;

use unbound_notify::NotificationServer;
use zbus::blocking::Connection;

fn main() -> ExitCode {
    let server =
        Connection::session().and_then(|connection| NotificationServer::start(&connection));
    match server {
        Ok(_server) => loop {
            std::thread::park();
        },
        Err(error) => {
            eprintln!("unbound-notify: {error}");
            ExitCode::FAILURE
        }
    }
}
