use std::error::Error;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use unbound_power::battery::{self, BatteryNotifier};
use unbound_power::logind::{self, SleepGuard};
use unbound_power::screensaver::InhibitionService;
use zbus::blocking::Connection;

const LOCK_WAIT: Duration = Duration::from_secs(3);

fn serve() -> Result<(SleepGuard, InhibitionService), Box<dyn Error>> {
    let system = Connection::system()?;
    let session = Connection::session()?;
    let guard = SleepGuard::start(&system, LOCK_WAIT)?;
    battery::watch(&system, Arc::new(BatteryNotifier::new(&session)))?;
    let logind = system.clone();
    let inhibitions = InhibitionService::start(&session, move |application, reason| {
        logind::inhibit(&logind, "idle", application, reason, "block")
            .inspect_err(|error| eprintln!("unbound-power: cannot inhibit idle: {error}"))
            .ok()
    })?;
    Ok((guard, inhibitions))
}

fn main() -> ExitCode {
    match serve() {
        Ok(_services) => loop {
            std::thread::park();
        },
        Err(error) => {
            eprintln!("unbound-power: {error}");
            ExitCode::FAILURE
        }
    }
}
