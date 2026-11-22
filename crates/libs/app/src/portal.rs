use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";

pub type Filter = (String, Vec<(u32, String)>);

static FOCUSED_WINDOW: Mutex<Option<String>> = Mutex::new(None);

pub(crate) fn remember_focused_window(handle: Option<&str>) {
    *FOCUSED_WINDOW
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = handle.map(str::to_owned);
}

pub fn parent_window() -> String {
    FOCUSED_WINDOW
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map_or_else(String::new, |handle| format!("wayland:{handle}"))
}

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
) -> zbus::Result<Vec<PathBuf>> {
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
    let _: OwnedObjectPath = chooser.call(method, &(parent_window(), title, options))?;
    let Some(response) = responses.next() else {
        return Ok(Vec::new());
    };
    let (code, results) = response
        .body()
        .deserialize::<(u32, HashMap<String, OwnedValue>)>()?;
    if code != 0 {
        return Ok(Vec::new());
    }
    let uris = results
        .get("uris")
        .and_then(|uris| Vec::<String>::try_from(uris.try_clone().ok()?).ok())
        .unwrap_or_default();
    Ok(uris
        .iter()
        .filter_map(|uri| bande_xdg::path_from_file_uri(uri))
        .collect())
}

fn filters(options: &mut HashMap<&str, Value<'_>>, filter: Filter) {
    options.insert("filters", Value::from(vec![filter.clone()]));
    options.insert("current_filter", Value::from(filter));
}

pub fn choose_file(
    session: &Connection,
    title: &str,
    filter: Option<Filter>,
) -> zbus::Result<Option<PathBuf>> {
    let mut options = HashMap::new();
    match filter {
        Some(filter) => filters(&mut options, filter),
        None => {
            options.insert("directory", Value::from(true));
        }
    }
    Ok(ask(session, "OpenFile", title, options)?.into_iter().next())
}

pub fn choose_files(
    session: &Connection,
    title: &str,
    filter: Option<Filter>,
) -> zbus::Result<Vec<PathBuf>> {
    let mut options = HashMap::from([("multiple", Value::from(true))]);
    if let Some(filter) = filter {
        filters(&mut options, filter);
    }
    ask(session, "OpenFile", title, options)
}

pub fn choose_save_file(
    session: &Connection,
    title: &str,
    name: &str,
) -> zbus::Result<Option<PathBuf>> {
    let options = HashMap::from([("current_name", Value::from(name))]);
    Ok(ask(session, "SaveFile", title, options)?.into_iter().next())
}
