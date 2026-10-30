use std::process::ExitCode;

use unbound_automount::Automounter;
use zbus::blocking::Connection;

fn main() -> ExitCode {
    unbound_log::init("unbound-automount");
    let started = Connection::system().and_then(|system| {
        let session = Connection::session()?;
        Ok((Automounter::start(&system, &session)?, session))
    });
    match started {
        Ok((_automounter, session)) => {
            unbound_bus::serve_until_closed(&session);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
