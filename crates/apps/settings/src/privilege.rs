use std::collections::HashMap;

use bande_settings_panes::SettingId;
use zbus::blocking::Connection;
use zbus::zvariant::Value;

use crate::follow::{Update, Updates, spawn};

const NO_INTERACTION: u32 = 0;
const SYSTEM_UPDATE: &str = "org.freedesktop.packagekit.system-update";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Privilege {
    Time,
    Locale,
    Hostname,
    Keyboard,
}

impl Privilege {
    const ALL: [Self; 4] = [Self::Time, Self::Locale, Self::Hostname, Self::Keyboard];

    fn action(self) -> &'static str {
        match self {
            Self::Time => "org.freedesktop.timedate1.set-timezone",
            Self::Locale => "org.freedesktop.locale1.set-locale",
            Self::Hostname => "org.freedesktop.hostname1.set-static-hostname",
            Self::Keyboard => "org.freedesktop.locale1.set-keyboard",
        }
    }

    pub fn of(setting: SettingId) -> Option<Self> {
        match setting {
            SettingId::AutomaticTime | SettingId::DateAndTime | SettingId::TimeZone => {
                Some(Self::Time)
            }
            SettingId::Language | SettingId::Region => Some(Self::Locale),
            SettingId::ComputerName => Some(Self::Hostname),
            _ => None,
        }
    }
}

#[zbus::proxy(
    interface = "org.freedesktop.PolicyKit1.Authority",
    default_service = "org.freedesktop.PolicyKit1",
    default_path = "/org/freedesktop/PolicyKit1/Authority"
)]
trait Authority {
    fn check_authorization(
        &self,
        subject: &(&str, HashMap<&str, Value<'_>>),
        action_id: &str,
        details: &HashMap<&str, &str>,
        flags: u32,
        cancellation_id: &str,
    ) -> zbus::Result<(bool, bool, HashMap<String, String>)>;
}

fn ask(
    authority: &AuthorityProxyBlocking<'_>,
    name: &str,
    action: &str,
) -> zbus::Result<(bool, bool)> {
    let subject = (
        "system-bus-name",
        HashMap::from([("name", Value::from(name))]),
    );
    authority
        .check_authorization(&subject, action, &HashMap::new(), NO_INTERACTION, "")
        .map(|(authorized, challenge, _)| (authorized, challenge))
}

pub fn refused(system: &Connection) -> zbus::Result<Vec<Privilege>> {
    let authority = AuthorityProxyBlocking::new(system)?;
    let Some(name) = system.unique_name() else {
        return Ok(Vec::new());
    };
    let mut refused = Vec::new();
    for privilege in Privilege::ALL {
        match ask(&authority, name.as_str(), privilege.action()) {
            Ok((true, _) | (false, true)) => {}
            Ok(_) => refused.push(privilege),
            Err(error) => {
                tracing::warn!("cannot check {}: {error}", privilege.action());
                refused.push(privilege);
            }
        }
    }
    Ok(refused)
}

pub(crate) fn may_update_live(system: &Connection) -> bool {
    let Some(name) = system.unique_name() else {
        return false;
    };
    let answer = AuthorityProxyBlocking::new(system)
        .and_then(|authority| ask(&authority, name.as_str(), SYSTEM_UPDATE));
    match answer {
        Ok((authorized, _)) => authorized,
        Err(error) => {
            tracing::warn!("cannot check {SYSTEM_UPDATE}: {error}");
            false
        }
    }
}

pub fn follow_privileges(system: &Connection, updates: Updates) {
    let system = system.clone();
    spawn("privileges", move || match refused(&system) {
        Ok(refused) => {
            updates.send(Update::Refused(refused));
        }
        Err(error) => tracing::warn!("cannot ask polkit: {error}"),
    });
}
