use std::path::PathBuf;
use std::process::ExitCode;

use bande_automount::Automounter;
use bande_automount::askpass::{self, answer_prompt};
use bande_automount::devices::Devices;
use bande_automount::mounts::Mounts;
use bande_automount::space::{SpaceWatch, free_space, open_storage_settings, watch_space};
use zbus::blocking::Connection;

fn askpass_requested() -> Option<ExitCode> {
    std::env::var_os(askpass::SOCKET_VARIABLE)?;
    let prompt = std::env::args().nth(1).unwrap_or_default();
    Some(match answer_prompt(&prompt) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) | Err(_) => ExitCode::FAILURE,
    })
}

fn main() -> ExitCode {
    if let Some(code) = askpass_requested() {
        return code;
    }
    bande_log::init("bande-automount");
    let started = Connection::system().and_then(|system| {
        let session = Connection::session()?;
        let automounter = Automounter::start(&system, &session)?;
        let mounts = Mounts::start(&session)?;
        automounter.follow_devices(Devices::start(mounts.clone()));
        mounts.follow_alerts();
        mounts.reconnect_saved();
        Ok((automounter, session))
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
