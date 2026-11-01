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

const IPV4: &str = "ipv4";
const DEVICE_STATE_UNMANAGED: u32 = 10;
const ACTIVE_STATE_ACTIVATING: u32 = 1;
const VPN_TYPES: [&str; 2] = ["vpn", "wireguard"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ethernet {
    pub interface: String,
    pub cable: bool,
    pub managed: bool,
    pub connected: bool,
    pub address: Option<String>,
    pub router: Option<String>,
    pub dns: Vec<String>,
    pub manual: bool,
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
        dns: dns
            .split([',', ' '])
            .filter(|server| !server.is_empty())
            .map(str::parse)
            .collect::<std::result::Result<_, _>>()
            .ok()?,
    })
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
    let dns: Vec<u32> = manual
        .dns
        .iter()
        .map(|server| u32::from_le_bytes(server.octets()))
        .collect();
    let entries = [
        ("method", owned(Value::from("manual"))),
        ("address-data", owned(Value::from(vec![address]))),
        (
            "gateway",
            manual
                .router
                .and_then(|router| owned(Value::from(router.to_string()))),
        ),
        ("dns", owned(Value::from(dns))),
    ];
    ipv4.extend(
        entries
            .into_iter()
            .filter_map(|(key, value)| Some((key.to_owned(), value?))),
    );
    ipv4
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

    fn device(&self, path: &OwnedObjectPath) -> Result<DeviceProxyBlocking<'_>> {
        Ok(DeviceProxyBlocking::builder(&self.bus)
            .path(path.clone())?
            .build()?)
    }

    fn connection_of(&self, device: &DeviceProxyBlocking<'_>) -> Result<Option<OwnedObjectPath>> {
        let active = device.active_connection()?;
        if active.as_str() != "/" {
            return Ok(Some(
                ActiveConnectionProxyBlocking::builder(&self.bus)
                    .path(active)?
                    .build()?
                    .connection()?,
            ));
        }
        Ok(device.available_connections()?.into_iter().next())
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
            address: None,
            router: None,
            dns: Vec::new(),
            manual: false,
        };
        let config = device.ip4_config()?;
        if ethernet.connected && config.as_str() != "/" {
            let config = Ip4ConfigProxyBlocking::builder(&self.bus)
                .path(config)?
                .build()?;
            ethernet.address = config.address_data()?.first().and_then(|entry| {
                let prefix = u32::try_from(entry.get("prefix")?).ok()?;
                Some(format!("{}/{prefix}", text(entry, "address")?))
            });
            ethernet.router = Some(config.gateway()?).filter(|router| !router.is_empty());
            ethernet.dns = config
                .nameserver_data()?
                .iter()
                .filter_map(|entry| text(entry, "address"))
                .collect();
        }
        if let Some(connection) = self.connection_of(&device)? {
            let settings: ConnectionSettings =
                self.settings_connection(&connection)?.get_settings()?;
            ethernet.manual = settings
                .get(IPV4)
                .and_then(|ipv4| text(ipv4, "method"))
                .is_some_and(|method| method == "manual");
        }
        Ok(Some(ethernet))
    }

    pub fn configure_ethernet(&self, manual: Option<&ManualIpv4>) -> Result<()> {
        let Some(path) = self.ethernet_device()? else {
            return Ok(());
        };
        let device = self.device(&path)?;
        let Some(connection) = self.connection_of(&device)? else {
            return Ok(());
        };
        let settings_connection = self.settings_connection(&connection)?;
        let mut settings = settings_connection.get_settings()?;
        settings.insert(IPV4.to_owned(), ipv4_settings(manual));
        settings_connection.update(settings)?;
        self.manager()?.activate_connection(
            &connection.as_ref(),
            &path.as_ref(),
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
