use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";

pub type Filter = (String, Vec<(u32, String)>);

fn request_path(connection: &Connection, token: &str) -> Option<OwnedObjectPath> {
    let sender = connection
        .unique_name()?
        .trim_start_matches(':')
        .replace('.', "_");
    OwnedObjectPath::try_from(format!("{PORTAL_PATH}/request/{sender}/{token}")).ok()
}

fn ask(
    session: &Connection,
    method: &str,
    title: &str,
    mut options: HashMap<&str, Value<'_>>,
) -> zbus::Result<Option<PathBuf>> {
    static REQUESTS: AtomicU32 = AtomicU32::new(0);
    let token = format!(
        "bande_app_{}_{}",
        std::process::id(),
        REQUESTS.fetch_add(1, Ordering::Relaxed)
    );
    let path = request_path(session, &token).ok_or(zbus::Error::Unsupported)?;
    let request = Proxy::new(session, PORTAL, path, REQUEST)?;
    let mut responses = request.receive_signal("Response")?;
    let chooser = Proxy::new(session, PORTAL, PORTAL_PATH, FILE_CHOOSER)?;
    options.insert("handle_token", Value::from(token.clone()));
    options.insert("modal", Value::from(true));
    let _: OwnedObjectPath = chooser.call(method, &("", title, options))?;
    let Some(response) = responses.next() else {
        return Ok(None);
    };
    let (code, results) = response
        .body()
        .deserialize::<(u32, HashMap<String, OwnedValue>)>()?;
    if code != 0 {
        return Ok(None);
    }
    let uris = results
        .get("uris")
        .and_then(|uris| Vec::<String>::try_from(uris.try_clone().ok()?).ok())
        .unwrap_or_default();
    Ok(uris
        .first()
        .and_then(|uri| bande_xdg::path_from_file_uri(uri)))
}

pub fn choose_file(
    session: &Connection,
    title: &str,
    filter: Option<Filter>,
) -> zbus::Result<Option<PathBuf>> {
    let mut options = HashMap::new();
    match filter {
        Some(filter) => {
            options.insert("filters", Value::from(vec![filter.clone()]));
            options.insert("current_filter", Value::from(filter));
        }
        None => {
            options.insert("directory", Value::from(true));
        }
    }
    ask(session, "OpenFile", title, options)
}

pub fn choose_save_file(
    session: &Connection,
    title: &str,
    name: &str,
) -> zbus::Result<Option<PathBuf>> {
    let options = HashMap::from([("current_name", Value::from(name))]);
    ask(session, "SaveFile", title, options)
}
