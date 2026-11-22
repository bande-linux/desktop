use std::collections::HashMap;

use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};

use crate::connections::text;
use crate::proxy::{ActiveConnectionProxyBlocking, SettingsProxyBlocking};
use crate::{CONNECTION, Error, JoinOutcome, NetworkManager, Result, WIRELESS, WIRELESS_SECURITY};

const ACCESS_POINT_MODE: &str = "ap";
const PASSWORD_LENGTH: std::ops::RangeInclusive<usize> = 8..=63;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotspot {
    pub name: String,
    pub active: bool,
}

pub fn valid_hotspot_password(password: &str) -> bool {
    PASSWORD_LENGTH.contains(&password.len()) && password.chars().all(|c| (' '..='~').contains(&c))
}

impl NetworkManager {
    fn hotspots(&self) -> Result<Vec<(OwnedObjectPath, String)>> {
        let mut found = Vec::new();
        for path in SettingsProxyBlocking::new(&self.bus)?.list_connections()? {
            let settings = self.settings_connection(&path)?.get_settings()?;
            let Some(wireless) = settings.get(WIRELESS) else {
                continue;
            };
            if text(wireless, "mode").as_deref() != Some(ACCESS_POINT_MODE) {
                continue;
            }
            let ssid = wireless
                .get("ssid")
                .and_then(|ssid| Vec::<u8>::try_from(ssid.try_clone().ok()?).ok())
                .unwrap_or_default();
            found.push((path, String::from_utf8_lossy(&ssid).into_owned()));
        }
        Ok(found)
    }

    fn active_hotspots(&self) -> Result<Vec<OwnedObjectPath>> {
        let saved: Vec<OwnedObjectPath> =
            self.hotspots()?.into_iter().map(|(path, _)| path).collect();
        let mut active = Vec::new();
        for path in self.manager()?.active_connections()? {
            let connection = ActiveConnectionProxyBlocking::builder(&self.bus)
                .path(path.clone())?
                .build()?
                .connection()?;
            if saved.contains(&connection) {
                active.push(path);
            }
        }
        Ok(active)
    }

    pub fn hotspot(&self) -> Result<Option<Hotspot>> {
        let Some((_, name)) = self.hotspots()?.into_iter().next() else {
            return Ok(None);
        };
        Ok(Some(Hotspot {
            name,
            active: !self.active_hotspots()?.is_empty(),
        }))
    }

    pub fn start_hotspot(&self, name: &str, password: Option<&str>) -> Result<JoinOutcome> {
        let device = self.wifi_device()?;
        let root = ObjectPath::from_static_str_unchecked(crate::ROOT);
        let saved = self.hotspots()?;
        let Some(password) = password else {
            let (connection, _) = saved.into_iter().next().ok_or(Error::PasswordRequired)?;
            let active = self
                .manager()?
                .activate_connection(&connection, &device, &root)?;
            return self.wait_for_activation(&active);
        };
        if !valid_hotspot_password(password) {
            return Err(Error::PasswordRequired);
        }
        for (connection, _) in saved {
            self.settings_connection(&connection)?.delete()?;
        }
        let settings = HashMap::from([
            (
                CONNECTION,
                HashMap::from([
                    ("id", Value::from(name.to_owned())),
                    ("autoconnect", Value::from(false)),
                ]),
            ),
            (
                WIRELESS,
                HashMap::from([
                    ("ssid", Value::from(name.as_bytes().to_vec())),
                    ("mode", Value::from(ACCESS_POINT_MODE)),
                ]),
            ),
            (
                WIRELESS_SECURITY,
                HashMap::from([
                    ("key-mgmt", Value::from("wpa-psk")),
                    ("psk", Value::from(password.to_owned())),
                    ("proto", Value::from(vec!["rsn"])),
                    ("pairwise", Value::from(vec!["ccmp"])),
                    ("group", Value::from(vec!["ccmp"])),
                ]),
            ),
            ("ipv4", HashMap::from([("method", Value::from("shared"))])),
            ("ipv6", HashMap::from([("method", Value::from("ignore"))])),
        ]);
        let (_, active) = self
            .manager()?
            .add_and_activate_connection(settings, &device, &root)
            .map_err(crate::refused)?;
        self.wait_for_activation(&active)
    }

    pub fn stop_hotspot(&self) -> Result<()> {
        let manager = self.manager()?;
        for active in self.active_hotspots()? {
            manager.deactivate_connection(&active.as_ref())?;
        }
        Ok(())
    }
}
