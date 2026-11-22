use std::collections::HashMap;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use bande_l10n::tr;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::connections::{DEVICE_STATE_UNMANAGED, IPV4, IPV6, ipv4_settings, ipv6_settings};
use crate::ifupdown::{Ipv4Setup, Plan};
use crate::proxy::{ActiveConnectionProxyBlocking, SettingsProxyBlocking};
use crate::{
    ACTIVE_STATE_ACTIVATED, CONNECTION, ConnectionSettings, Error, NetworkManager, ROOT, Result,
    WIRELESS, WIRELESS_SECURITY,
};

const ETHERNET: &str = "802-3-ethernet";
const RELOAD_EVERYTHING: u32 = 0;
const MANAGED_WAIT: Duration = Duration::from_millis(250);
const MANAGED_TRIES: u32 = 40;
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(90);
const UUID_SOURCE: &str = "/proc/sys/kernel/random/uuid";

fn owned(value: Value<'_>) -> Option<OwnedValue> {
    OwnedValue::try_from(value).ok()
}

fn section(entries: Vec<(&str, Option<OwnedValue>)>) -> HashMap<String, OwnedValue> {
    entries
        .into_iter()
        .filter_map(|(key, value)| Some((key.to_owned(), value?)))
        .collect()
}

fn connection_settings(plan: &Plan, uuid: &str) -> (String, ConnectionSettings) {
    let setup = &plan.setup;
    let (id, kind) = match &setup.wifi {
        Some(wifi) => (String::from_utf8_lossy(&wifi.ssid).into_owned(), WIRELESS),
        None => (plan.interface.clone(), ETHERNET),
    };
    let mut settings = ConnectionSettings::new();
    settings.insert(
        CONNECTION.to_owned(),
        section(vec![
            ("id", owned(Value::from(id.as_str()))),
            ("uuid", owned(Value::from(uuid))),
            ("type", owned(Value::from(kind))),
            (
                "interface-name",
                owned(Value::from(plan.interface.as_str())),
            ),
            ("autoconnect", owned(Value::from(true))),
        ]),
    );
    let mut link = section(vec![
        ("mtu", setup.mtu.and_then(|mtu| owned(Value::from(mtu)))),
        (
            "cloned-mac-address",
            setup
                .hardware_address
                .as_deref()
                .and_then(|address| owned(Value::from(address))),
        ),
    ]);
    if let Some(wifi) = &setup.wifi {
        link.extend(section(vec![
            ("ssid", owned(Value::from(wifi.ssid.clone()))),
            ("mode", owned(Value::from("infrastructure"))),
            ("hidden", owned(Value::from(wifi.hidden))),
        ]));
        if let Some(psk) = &wifi.psk {
            settings.insert(
                WIRELESS_SECURITY.to_owned(),
                section(vec![
                    ("key-mgmt", owned(Value::from("wpa-psk"))),
                    ("psk", owned(Value::from(psk.as_str()))),
                ]),
            );
        }
    }
    settings.insert(kind.to_owned(), link);
    let mut ipv4 = match &setup.ipv4 {
        Ipv4Setup::Automatic => ipv4_settings(None),
        Ipv4Setup::Manual(manual) => ipv4_settings(Some(manual)),
        Ipv4Setup::Off => section(vec![("method", owned(Value::from("disabled")))]),
    };
    ipv4.extend(section(vec![
        (
            "dhcp-hostname",
            setup
                .dhcp_hostname
                .as_deref()
                .and_then(|name| owned(Value::from(name))),
        ),
        (
            "dns-search",
            (!setup.dns_search.is_empty())
                .then(|| owned(Value::from(setup.dns_search.clone())))
                .flatten(),
        ),
    ]));
    settings.insert(IPV4.to_owned(), ipv4);
    let mut ipv6 = ipv6_settings(setup.ipv6);
    if !setup.ipv6_dns.is_empty() {
        let servers: Vec<Vec<u8>> = setup
            .ipv6_dns
            .iter()
            .map(|server| server.octets().to_vec())
            .collect();
        ipv6.extend(section(vec![("dns", owned(Value::from(servers)))]));
    }
    settings.insert(IPV6.to_owned(), ipv6);
    (id, settings)
}

