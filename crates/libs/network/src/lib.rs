mod connections;
mod proxy;

pub use connections::{
    Ethernet, ManualIpv4, Vpn, VpnState, import_vpn, ipv4_settings, parse_manual, vpn_type_of,
};
use std::collections::HashMap;
use std::fmt;

use proxy::{
    AccessPointProxyBlocking, ActiveConnectionProxyBlocking, DeviceProxyBlocking,
    NetworkManagerProxyBlocking, SettingsConnectionProxyBlocking, SettingsProxyBlocking,
    WiredProxyBlocking, WirelessProxyBlocking,
};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const DEVICE_TYPE_ETHERNET: u32 = 1;
const DEVICE_TYPE_WIFI: u32 = 2;
const DEVICE_STATE_ACTIVATED: u32 = 100;
const AP_FLAGS_PRIVACY: u32 = 0x1;
const AP_SEC_KEY_MGMT_PSK: u32 = 0x100;
const AP_SEC_KEY_MGMT_802_1X: u32 = 0x200;
const AP_SEC_KEY_MGMT_SAE: u32 = 0x400;
const ACTIVE_STATE_ACTIVATED: u32 = 2;
const ACTIVE_STATE_DEACTIVATED: u32 = 4;
const ACTIVE_REASON_NO_SECRETS: u32 = 9;
const CONNECTIVITY_PORTAL: u32 = 2;
const ACTIVE_REASON_LOGIN_FAILED: u32 = 10;
const WIRELESS: &str = "802-11-wireless";
const WIRELESS_SECURITY: &str = "802-11-wireless-security";
const CONNECTION: &str = "connection";
const INTERFACE_NAME: &str = "interface-name";

#[derive(Debug)]
pub enum Error {
    Bus(zbus::Error),
    NoWifiDevice,
    PasswordRequired,
    Refused(String),
    Unsupported(Security),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bus(error) => write!(f, "NetworkManager: {error}"),
            Self::NoWifiDevice => f.write_str("this computer has no Wi-Fi device"),
            Self::PasswordRequired => f.write_str("the network requires a password"),
            Self::Refused(reason) => f.write_str(reason),
            Self::Unsupported(security) => write!(f, "cannot join {security:?} networks yet"),
        }
    }
}

impl std::error::Error for Error {}

impl From<zbus::Error> for Error {
    fn from(error: zbus::Error) -> Self {
        Self::Bus(error)
    }
}

impl From<zbus::zvariant::Error> for Error {
    fn from(error: zbus::zvariant::Error) -> Self {
        Self::Bus(error.into())
    }
}

