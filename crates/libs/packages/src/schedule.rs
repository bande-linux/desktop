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
    fn software_updates(&self) -> zbus::Result<String>;
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

pub fn software_updates(session: &Connection) -> String {
    ConfigProxyBlocking::new(session)
        .and_then(|config| config.software_updates())
        .unwrap_or_else(|error| {
            tracing::warn!("cannot read what to do with software updates: {error}");
            OFF.to_owned()
        })
}

pub const OFF: &str = "off";
pub const INSTALL: &str = "install";

pub fn check_due(session: &Connection, client: &Client) -> bool {
    if software_updates(session) == OFF {
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
