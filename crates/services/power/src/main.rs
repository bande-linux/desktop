use std::error::Error;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use std::path::Path;

use unbound_power::backlight::{Backlight, SYSFS_BACKLIGHT};
use unbound_power::battery::{self, BatteryNotifier};
use unbound_power::controller::{self, IdleController};
use unbound_power::logind::{self, ManagerProxyBlocking, SleepGuard};
use unbound_power::screensaver::InhibitionService;
use zbus::blocking::Connection;

const LOCK_WAIT: Duration = Duration::from_secs(3);

type Services = (
    SleepGuard,
    InhibitionService,
    Arc<IdleController>,
    Connection,
);

fn serve() -> Result<Services, Box<dyn Error>> {
    let system = Connection::system()?;
    let session = Connection::session()?;
    let guard = SleepGuard::start(&system, LOCK_WAIT)?;
    battery::watch(&system, Arc::new(BatteryNotifier::new(&session)))?;
    let logind = system.clone();
    let inhibitions = InhibitionService::start(&session, move |application, reason| {
        logind::inhibit(&logind, "idle", application, reason, "block")
            .inspect_err(|error| tracing::warn!("cannot inhibit idle: {error}"))
            .ok()
    })?;
    let idle = Arc::new(IdleController::new(
        controller::configured_policy(&session)?,
        controller::logind_executor(
            ManagerProxyBlocking::new(&system)?,
            logind::own_session(&system)?,
            Backlight::find(Path::new(SYSFS_BACKLIGHT)),
        ),
    ));
    controller::follow_config(&session, &idle)?;
    controller::follow_lock(&system, &idle)?;
    match wayland_client::Connection::connect_to_env() {
        Ok(connection) => {
            if let Err(error) = unbound_power::wayland::watch_idle(connection, Arc::clone(&idle)) {
                tracing::warn!("cannot watch for idle: {error}");
            }
        }
        Err(error) => tracing::warn!("cannot watch for idle without Wayland: {error}"),
    }
    Ok((guard, inhibitions, idle, session))
}

fn main() -> ExitCode {
    unbound_log::init("unbound-power");
    match serve() {
        Ok((_guard, _inhibitions, _idle, session)) => {
            unbound_bus::serve_until_closed(&session);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
