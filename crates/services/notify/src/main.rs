use std::path::PathBuf;
use std::process::ExitCode;

use bande_notify::{NotificationServer, do_not_disturb_path};
use zbus::blocking::Connection;

fn main() -> ExitCode {
    bande_log::init("bande-notify");
    let state_home = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let remembered = do_not_disturb_path(state_home.as_deref(), home.as_deref());
    let server = Connection::session().and_then(|connection| {
        let server = match remembered {
            Some(file) => NotificationServer::start_remembering(&connection, file),
            None => NotificationServer::start(&connection),
        }?;
        Ok((connection, server))
    });
    match server {
        Ok((connection, _server)) => {
            bande_bus::serve_until_closed(&connection);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
