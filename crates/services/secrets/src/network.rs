use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_lite::StreamExt;
use futures_lite::future;
use oo7::Secret;
use zbus::message::Header;
use zbus::names::WellKnownName;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, interface};

use crate::service::Daemon;

pub const NETWORK_MANAGER: &str = "/usr/sbin/NetworkManager";
const BUS_NAME: &str = "org.freedesktop.NetworkManager";
const AGENT_PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
const AGENT_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager/AgentManager";
const AGENT_MANAGER: &str = "org.freedesktop.NetworkManager.AgentManager";
const IDENTIFIER: &str = "org.bande.Secrets";
const SCHEMA: &str = "org.freedesktop.NetworkManager.Connection";
const ALLOW_INTERACTION: u32 = 0x1;
const REQUEST_NEW: u32 = 0x2;
const AGENT_OWNED: u32 = 0x1;
const NOT_SAVED: u32 = 0x2;
const WIRELESS_SECURITY: &str = "802-11-wireless-security";
const ENTERPRISE: &str = "802-1x";
const VPN: &str = "vpn";
const SECRET_KEYS: &[(&str, &[&str])] = &[
    (
        WIRELESS_SECURITY,
        &[
            "psk",
            "wep-key0",
            "wep-key1",
            "wep-key2",
            "wep-key3",
            "leap-password",
        ],
    ),
    (
        ENTERPRISE,
        &[
            "password",
            "private-key-password",
            "phase2-private-key-password",
            "pin",
        ],
    ),
    ("gsm", &["password", "pin"]),
    ("cdma", &["password"]),
    ("pppoe", &["password"]),
];

type Settings = HashMap<String, HashMap<String, OwnedValue>>;
type Answer = HashMap<String, HashMap<String, OwnedValue>>;

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.NetworkManager.SecretAgent")]
pub enum Error {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotAuthorized(String),
    InvalidConnection(String),
    UserCanceled(String),
    AgentCanceled(String),
    NoSecrets(String),
}

fn text(settings: &Settings, setting: &str, key: &str) -> Option<String> {
    let value = settings.get(setting)?.get(key)?.try_clone().ok()?;
    String::try_from(value).ok()
}

fn number(settings: &Settings, setting: &str, key: &str) -> u32 {
    settings
        .get(setting)
        .and_then(|values| values.get(key))
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_default()
}

fn map(settings: &Settings, setting: &str, key: &str) -> HashMap<String, String> {
    settings
        .get(setting)
        .and_then(|values| values.get(key))
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| HashMap::<String, String>::try_from(value).ok())
        .unwrap_or_default()
}

fn saved(flags: u32) -> bool {
    flags & AGENT_OWNED != 0 && flags & NOT_SAVED == 0
}

fn attributes(uuid: &str, setting: &str, key: Option<&str>) -> HashMap<String, String> {
    let mut attributes = HashMap::from([
        ("xdg:schema".to_owned(), SCHEMA.to_owned()),
        ("connection-uuid".to_owned(), uuid.to_owned()),
        ("setting-name".to_owned(), setting.to_owned()),
    ]);
    if let Some(key) = key {
        attributes.insert("setting-key".to_owned(), key.to_owned());
    }
    attributes
}

fn asked_key(settings: &Settings, setting: &str) -> Option<&'static str> {
    match setting {
        WIRELESS_SECURITY => match text(settings, WIRELESS_SECURITY, "key-mgmt").as_deref() {
            Some("wpa-psk" | "sae") => Some("psk"),
            Some("none") => Some("wep-key0"),
            _ => None,
        },
        ENTERPRISE => Some("password"),
        _ => None,
    }
}

fn owned(value: Value<'_>) -> Option<OwnedValue> {
    value.try_into_owned().ok()
}