#[derive(Default)]
struct Taken {
    device: Option<OwnedObjectPath>,
    connection: Option<OwnedObjectPath>,
}

impl NetworkManager {
    /// Hands an interface that ifupdown no longer configures to NetworkManager, with a saved system connection equivalent to `plan`, and returns that connection's name once it is connected. On any failure it deletes that connection and leaves the device unmanaged again.
    pub fn take_over(&self, plan: &Plan) -> Result<String> {
        let mut taken = Taken::default();
        let outcome = self.hand_over(plan, &mut taken);
        if let Err(error) = outcome {
            let leftovers = self.give_back(&taken);
            if leftovers.is_empty() {
                return Err(error);
            }
            return Err(Error::Refused(format!("{error}; {}", leftovers.join("; "))));
        }
        outcome
    }

    fn hand_over(&self, plan: &Plan, taken: &mut Taken) -> Result<String> {
        let settings = SettingsProxyBlocking::new(&self.bus)?;
        settings.reload_connections()?;
        self.manager()?.reload(RELOAD_EVERYTHING)?;
        let missing = || {
            Error::Refused(tr!(
                "NetworkManager has no device called {interface}",
                interface = plan.interface.clone()
            ))
        };
        let path = self.device_named(&plan.interface)?.ok_or_else(missing)?;
        let device = self.device(&path)?;
        let mut tries = 0;
        while device.state()? == DEVICE_STATE_UNMANAGED {
            if tries == 0 {
                device.set_managed(true)?;
                taken.device = Some(path.clone());
            }
            tries += 1;
            if tries > MANAGED_TRIES {
                return Err(Error::Refused(tr!(
                    "NetworkManager still leaves {interface} alone",
                    interface = plan.interface.clone()
                )));
            }
            thread::sleep(MANAGED_WAIT);
        }
        let uuid =
            fs::read_to_string(UUID_SOURCE).map_err(|error| Error::Refused(error.to_string()))?;
        let (id, connection) = connection_settings(plan, uuid.trim());
        let connection = settings.add_connection(connection)?;
        taken.connection = Some(connection.clone());
        let active = self.manager()?.activate_connection(
            &connection,
            &path,
            &ObjectPath::from_static_str_unchecked(ROOT),
        )?;
        let active = ActiveConnectionProxyBlocking::builder(&self.bus)
            .path(active)?
            .build()?;
        let started = Instant::now();
        loop {
            match active.state() {
                Ok(ACTIVE_STATE_ACTIVATED) => return Ok(id),
                Ok(state) if state < ACTIVE_STATE_ACTIVATED => {}
                _ => {
                    return Err(Error::Refused(tr!(
                        "NetworkManager could not connect {interface}",
                        interface = plan.interface.clone()
                    )));
                }
            }
            if started.elapsed() > ACTIVATION_TIMEOUT {
                return Err(Error::Refused(tr!(
                    "NetworkManager did not connect {interface} in time",
                    interface = plan.interface.clone()
                )));
            }
            thread::sleep(MANAGED_WAIT);
        }
    }

    fn give_back(&self, taken: &Taken) -> Vec<String> {
        let mut leftovers = Vec::new();
        if let Some(connection) = &taken.connection
            && let Err(error) = self
                .settings_connection(connection)
                .and_then(|connection| Ok(connection.delete()?))
        {
            leftovers.push(tr!(
                "the new connection is still saved ({error})",
                error = error.to_string()
            ));
        }
        if let Some(device) = &taken.device
            && let Err(error) = self
                .device(device)
                .and_then(|device| Ok(device.set_managed(false)?))
        {
            leftovers.push(tr!(
                "NetworkManager still manages the interface ({error})",
                error = error.to_string()
            ));
        }
        leftovers
    }
}