impl From<zbus::fdo::Error> for Error {
    fn from(error: zbus::fdo::Error) -> Self {
        Self::Bus(error.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn refused(error: zbus::Error) -> Error {
    match error {
        zbus::Error::MethodError(_, Some(reason), _) => Error::Refused(reason),
        other => Error::Bus(other),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    Open,
    Personal { sae_only: bool },
    Enterprise,
    Wep,
}

impl Security {
    fn of(flags: u32, wpa_flags: u32, rsn_flags: u32) -> Self {
        let key_management = wpa_flags | rsn_flags;
        if key_management & AP_SEC_KEY_MGMT_802_1X != 0 {
            Self::Enterprise
        } else if key_management & (AP_SEC_KEY_MGMT_PSK | AP_SEC_KEY_MGMT_SAE) != 0 {
            Self::Personal {
                sae_only: key_management & AP_SEC_KEY_MGMT_PSK == 0,
            }
        } else if flags & AP_FLAGS_PRIVACY != 0 {
            Self::Wep
        } else {
            Self::Open
        }
    }

    pub fn needs_password(self) -> bool {
        self != Self::Open
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub ssid: Vec<u8>,
    pub strength: u8,
    pub security: Security,
    pub known: bool,
    pub current: bool,
    access_point: OwnedObjectPath,
}

impl Network {
    pub fn name(&self) -> String {
        String::from_utf8_lossy(&self.ssid).into_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wired {
    pub cable: bool,
    pub connected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinOutcome {
    Joined,
    WrongPassword,
    Failed { reason: u32 },
}

impl fmt::Display for JoinOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Joined => "joined",
            Self::WrongPassword => "the password is not correct",
            Self::Failed { reason } => match *reason {
                2 => "the connection was turned off",
                3 => "the Wi-Fi device disconnected",
                4 | 7 | 8 => "a network service stopped or did not start",
                5 => "the network gave no usable address",
                6 => "the network did not answer in time",
                11 => "the saved network was removed",
                12 => "a connection it depends on failed",
                13 | 14 => "the Wi-Fi device went away",
                _ => "the connection failed",
            },
        })
    }
}

pub struct NetworkManager {
    bus: Connection,
}

type ConnectionSettings = HashMap<String, HashMap<String, OwnedValue>>;

impl NetworkManager {
    pub fn new(system: &Connection) -> Self {
        Self {
            bus: system.clone(),
        }
    }

    fn manager(&self) -> Result<NetworkManagerProxyBlocking<'_>> {
        Ok(NetworkManagerProxyBlocking::new(&self.bus)?)
    }

    pub fn wifi_enabled(&self) -> Result<bool> {
        Ok(self.manager()?.wireless_enabled()?)
    }

    pub fn set_wifi_enabled(&self, enabled: bool) -> Result<()> {
        Ok(self.manager()?.set_wireless_enabled(enabled)?)
    }

    pub fn sign_in_page(&self) -> Result<Option<String>> {
        let manager = self.manager()?;
        if manager.connectivity()? != CONNECTIVITY_PORTAL {
            return Ok(None);
        }
        Ok(Some(manager.connectivity_check_uri()?).filter(|uri| !uri.is_empty()))
    }

    fn devices_of_type(&self, device_type: u32) -> Result<Vec<OwnedObjectPath>> {
        let mut found = Vec::new();
        for path in self.manager()?.get_devices()? {
            let device = DeviceProxyBlocking::builder(&self.bus)
                .path(path.clone())?
                .build()?;
            if device.device_type()? == device_type {
                found.push(path);
            }
        }
        Ok(found)
    }

    fn wifi_device(&self) -> Result<OwnedObjectPath> {
        self.devices_of_type(DEVICE_TYPE_WIFI)?
            .into_iter()
            .next()
            .ok_or(Error::NoWifiDevice)
    }

    fn wireless(&self, path: &OwnedObjectPath) -> Result<WirelessProxyBlocking<'_>> {
        Ok(WirelessProxyBlocking::builder(&self.bus)
            .path(path.clone())?
            .build()?)
    }

    pub fn has_wifi(&self) -> Result<bool> {
        Ok(!self.devices_of_type(DEVICE_TYPE_WIFI)?.is_empty())
    }

    pub fn wired(&self) -> Result<Option<Wired>> {
        let Some(path) = self
            .devices_of_type(DEVICE_TYPE_ETHERNET)?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        let device = DeviceProxyBlocking::builder(&self.bus)
            .path(path.clone())?
            .build()?;
        let wired = WiredProxyBlocking::builder(&self.bus)
            .path(path.clone())?
            .build()?;
        Ok(Some(Wired {
            cable: wired.carrier()?,
            connected: device.state()? == DEVICE_STATE_ACTIVATED,
        }))
    }

    pub fn scan(&self) -> Result<()> {
        let device = self.wifi_device()?;
        Ok(self.wireless(&device)?.request_scan(HashMap::new())?)
    }

    fn saved_wifi(&self) -> Result<Vec<(Vec<u8>, OwnedObjectPath)>> {
        let mut saved = Vec::new();
        for path in SettingsProxyBlocking::new(&self.bus)?.list_connections()? {
            let settings = self.settings_connection(&path)?.get_settings()?;
            if let Some(ssid) = settings
                .get(WIRELESS)
                .and_then(|wireless| wireless.get("ssid"))
                .and_then(|ssid| Vec::<u8>::try_from(ssid.clone()).ok())
            {
                saved.push((ssid, path));
            }
        }
        Ok(saved)
    }

    fn settings_connection(
        &self,
        path: &OwnedObjectPath,
    ) -> Result<SettingsConnectionProxyBlocking<'_>> {
        Ok(SettingsConnectionProxyBlocking::builder(&self.bus)
            .path(path.clone())?
            .build()?)
    }

    pub fn networks(&self) -> Result<Vec<Network>> {
        let device = self.wifi_device()?;
        let wireless = self.wireless(&device)?;
        let active = wireless.active_access_point()?;
        let saved = self.saved_wifi()?;
        let mut networks: Vec<Network> = Vec::new();
        for path in wireless.get_all_access_points()? {
            let point = AccessPointProxyBlocking::builder(&self.bus)
                .path(path.clone())?
                .build()?;
            let ssid = point.ssid()?;
            if ssid.is_empty() {
                continue;
            }
            let network = Network {
                strength: point.strength()?,
                security: Security::of(point.flags()?, point.wpa_flags()?, point.rsn_flags()?),
                known: saved.iter().any(|(saved, _)| *saved == ssid),
                current: path == active,
                ssid,
                access_point: path,
            };
            match networks.iter_mut().find(|seen| seen.ssid == network.ssid) {
                Some(seen) => {
                    let current = seen.current || network.current;
                    if network.strength > seen.strength {
                        *seen = network;
                    }
                    seen.current = current;
                }
                None => networks.push(network),
            }
        }
        networks.sort_by(|a, b| {
            b.current
                .cmp(&a.current)
                .then(b.known.cmp(&a.known))
                .then(b.strength.cmp(&a.strength))
                .then(a.ssid.cmp(&b.ssid))
        });
        Ok(networks)
    }

    pub fn join(&self, network: &Network, password: Option<&str>) -> Result<JoinOutcome> {
        let key_management = match network.security {
            Security::Open => None,
            Security::Personal { sae_only: false } => Some("wpa-psk"),
            Security::Personal { sae_only: true } => Some("sae"),
            other => return Err(Error::Unsupported(other)),
        };
        let device = self.wifi_device()?;
        let manager = self.manager()?;
        let saved = self
            .saved_wifi()?
            .into_iter()
            .find(|(ssid, _)| *ssid == network.ssid)
            .map(|(_, path)| path);
        let (added, active) = match (saved, password) {
            (Some(connection), password) => {
                if let Some(psk) = password {
                    self.replace_password(&connection, psk)?;
                }
                let activate = || {
                    manager
                        .activate_connection(&connection, &device, &network.access_point)
                        .map_err(refused)
                };
                let active = match activate() {
                    Err(_) if self.release_stale_interface(&connection)? => activate()?,
                    activated => activated?,
                };
                (None, active)
            }
            (None, None) if key_management.is_some() => return Err(Error::PasswordRequired),
            (None, password) => {
                let mut settings = HashMap::from([(
                    WIRELESS,
                    HashMap::from([("ssid", Value::from(network.ssid.clone()))]),
                )]);
                if let (Some(key_management), Some(psk)) = (key_management, password) {
                    settings.insert(
                        WIRELESS_SECURITY,
                        HashMap::from([
                            ("key-mgmt", Value::from(key_management)),
                            ("psk", Value::from(psk)),
                        ]),
                    );
                }
                let (connection, active) = manager
                    .add_and_activate_connection(settings, &device, &network.access_point)
                    .map_err(refused)?;
                (Some(connection), active)
            }
        };
        let outcome = self.wait_for_activation(&active)?;
        if let Some(connection) = added
            && outcome != JoinOutcome::Joined
        {
            self.settings_connection(&connection)?.delete()?;
        }
        Ok(outcome)
    }

    pub fn forget(&self, network: &Network) -> Result<()> {
        for (ssid, connection) in self.saved_wifi()? {
            if ssid == network.ssid {
                self.settings_connection(&connection)?.delete()?;
            }
        }
        Ok(())
    }

    fn interfaces(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for path in self.manager()?.get_devices()? {
            names.push(
                DeviceProxyBlocking::builder(&self.bus)
                    .path(path)?
                    .build()?
                    .interface()?,
            );
        }
        Ok(names)
    }

    fn release_stale_interface(&self, connection: &OwnedObjectPath) -> Result<bool> {
        let proxy = self.settings_connection(connection)?;
        let mut settings: ConnectionSettings = proxy.get_settings()?;
        let Some(section) = settings.get_mut(CONNECTION) else {
            return Ok(false);
        };
        let bound = connections::text(section, INTERFACE_NAME).unwrap_or_default();
        if bound.is_empty() || self.interfaces()?.contains(&bound) {
            return Ok(false);
        }
        section.remove(INTERFACE_NAME);
        proxy.update(settings)?;
        Ok(true)
    }

    fn replace_password(&self, connection: &OwnedObjectPath, psk: &str) -> Result<()> {
        let proxy = self.settings_connection(connection)?;
        let mut settings: ConnectionSettings = proxy.get_settings()?;
        settings
            .entry(WIRELESS_SECURITY.to_owned())
            .or_default()
            .insert("psk".to_owned(), OwnedValue::try_from(Value::from(psk))?);
        Ok(proxy.update(settings)?)
    }

    fn wait_for_activation(&self, path: &OwnedObjectPath) -> Result<JoinOutcome> {
        let active = ActiveConnectionProxyBlocking::builder(&self.bus)
            .path(path.clone())?
            .build()?;
        let changes = active.receive_activation_state_changed()?;
        let outcome = |state: u32, reason: u32| match state {
            ACTIVE_STATE_ACTIVATED => Some(JoinOutcome::Joined),
            ACTIVE_STATE_DEACTIVATED => Some(match reason {
                ACTIVE_REASON_NO_SECRETS | ACTIVE_REASON_LOGIN_FAILED => JoinOutcome::WrongPassword,
                reason => JoinOutcome::Failed { reason },
            }),
            _ => None,
        };
        if let Some(done) = outcome(active.state()?, 0) {
            return Ok(done);
        }
        for change in changes {
            let args = change.args()?;
            if let Some(done) = outcome(args.state, args.reason) {
                return Ok(done);
            }
        }
        Err(Error::Bus(zbus::Error::Failure(
            "NetworkManager left the bus during activation".to_owned(),
        )))
    }
}
