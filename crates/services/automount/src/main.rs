use std::process::ExitCode;

use unbound_automount::Automounter;
use zbus::blocking::Connection;

fn main() -> ExitCode {
    let started = Connection::system().and_then(|system| {
        let session = Connection::session()?;
        Automounter::start(&system, &session)
    });
    match started {
        Ok(_automounter) => loop {
            std::thread::park();
        },
        Err(error) => {
            eprintln!("unbound-automount: {error}");
            ExitCode::FAILURE
        }
    }
}
