use std::path::PathBuf;
use std::process::ExitCode;

use unbound_automount::Automounter;
use unbound_automount::space::{SpaceWatch, free_space, open_storage_settings, watch_space};
use zbus::blocking::Connection;

fn main() -> ExitCode {
    unbound_log::init("unbound-automount");
    let started = Connection::system().and_then(|system| {
        let session = Connection::session()?;
        Ok((Automounter::start(&system, &session)?, session))
    });
    match started {
        Ok((_automounter, session)) => {
            let mut volumes = vec![PathBuf::from("/")];
            volumes.extend(std::env::var_os("HOME").map(PathBuf::from));
            let _space = watch_space(
                SpaceWatch::new(&session, &volumes, free_space),
                open_storage_settings,
            );
            unbound_bus::serve_until_closed(&session);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