fn answer(setting: &str, secrets: HashMap<String, String>) -> Answer {
    let values = if setting == VPN {
        owned(Value::from(secrets))
            .map(|secrets| HashMap::from([("secrets".to_owned(), secrets)]))
            .unwrap_or_default()
    } else {
        secrets
            .into_iter()
            .filter_map(|(key, secret)| Some((key, owned(Value::from(secret))?)))
            .collect()
    };
    HashMap::from([(setting.to_owned(), values)])
}

type Waiting = HashMap<(OwnedObjectPath, String), async_channel::Sender<()>>;

pub struct Agent {
    daemon: Arc<Daemon>,
    waiting: Mutex<Waiting>,
}

impl Agent {
    fn waiting(&self) -> MutexGuard<'_, Waiting> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn from_network_manager(connection: &Connection, header: &Header<'_>) -> bool {
        let Ok(bus) = zbus::fdo::DBusProxy::new(connection).await else {
            return false;
        };
        let Ok(owner) = bus
            .get_name_owner(WellKnownName::from_static_str_unchecked(BUS_NAME).into())
            .await
        else {
            return false;
        };
        header.sender().map(|sender| sender.as_str()) == Some(owner.as_str())
    }

    async fn lookup(
        &self,
        settings: &Settings,
        setting: &str,
        flags: u32,
    ) -> Result<Answer, Error> {
        let uuid = text(settings, "connection", "uuid")
            .ok_or_else(|| Error::InvalidConnection("the connection has no uuid".to_owned()))?;
        let name = text(settings, "connection", "id").unwrap_or_else(|| uuid.clone());
        let interactive = flags & ALLOW_INTERACTION != 0;
        if flags & REQUEST_NEW == 0 {
            let stored = self
                .daemon
                .internal_items(&attributes(&uuid, setting, None), interactive)
                .await;
            let secrets = stored
                .into_iter()
                .filter_map(|(attributes, secret)| {
                    Some((
                        attributes.get("setting-key")?.clone(),
                        secret.as_str()?.to_owned(),
                    ))
                })
                .collect::<HashMap<_, _>>();
            if !secrets.is_empty() {
                return Ok(answer(setting, secrets));
            }
        }
        let key = asked_key(settings, setting)
            .ok_or_else(|| Error::NoSecrets("no saved secrets for this connection".to_owned()))?;
        if !interactive {
            return Err(Error::NoSecrets(
                "no saved secrets for this connection".to_owned(),
            ));
        }
        let password = self
            .daemon
            .ask_network(&name)
            .await
            .ok_or_else(|| Error::UserCanceled("the password was not given".to_owned()))?;
        let password = password
            .as_str()
            .ok_or_else(|| Error::UserCanceled("the password is not text".to_owned()))?;
        Ok(answer(
            setting,
            HashMap::from([(key.to_owned(), password.to_owned())]),
        ))
    }
}

