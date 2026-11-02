use std::collections::HashMap;
use std::fs::File;
use std::io::Write;

use zbus::blocking::{Connection, Proxy};
use zbus::message::Header;
use zbus::object_server::ObjectServer;
use zbus::zvariant::{ObjectPath, OwnedFd, OwnedObjectPath, OwnedValue, Value};
use zeroize::Zeroizing;

use crate::from_frontend;
use crate::request::{self, CANCELLED, Cancellation, OTHER, SUCCESS};

const SECRETS: &str = "org.freedesktop.secrets";
const SECRETS_PATH: &str = "/org/freedesktop/secrets";
const SERVICE: &str = "org.freedesktop.Secret.Service";
const PROMPT: &str = "org.freedesktop.Secret.Prompt";
const SCHEMA: &str = "org.freedesktop.impl.portal.Secret";
const APP_ID: &str = "app_id";
const KEY_LENGTH: usize = 64;

type WireSecret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

#[derive(Clone)]
pub struct Secret {
    pub session: Connection,
}

fn service<R>(
    connection: &Connection,
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> zbus::Result<R>
where
    R: for<'a> serde::Deserialize<'a> + zbus::zvariant::Type,
{
    connection
        .call_method(Some(SECRETS), SECRETS_PATH, Some(SERVICE), method, body)?
        .body()
        .deserialize()
}

fn prompt(
    connection: &Connection,
    path: &OwnedObjectPath,
    cancellation: &Cancellation,
) -> zbus::Result<bool> {
    if path.as_str() == "/" {
        return Ok(true);
    }
    let proxy: Proxy<'static> = Proxy::new(connection, SECRETS, path.clone(), PROMPT)?;
    let mut completed = proxy.receive_signal("Completed")?;
    let dismiss = proxy.clone();
    cancellation.on_cancel(move || {
        let _ = dismiss.call_method("Dismiss", &());
    });
    proxy.call_method("Prompt", &(""))?;
    let Some(signal) = completed.next() else {
        return Ok(false);
    };
    let (dismissed, _): (bool, OwnedValue) = signal.body().deserialize()?;
    Ok(!dismissed)
}

fn unlock(
    connection: &Connection,
    paths: Vec<OwnedObjectPath>,
    cancellation: &Cancellation,
) -> zbus::Result<bool> {
    if paths.is_empty() {
        return Ok(true);
    }
    let (_, waiting): (Vec<OwnedObjectPath>, OwnedObjectPath) =
        service(connection, "Unlock", &(paths,))?;
    prompt(connection, &waiting, cancellation)
}

fn random_key() -> std::io::Result<Zeroizing<Vec<u8>>> {
    let mut key = Zeroizing::new(vec![0u8; KEY_LENGTH]);
    let mut filled = 0;
    while filled < KEY_LENGTH {
        filled +=
            rustix::rand::getrandom(&mut key[filled..], rustix::rand::GetRandomFlags::empty())?;
    }
    Ok(key)
}

fn app_key(
    connection: &Connection,
    app_id: &str,
    cancellation: &Cancellation,
) -> zbus::Result<Option<Zeroizing<Vec<u8>>>> {
    let (_, session): (OwnedValue, OwnedObjectPath) =
        service(connection, "OpenSession", &("plain", Value::from("")))?;
    let attributes = HashMap::from([(APP_ID, app_id)]);
    let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
        service(connection, "SearchItems", &(&attributes,))?;
    let found = unlocked.into_iter().chain(locked.iter().cloned()).next();
    if let Some(item) = found {
        if !unlock(connection, locked, cancellation)? {
            return Ok(None);
        }
        let secrets: HashMap<OwnedObjectPath, WireSecret> =
            service(connection, "GetSecrets", &(vec![&item], &session))?;
        return Ok(secrets
            .get(&item)
            .map(|(_, _, value, _)| Zeroizing::new(value.clone())));
    }
    let (collection,): (OwnedObjectPath,) = service(connection, "ReadAlias", &("default",))?;
    if collection.as_str() == "/" || !unlock(connection, vec![collection.clone()], cancellation)? {
        return Ok(None);
    }
    let key = random_key().map_err(zbus::Error::from)?;
    let label = format!("Application key for {app_id}");
    let attributes = HashMap::from([("xdg:schema", SCHEMA), (APP_ID, app_id)]);
    let properties = HashMap::from([
        ("org.freedesktop.Secret.Item.Label", Value::from(label)),
        (
            "org.freedesktop.Secret.Item.Attributes",
            Value::from(attributes),
        ),
    ]);
    let secret = (
        session,
        Vec::<u8>::new(),
        key.to_vec(),
        "application/octet-stream",
    );
    connection.call_method(
        Some(SECRETS),
        &collection,
        Some("org.freedesktop.Secret.Collection"),
        "CreateItem",
        &(properties, secret, false),
    )?;
    Ok(Some(key))
}

impl Secret {
    fn retrieve(&self, app_id: &str, fd: OwnedFd, cancellation: &Cancellation) -> u32 {
        if app_id.is_empty() {
            return OTHER;
        }
        let key = match app_key(&self.session, app_id, cancellation) {
            Ok(Some(key)) => key,
            Ok(None) => return CANCELLED,
            Err(error) => {
                tracing::warn!("cannot get the keychain key for {app_id}: {error}");
                return OTHER;
            }
        };
        let mut file = File::from(std::os::fd::OwnedFd::from(fd));
        match file.write_all(&key) {
            Ok(()) => SUCCESS,
            Err(error) => {
                tracing::warn!("cannot hand {app_id} its keychain key: {error}");
                OTHER
            }
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Secret")]
impl Secret {
    #[allow(clippy::too_many_arguments)]
    async fn retrieve_secret(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        fd: OwnedFd,
        _options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        if !from_frontend(connection, &header).await {
            tracing::warn!("refused a secret request that did not come from xdg-desktop-portal");
            return (OTHER, HashMap::new());
        }
        let secret = self.clone();
        let outcome = request::run(server, &handle, move |cancellation| {
            secret.retrieve(&app_id, fd, cancellation)
        })
        .await;
        let response = match outcome {
            Ok(Some(response)) => response,
            Ok(None) => CANCELLED,
            Err(_) => OTHER,
        };
        (response, HashMap::new())
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}
