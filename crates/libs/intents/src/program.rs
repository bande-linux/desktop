use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use signal_hook::consts::SIGTERM;

use crate::{Owner, Serving};

/// The argument that makes a program write its manifest into the folder that follows, and exit.
pub const WRITE_INTENTS: &str = "--write-intents";
const IDLE: Duration = Duration::from_secs(60);
const LATE_CALLS: Duration = Duration::from_millis(200);

/// The `main` of a service that only serves intents: writes its manifest when asked to, else
/// takes its bus name and serves until it has been idle for a minute.
pub fn run(owner: Owner) -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let ended = match (arguments.next(), arguments.next()) {
        (Some(flag), Some(folder)) if flag == WRITE_INTENTS => owner
            .write_manifest(Path::new(&folder))
            .map_err(zbus::Error::from),
        (None, _) => serve(owner),
        _ => Err(zbus::Error::Failure(format!(
            "the only argument is {WRITE_INTENTS} <folder>"
        ))),
    };
    match ended {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn serve(owner: Owner) -> zbus::Result<()> {
    let connection = bande_bus::session()?;
    let name = owner.bus();
    let serving = owner.serve(&connection)?;
    connection.request_name(name)?;
    let bus = connection.clone();
    let watched = serving.watch();
    std::thread::Builder::new()
        .name("bus".to_owned())
        .spawn(move || {
            bande_bus::serve_until_closed(&bus);
            watched.lost_the_bus();
        })?;
    stop_on_signal(serving.watch())?;
    while serving.wait_until_idle(IDLE) {
        connection.release_name(name)?;
        std::thread::sleep(LATE_CALLS);
        if !serving.is_busy() {
            return Ok(());
        }
        connection.request_name(name)?;
    }
    Ok(())
}

fn stop_on_signal(serving: Serving) -> zbus::Result<()> {
    let (mut asked, wake) = UnixStream::pair()?;
    signal_hook::low_level::pipe::register(SIGTERM, wake)?;
    std::thread::Builder::new()
        .name("signals".to_owned())
        .spawn(move || {
            if asked.read_exact(&mut [0]).is_ok() {
                serving.stop();
                std::process::exit(0);
            }
        })?;
    Ok(())
}
