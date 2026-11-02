use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::connections::{IPV4, configured_ipv4, ipv4_settings, text};
use crate::proxy::SettingsProxyBlocking;
use crate::{
    Addressing, CONNECTION, ConnectionSettings, ManualIpv4, NetworkManager, Result, WIRELESS,
};

const AUTOCONNECT: &str = "autoconnect";
const METERED: &str = "metered";
const METERED_YES: i32 = 1;
const METERED_UNKNOWN: i32 = 0;
const ASSIGNED_MAC: &str = "assigned-mac-address";
const CLONED_MAC: &str = "cloned-mac-address";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateAddress {
    Off,
    Fixed,
    Rotating,
}

impl PrivateAddress {
    pub const ALL: [Self; 3] = [Self::Off, Self::Fixed, Self::Rotating];

    fn of(value: Option<&str>) -> Self {
        match value {
            Some("stable") => Self::Fixed,
            Some("random") => Self::Rotating,
            _ => Self::Off,
        }
    }

    fn setting(self) -> &'static str {
        match self {
            Self::Off => "permanent",
            Self::Fixed => "stable",
            Self::Rotating => "random",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownNetwork {
    pub ssid: Vec<u8>,
    pub path: String,
    pub auto_join: bool,
    pub low_data: bool,
    pub private_address: PrivateAddress,
    pub manual: Option<ManualIpv4>,
}

impl KnownNetwork {
    pub fn name(&self) -> String {
        String::from_utf8_lossy(&self.ssid).into_owned()
    }

    fn of(path: &OwnedObjectPath, settings: &ConnectionSettings) -> Option<Self> {
        let wireless = settings.get(WIRELESS)?;
        let ssid = Vec::<u8>::try_from(wireless.get("ssid")?.try_clone().ok()?).ok()?;
        let connection = settings.get(CONNECTION);
        let flag = |key: &str| {
            connection
                .and_then(|connection| connection.get(key))
                .and_then(|value| bool::try_from(value).ok())
        };
        let metered = connection
            .and_then(|connection| connection.get(METERED))
            .and_then(|value| i32::try_from(value).ok());
        Some(Self {
            ssid,
            path: path.to_string(),
            auto_join: flag(AUTOCONNECT).unwrap_or(true),
            low_data: metered == Some(METERED_YES),
            private_address: PrivateAddress::of(text(wireless, ASSIGNED_MAC).as_deref()),
            manual: configured_ipv4(settings),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WifiLink {
    pub hardware: String,
    pub addressing: Addressing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiChange {
    AutoJoin(bool),
    LowData(bool),
    PrivateAddress(PrivateAddress),
    Ipv4(Option<ManualIpv4>),
}

impl WifiChange {
    fn needs_reconnect(&self) -> bool {
        matches!(self, Self::PrivateAddress(_) | Self::Ipv4(_))
    }

    fn apply(&self, settings: &mut ConnectionSettings) {
        let owned = |value: Value<'_>| OwnedValue::try_from(value).ok();
        let mut set = |section: &str, key: &str, value: Option<OwnedValue>| {
            if let Some(value) = value {
                settings
                    .entry(section.to_owned())
                    .or_default()
                    .insert(key.to_owned(), value);
            }
        };
        match self {
            Self::AutoJoin(on) => set(CONNECTION, AUTOCONNECT, owned(Value::from(*on))),
            Self::LowData(on) => set(
                CONNECTION,
                METERED,
                owned(Value::from(if *on { METERED_YES } else { METERED_UNKNOWN })),
            ),
            Self::PrivateAddress(choice) => {
                set(WIRELESS, ASSIGNED_MAC, owned(Value::from(choice.setting())));
                if let Some(wireless) = settings.get_mut(WIRELESS) {
                    wireless.remove(CLONED_MAC);
                }
            }
            Self::Ipv4(manual) => {
                settings.insert(IPV4.to_owned(), ipv4_settings(manual.as_ref()));
            }
        }
    }
}

impl NetworkManager {
    pub fn known_networks(&self) -> Result<Vec<KnownNetwork>> {
        let mut known: Vec<KnownNetwork> = Vec::new();
        for path in SettingsProxyBlocking::new(&self.bus)?.list_connections()? {
            let settings = self.settings_connection(&path)?.get_settings()?;
            if let Some(network) = KnownNetwork::of(&path, &settings)
                && !known.iter().any(|seen| seen.ssid == network.ssid)
            {
                known.push(network);
            }
        }
        known.sort_by_key(|network| network.name().to_lowercase());
        Ok(known)
    }

    pub fn wifi_link(&self) -> Result<WifiLink> {
        let path = self.wifi_device()?;
        let device = self.device(&path)?;
        Ok(WifiLink {
            hardware: device.hw_address()?,
            addressing: self.addressing(&device)?,
        })
    }

    pub fn change_wifi(&self, path: &str, change: &WifiChange) -> Result<()> {
        let connection = OwnedObjectPath::try_from(path.to_owned())?;
        self.edit_connection(&connection, |settings| change.apply(settings))?;
        if !change.needs_reconnect() {
            return Ok(());
        }
        let device_path = self.wifi_device()?;
        let device = self.device(&device_path)?;
        if self.active_connection_of(&device)?.as_ref() == Some(&connection) {
            self.reactivate(&connection, &device_path)?;
        }
        Ok(())
    }
}
