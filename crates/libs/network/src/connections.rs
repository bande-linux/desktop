use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::process::Command;

use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::enterprise::ENTERPRISE;
use crate::proxy::{
    ActiveConnectionProxyBlocking, DeviceProxyBlocking, Ip4ConfigProxyBlocking, WiredProxyBlocking,
};
use crate::{
    ACTIVE_STATE_ACTIVATED, ConnectionSettings, DEVICE_STATE_ACTIVATED, DEVICE_TYPE_ETHERNET,
    Enterprise, NetworkManager, Result, ifupdown, kernel,
};

pub(crate) const IGNORE_AUTO_DNS: &str = "ignore-auto-dns";
pub(crate) const IPV4: &str = "ipv4";
pub(crate) const IPV6: &str = "ipv6";
const IFDOWN: &str = "/usr/sbin/ifdown";
const OPENVPN_PLUGIN: &str = "/usr/lib/NetworkManager/VPN/nm-openvpn-service.name";
const OPENVPN_PACKAGE: &str = "network-manager-openvpn";
pub(crate) const DEVICE_STATE_UNMANAGED: u32 = 10;
const ACTIVE_STATE_ACTIVATING: u32 = 1;
const VPN_TYPES: [&str; 2] = ["vpn", "wireguard"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outside {
    pub interface: String,
    pub addressing: Addressing,
    pub configured_in: Option<PathBuf>,
    pub hand_over: bool,
}

impl Outside {
    pub(crate) fn of(interface: &str) -> Self {
        let configuration = ifupdown::read();
        let configured_in = configuration
            .as_ref()
            .ok()
            .and_then(|configuration| ifupdown::configuring_file(configuration, interface));
        Self {
            interface: interface.to_owned(),
            addressing: kernel::addressing(interface),
            hand_over: Path::new(IFDOWN).exists()
                && (configured_in.is_some() || configuration.is_err()),
            configured_in,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ethernet {
    pub interface: String,
    pub cable: bool,
    pub outside: Option<Outside>,
    pub connected: bool,
    pub addressing: Addressing,
    pub manual: bool,
    pub ipv6: Ipv6,
    pub connection: Option<String>,
    pub dns: Vec<Ipv4Addr>,
    pub security: Option<Enterprise>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManualIpv6 {
    pub address: Ipv6Addr,
    pub prefix: u32,
    pub router: Option<Ipv6Addr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ipv6 {
    #[default]
    Automatic,
    Manual(ManualIpv6),
    Off,
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

pub fn parse_manual_ipv6(address: &str, router: &str) -> Option<ManualIpv6> {
    let (address, prefix) = match address.trim().split_once('/') {
        Some((address, prefix)) => (address, prefix.trim().parse().ok()?),
        None => (address.trim(), 64),
    };
    let router = router.trim();
    Some(ManualIpv6 {
        address: address.trim().parse().ok()?,
        prefix: Some(prefix).filter(|prefix| (1..=128).contains(prefix))?,
        router: if router.is_empty() {
            None
        } else {
            Some(router.parse().ok()?)
        },
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

pub(crate) fn merge_ipv4(settings: &mut ConnectionSettings, manual: Option<&ManualIpv4>) {
    let ipv4 = settings.entry(IPV4.to_owned()).or_default();
    let keeps_dns = manual.is_none()
        && ipv4
            .get(IGNORE_AUTO_DNS)
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(false);
    for key in ["method", "address-data", "addresses", "gateway"] {
        ipv4.remove(key);
    }
    if !keeps_dns {
        ipv4.remove("dns");
    }
    ipv4.extend(ipv4_settings(manual));
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

pub(crate) fn ipv6_settings(ipv6: Ipv6) -> HashMap<String, OwnedValue> {
    let address = |manual: &ManualIpv6| {
        HashMap::from([
            (
                "address".to_owned(),
                Value::from(manual.address.to_string()),
            ),
            ("prefix".to_owned(), Value::from(manual.prefix)),
        ])
    };
    let entries = match ipv6 {
        Ipv6::Automatic => vec![("method", owned(Value::from("auto")))],
        Ipv6::Off => vec![("method", owned(Value::from("disabled")))],
        Ipv6::Manual(manual) => vec![
            ("method", owned(Value::from("manual"))),
            ("address-data", owned(Value::from(vec![address(&manual)]))),
            (
                "gateway",
                manual
                    .router
                    .and_then(|router| owned(Value::from(router.to_string()))),
            ),
        ],
    };
    entries
        .into_iter()
        .filter_map(|(key, value)| Some((key.to_owned(), value?)))
        .collect()
}

pub(crate) fn merge_ipv6(settings: &mut ConnectionSettings, ipv6: Ipv6) {
    let section = settings.entry(IPV6.to_owned()).or_default();
    for key in ["method", "address-data", "addresses", "gateway"] {
        section.remove(key);
    }
    section.extend(ipv6_settings(ipv6));
}

pub(crate) fn configured_ipv6(settings: &ConnectionSettings) -> Ipv6 {
    let Some(section) = settings.get(IPV6) else {
        return Ipv6::Automatic;
    };
    match text(section, "method").as_deref() {
        Some("disabled" | "ignore") => Ipv6::Off,
        Some("manual") => {
            let manual = || {
                let first = Vec::<HashMap<String, OwnedValue>>::try_from(
                    section.get("address-data")?.try_clone().ok()?,
                )
                .ok()?
                .into_iter()
                .next()?;
                Some(ManualIpv6 {
                    address: text(&first, "address")?.parse().ok()?,
                    prefix: u32::try_from(first.get("prefix")?).ok()?,
                    router: text(section, "gateway").and_then(|router| router.parse().ok()),
                })
            };
            manual().map_or(Ipv6::Automatic, Ipv6::Manual)
        }
        _ => Ipv6::Automatic,
    }
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

    pub fn ethernets(&self) -> Result<Vec<Ethernet>> {
        let mut found = Vec::new();
        for path in self.devices_of_type(DEVICE_TYPE_ETHERNET)? {
            found.push(self.ethernet(&path)?);
        }
        found.sort_by(|a, b| a.interface.cmp(&b.interface));
        Ok(found)
    }

    fn ethernet(&self, path: &OwnedObjectPath) -> Result<Ethernet> {
        let device = self.device(path)?;
        let wired = WiredProxyBlocking::builder(&self.bus)
            .path(path.clone())?
            .build()?;
        let state = device.state()?;
        let interface = device.interface()?;
        let outside = (state == DEVICE_STATE_UNMANAGED).then(|| Outside::of(&interface));
        let mut ethernet = Ethernet {
            interface,
            cable: wired.carrier()?,
            connected: state == DEVICE_STATE_ACTIVATED,
            addressing: Addressing::default(),
            manual: false,
            ipv6: Ipv6::Automatic,
            connection: None,
            dns: Vec::new(),
            security: None,
            outside,
        };
        if let Some(outside) = &ethernet.outside {
            ethernet.addressing = outside.addressing.clone();
            return Ok(ethernet);
        }
        if ethernet.connected {
            ethernet.addressing = self.addressing(&device)?;
        }
        if let Some(connection) = self.connection_of(&device)? {
            let settings: ConnectionSettings =
                self.settings_connection(&connection)?.get_settings()?;
            ethernet.manual = configured_ipv4(&settings).is_some();
            ethernet.ipv6 = configured_ipv6(&settings);
            ethernet.dns = dns_override(&settings);
            ethernet.security = settings.get(ENTERPRISE).and_then(Enterprise::of);
            ethernet.connection = Some(connection.to_string());
        }
        Ok(ethernet)
    }

    pub(crate) fn device_named(&self, interface: &str) -> Result<Option<OwnedObjectPath>> {
        for path in self.manager()?.get_devices()? {
            if self.device(&path)?.interface()? == interface {
                return Ok(Some(path));
            }
        }
        Ok(None)
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

    fn edit_wired(
        &self,
        interface: &str,
        edit: impl FnOnce(&mut ConnectionSettings),
    ) -> Result<()> {
        let Some(path) = self.device_named(interface)? else {
            return Ok(());
        };
        let device = self.device(&path)?;
        let Some(connection) = self.connection_of(&device)? else {
            return Ok(());
        };
        self.edit_connection(&connection, edit)?;
        self.reactivate(&connection, &path)
    }

    pub fn configure_ethernet(&self, interface: &str, manual: Option<&ManualIpv4>) -> Result<()> {
        self.edit_wired(interface, |settings| merge_ipv4(settings, manual))
    }

    pub fn secure_ethernet(&self, interface: &str, login: Option<&Enterprise>) -> Result<()> {
        self.edit_wired(interface, |settings| match login {
            Some(login) => {
                settings.insert(ENTERPRISE.to_owned(), login.owned_settings());
            }
            None => {
                settings.remove(ENTERPRISE);
            }
        })
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

pub fn missing_vpn_plugin(file: &Path) -> Option<&'static str> {
    match vpn_type_of(file)? {
        "openvpn" if !Path::new(OPENVPN_PLUGIN).exists() => Some(OPENVPN_PACKAGE),
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
