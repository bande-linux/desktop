use std::path::PathBuf;
use std::process::ExitCode;

use bande_automount::Automounter;
use bande_automount::space::{SpaceWatch, free_space, open_storage_settings, watch_space};
use zbus::blocking::Connection;

fn main() -> ExitCode {
    bande_log::init("bande-automount");
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
            bande_bus::serve_until_closed(&session);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
