use std::error::Error;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use unbound_power::battery::{self, BatteryNotifier};
use unbound_power::controller::{self, IdleController};
use unbound_power::logind::{self, ManagerProxyBlocking, SleepGuard};
use unbound_power::screensaver::InhibitionService;
use zbus::blocking::Connection;

const LOCK_WAIT: Duration = Duration::from_secs(3);

type Services = (SleepGuard, InhibitionService, Arc<IdleController>);

fn serve() -> Result<Services, Box<dyn Error>> {
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
    let idle = Arc::new(IdleController::new(
        controller::configured_policy(&session)?,
        controller::logind_executor(
            ManagerProxyBlocking::new(&system)?,
            logind::own_session(&system)?,
        ),
    ));
    controller::follow_config(&session, &idle)?;
    controller::follow_lock(&system, &idle)?;
    Ok((guard, inhibitions, idle))
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
