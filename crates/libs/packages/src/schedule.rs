use std::time::Duration;

use zbus::blocking::Connection;

use crate::Client;

pub const CHECK_SLACK: Duration = Duration::from_secs(6 * 60 * 60);

#[zbus::proxy(
    interface = "org.bande.Config",
    default_service = "org.bande.Config",
    default_path = "/org/bande/Config"
)]
trait Config {
    #[zbus(property)]
    fn check_updates_automatically(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn automatic_updates(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn update_check_every(&self) -> zbus::Result<u64>;
}

pub fn check_interval(session: &Connection) -> Option<Duration> {
    let seconds = ConfigProxyBlocking::new(session)
        .and_then(|config| config.update_check_every())
        .unwrap_or_else(|error| {
            tracing::warn!("cannot read how often to check for updates: {error}");
            0
        });
    (seconds > 0).then(|| Duration::from_secs(seconds))
}

fn checks_automatically(session: &Connection) -> bool {
    ConfigProxyBlocking::new(session)
        .and_then(|config| Ok(config.check_updates_automatically()? || config.automatic_updates()?))
        .unwrap_or_else(|error| {
            tracing::warn!("cannot read whether to check for updates automatically: {error}");
            false
        })
}

pub fn check_due(session: &Connection, client: &Client) -> bool {
    if !checks_automatically(session) {
        return false;
    }
    let Some(every) = check_interval(session) else {
        return true;
    };
    client
        .seconds_since_refresh()
        .map(|since| u64::from(since) + CHECK_SLACK.as_secs() >= every.as_secs())
        .unwrap_or(true)
}
