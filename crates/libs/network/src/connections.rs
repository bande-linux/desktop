use std::collections::HashMap;
use std::io;
use std::net::Ipv4Addr;
use std::path::Path;
use std::process::Command;

use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::proxy::{
    ActiveConnectionProxyBlocking, DeviceProxyBlocking, Ip4ConfigProxyBlocking, WiredProxyBlocking,
};
use crate::{
    ACTIVE_STATE_ACTIVATED, ConnectionSettings, DEVICE_STATE_ACTIVATED, DEVICE_TYPE_ETHERNET,
    NetworkManager, Result,
};

pub(crate) const IGNORE_AUTO_DNS: &str = "ignore-auto-dns";
pub(crate) const IPV4: &str = "ipv4";
const DEVICE_STATE_UNMANAGED: u32 = 10;
const ACTIVE_STATE_ACTIVATING: u32 = 1;
const VPN_TYPES: [&str; 2] = ["vpn", "wireguard"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ethernet {
    pub interface: String,
    pub cable: bool,
    pub managed: bool,
    pub connected: bool,
    pub addressing: Addressing,
    pub manual: bool,
    pub connection: Option<String>,
    pub dns: Vec<Ipv4Addr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Addressing {
    pub address: Option<String>,
    pub router: Option<String>,
    pub dns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualIpv4 {
    pub address: Ipv4Addr,
    pub prefix: u32,
    pub router: Option<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VpnState {
    Off,
    Connecting,
    Connected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vpn {
    pub path: String,
    pub name: String,
    pub state: VpnState,
}

pub fn parse_manual(address: &str, router: &str, dns: &str) -> Option<ManualIpv4> {
    let (address, prefix) = match address.trim().split_once('/') {
        Some((address, prefix)) => (address, prefix.trim().parse().ok()?),
        None => (address.trim(), 24),
    };
    let router = router.trim();
    Some(ManualIpv4 {
        address: address.trim().parse().ok()?,
        prefix: Some(prefix).filter(|prefix| (1..=32).contains(prefix))?,
        router: if router.is_empty() {
            None
        } else {
            Some(router.parse().ok()?)
        },
        dns: parse_dns(dns)?,
    })
}

pub fn parse_dns(dns: &str) -> Option<Vec<Ipv4Addr>> {
    dns.split([',', ' '])
        .filter(|server| !server.is_empty())
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()
        .ok()
}

pub(crate) fn dns_override(settings: &ConnectionSettings) -> Vec<Ipv4Addr> {
    settings
        .get(IPV4)
        .filter(|ipv4| {
            ipv4.get(IGNORE_AUTO_DNS)
                .and_then(|value| bool::try_from(value).ok())
                .unwrap_or(false)
        })
        .map(dns_servers)
        .unwrap_or_default()
}

pub(crate) fn dns_servers(ipv4: &HashMap<String, OwnedValue>) -> Vec<Ipv4Addr> {
    ipv4.get("dns")
        .and_then(|dns| Vec::<u32>::try_from(dns.try_clone().ok()?).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|server| Ipv4Addr::from(server.to_le_bytes()))
        .collect()
}

pub(crate) fn dns_value(servers: &[Ipv4Addr]) -> Option<OwnedValue> {
    let servers: Vec<u32> = servers
        .iter()
        .map(|server| u32::from_le_bytes(server.octets()))
        .collect();
    owned(Value::from(servers))
}

fn owned(value: Value<'_>) -> Option<OwnedValue> {
    OwnedValue::try_from(value).ok()
}

pub fn ipv4_settings(manual: Option<&ManualIpv4>) -> HashMap<String, OwnedValue> {
    let mut ipv4 = HashMap::new();
    let Some(manual) = manual else {
        ipv4.extend(owned(Value::from("auto")).map(|method| ("method".to_owned(), method)));
        return ipv4;
    };
    let address = HashMap::from([
        (
            "address".to_owned(),
            Value::from(manual.address.to_string()),
        ),
        ("prefix".to_owned(), Value::from(manual.prefix)),
    ]);
    let entries = [
        ("method", owned(Value::from("manual"))),
        ("address-data", owned(Value::from(vec![address]))),
        (
            "gateway",
            manual
                .router
                .and_then(|router| owned(Value::from(router.to_string()))),
        ),
        ("dns", dns_value(&manual.dns)),
    ];
    ipv4.extend(
        entries
            .into_iter()
            .filter_map(|(key, value)| Some((key.to_owned(), value?))),
    );
    ipv4
}

pub(crate) fn configured_ipv4(settings: &ConnectionSettings) -> Option<ManualIpv4> {
    let ipv4 = settings.get(IPV4)?;
    if text(ipv4, "method")? != "manual" {
        return None;
    }
    let first =
        Vec::<HashMap<String, OwnedValue>>::try_from(ipv4.get("address-data")?.try_clone().ok()?)
            .ok()?
            .into_iter()
            .next()?;
    Some(ManualIpv4 {
        address: text(&first, "address")?.parse().ok()?,
        prefix: u32::try_from(first.get("prefix")?).ok()?,
        router: text(ipv4, "gateway").and_then(|router| router.parse().ok()),
        dns: dns_servers(ipv4),
    })
}

pub(crate) fn text(entry: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    String::try_from(entry.get(key)?.try_clone().ok()?).ok()
}

impl NetworkManager {
    fn ethernet_device(&self) -> Result<Option<OwnedObjectPath>> {
        Ok(self
            .devices_of_type(DEVICE_TYPE_ETHERNET)?
            .into_iter()
            .next())
    }

    pub(crate) fn device(&self, path: &OwnedObjectPath) -> Result<DeviceProxyBlocking<'_>> {
        Ok(DeviceProxyBlocking::builder(&self.bus)
            .path(path.clone())?
            .build()?)
    }

    fn connection_of(&self, device: &DeviceProxyBlocking<'_>) -> Result<Option<OwnedObjectPath>> {
        match self.active_connection_of(device)? {
            Some(connection) => Ok(Some(connection)),
            None => Ok(device.available_connections()?.into_iter().next()),
        }
    }

    pub(crate) fn active_connection_of(
        &self,
        device: &DeviceProxyBlocking<'_>,
    ) -> Result<Option<OwnedObjectPath>> {
        let active = device.active_connection()?;
        if active.as_str() == "/" {
            return Ok(None);
        }
        Ok(Some(
            ActiveConnectionProxyBlocking::builder(&self.bus)
                .path(active)?
                .build()?
                .connection()?,
        ))
    }

    pub fn ethernet(&self) -> Result<Option<Ethernet>> {
        let Some(path) = self.ethernet_device()? else {
            return Ok(None);
        };
        let device = self.device(&path)?;
        let wired = WiredProxyBlocking::builder(&self.bus)
            .path(path.clone())?
            .build()?;
        let state = device.state()?;
        let mut ethernet = Ethernet {
            interface: device.interface()?,
            cable: wired.carrier()?,
            managed: state != DEVICE_STATE_UNMANAGED,
            connected: state == DEVICE_STATE_ACTIVATED,
            addressing: Addressing::default(),
            manual: false,
            connection: None,
            dns: Vec::new(),
        };
        if ethernet.connected {
            ethernet.addressing = self.addressing(&device)?;
        }
        if let Some(connection) = self.connection_of(&device)? {
            let settings: ConnectionSettings =
                self.settings_connection(&connection)?.get_settings()?;
            ethernet.manual = configured_ipv4(&settings).is_some();
            ethernet.dns = dns_override(&settings);
            ethernet.connection = Some(connection.to_string());
        }
        Ok(Some(ethernet))
    }

    pub(crate) fn addressing(&self, device: &DeviceProxyBlocking<'_>) -> Result<Addressing> {
        let config = device.ip4_config()?;
        if config.as_str() == "/" {
            return Ok(Addressing::default());
        }
        let config = Ip4ConfigProxyBlocking::builder(&self.bus)
            .path(config)?
            .build()?;
        Ok(Addressing {
            address: config.address_data()?.first().and_then(|entry| {
                let prefix = u32::try_from(entry.get("prefix")?).ok()?;
                Some(format!("{}/{prefix}", text(entry, "address")?))
            }),
            router: Some(config.gateway()?).filter(|router| !router.is_empty()),
            dns: config
                .nameserver_data()?
                .iter()
                .filter_map(|entry| text(entry, "address"))
                .collect(),
        })
    }

    pub fn configure_ethernet(&self, manual: Option<&ManualIpv4>) -> Result<()> {
        let Some(path) = self.ethernet_device()? else {
            return Ok(());
        };
        let device = self.device(&path)?;
        let Some(connection) = self.connection_of(&device)? else {
            return Ok(());
        };
        self.edit_connection(&connection, |settings| {
            settings.insert(IPV4.to_owned(), ipv4_settings(manual));
        })?;
        self.reactivate(&connection, &path)
    }

    pub(crate) fn edit_connection(
        &self,
        connection: &OwnedObjectPath,
        edit: impl FnOnce(&mut ConnectionSettings),
    ) -> Result<()> {
        let settings_connection = self.settings_connection(connection)?;
        let mut settings = settings_connection.get_settings()?;
        edit(&mut settings);
        Ok(settings_connection.update(settings)?)
    }

    pub(crate) fn reactivate(
        &self,
        connection: &OwnedObjectPath,
        device: &OwnedObjectPath,
    ) -> Result<()> {
        self.manager()?.activate_connection(
            &connection.as_ref(),
            &device.as_ref(),
            &ObjectPath::from_static_str_unchecked("/"),
        )?;
        Ok(())
    }

    pub fn vpns(&self) -> Result<Vec<Vpn>> {
        let mut states: HashMap<String, u32> = HashMap::new();
        for active in self.manager()?.active_connections()? {
            let active = ActiveConnectionProxyBlocking::builder(&self.bus)
                .path(active)?
                .build()?;
            if let (Ok(connection), Ok(state)) = (active.connection(), active.state()) {
                states.insert(connection.to_string(), state);
            }
        }
        let mut vpns = Vec::new();
        for path in crate::proxy::SettingsProxyBlocking::new(&self.bus)?.list_connections()? {
            let settings = self.settings_connection(&path)?.get_settings()?;
            let Some(connection) = settings.get("connection") else {
                continue;
            };
            let kind = text(connection, "type").unwrap_or_default();
            if !VPN_TYPES.contains(&kind.as_str()) {
                continue;
            }
            let state = match states.get(path.as_str()) {
                Some(&ACTIVE_STATE_ACTIVATED) => VpnState::Connected,
                Some(&ACTIVE_STATE_ACTIVATING) => VpnState::Connecting,
                _ => VpnState::Off,
            };
            vpns.push(Vpn {
                path: path.to_string(),
                name: text(connection, "id").unwrap_or_default(),
                state,
            });
        }
        vpns.sort_by_key(|vpn| vpn.name.to_lowercase());
        Ok(vpns)
    }

    pub fn connect_vpn(&self, path: &str) -> Result<()> {
        let root = ObjectPath::from_static_str_unchecked("/");
        self.manager()?
            .activate_connection(&ObjectPath::try_from(path)?, &root, &root)?;
        Ok(())
    }

    pub fn remove_vpn(&self, path: &str) -> Result<()> {
        let connection = OwnedObjectPath::try_from(path.to_owned())?;
        Ok(self.settings_connection(&connection)?.delete()?)
    }

    pub fn disconnect_vpn(&self, path: &str) -> Result<()> {
        let manager = self.manager()?;
        for active in manager.active_connections()? {
            let connection = ActiveConnectionProxyBlocking::builder(&self.bus)
                .path(active.clone())?
                .build()?
                .connection()?;
            if connection.as_str() == path {
                manager.deactivate_connection(&active.as_ref())?;
            }
        }
        Ok(())
    }
}

pub fn vpn_type_of(file: &Path) -> Option<&'static str> {
    match file.extension()?.to_str()? {
        "conf" => Some("wireguard"),
        "ovpn" => Some("openvpn"),
        _ => None,
    }
}

pub fn import_vpn(file: &Path) -> io::Result<()> {
    let kind = vpn_type_of(file).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "choose a WireGuard .conf or an OpenVPN .ovpn file",
        )
    })?;
    let output = Command::new("nmcli")
        .args(["connection", "import", "type", kind, "file"])
        .arg(file)
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::other(
        String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    ))
}