#[interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl Agent {
    #[allow(clippy::too_many_arguments)]
    async fn get_secrets(
        &self,
        connection: Settings,
        connection_path: OwnedObjectPath,
        setting_name: String,
        _hints: Vec<String>,
        flags: u32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] bus: &Connection,
    ) -> Result<Answer, Error> {
        if !Self::from_network_manager(bus, &header).await {
            return Err(Error::NotAuthorized(
                "only NetworkManager may ask".to_owned(),
            ));
        }
        let (cancel, cancelled) = async_channel::bounded(1);
        self.waiting()
            .insert((connection_path.clone(), setting_name.clone()), cancel);
        let result = future::or(self.lookup(&connection, &setting_name, flags), async {
            let _ = cancelled.recv().await;
            Err(Error::AgentCanceled(
                "NetworkManager cancelled the request".to_owned(),
            ))
        })
        .await;
        self.waiting().remove(&(connection_path, setting_name));
        result
    }

    async fn cancel_get_secrets(
        &self,
        connection_path: OwnedObjectPath,
        setting_name: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] bus: &Connection,
    ) -> Result<(), Error> {
        if !Self::from_network_manager(bus, &header).await {
            return Err(Error::NotAuthorized(
                "only NetworkManager may cancel".to_owned(),
            ));
        }
        if let Some(cancel) = self.waiting().remove(&(connection_path, setting_name)) {
            let _ = cancel.try_send(());
        }
        Ok(())
    }

    async fn save_secrets(
        &self,
        connection: Settings,
        _connection_path: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] bus: &Connection,
    ) -> Result<(), Error> {
        if !Self::from_network_manager(bus, &header).await {
            return Err(Error::NotAuthorized(
                "only NetworkManager may save".to_owned(),
            ));
        }
        let uuid = text(&connection, "connection", "uuid")
            .ok_or_else(|| Error::InvalidConnection("the connection has no uuid".to_owned()))?;
        let name = text(&connection, "connection", "id").unwrap_or_else(|| uuid.clone());
        let mut secrets = Vec::new();
        for (setting, keys) in SECRET_KEYS {
            for key in *keys {
                if let Some(value) = text(&connection, setting, key)
                    && saved(number(&connection, setting, &format!("{key}-flags")))
                {
                    secrets.push((setting.to_string(), key.to_string(), value));
                }
            }
        }
        let data = map(&connection, VPN, "data");
        for (key, value) in map(&connection, VPN, "secrets") {
            let flags = data
                .get(&format!("{key}-flags"))
                .and_then(|flags| flags.parse().ok())
                .unwrap_or_default();
            if saved(flags) {
                secrets.push((VPN.to_owned(), key, value));
            }
        }
        for (setting, key, value) in secrets {
            let label = format!("Network secret for {name}/{setting}/{key}");
            self.daemon
                .store_internal(
                    &label,
                    attributes(&uuid, &setting, Some(&key)),
                    Secret::text(value),
                )
                .await;
        }
        Ok(())
    }

    async fn delete_secrets(
        &self,
        connection: Settings,
        _connection_path: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] bus: &Connection,
    ) -> Result<(), Error> {
        if !Self::from_network_manager(bus, &header).await {
            return Err(Error::NotAuthorized(
                "only NetworkManager may delete".to_owned(),
            ));
        }
        let uuid = text(&connection, "connection", "uuid")
            .ok_or_else(|| Error::InvalidConnection("the connection has no uuid".to_owned()))?;
        let wanted = HashMap::from([
            ("xdg:schema".to_owned(), SCHEMA.to_owned()),
            ("connection-uuid".to_owned(), uuid),
        ]);
        self.daemon.delete_internal(&wanted).await;
        Ok(())
    }
}

async fn register(system: &Connection) -> zbus::Result<()> {
    system
        .call_method(
            Some(BUS_NAME),
            AGENT_MANAGER_PATH,
            Some(AGENT_MANAGER),
            "RegisterWithCapabilities",
            &(IDENTIFIER, 0u32),
        )
        .await?;
    Ok(())
}

pub async fn serve(daemon: Arc<Daemon>, system: Connection) -> zbus::Result<()> {
    system
        .object_server()
        .at(
            ObjectPath::from_static_str_unchecked(AGENT_PATH),
            Agent {
                daemon,
                waiting: Mutex::new(HashMap::new()),
            },
        )
        .await?;
    let bus = zbus::fdo::DBusProxy::new(&system).await?;
    let mut appearances = bus
        .receive_name_owner_changed_with_args(&[(0, BUS_NAME)])
        .await?;
    if let Err(error) = register(&system).await {
        tracing::info!("NetworkManager does not take the keychain's secret agent yet: {error}");
    }
    while let Some(change) = appearances.next().await {
        let appeared = change
            .args()
            .is_ok_and(|arguments| arguments.new_owner().is_some());
        if appeared && let Err(error) = register(&system).await {
            tracing::warn!("cannot register the keychain's secret agent: {error}");
        }
    }
    Ok(())
}
