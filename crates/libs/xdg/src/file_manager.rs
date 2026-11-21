use std::path::Path;

use zbus::blocking::Connection;

use crate::file_uri;

/// The bus name and interface of the freedesktop.org file manager interface.
pub const FILE_MANAGER: &str = "org.freedesktop.FileManager1";
/// The object path of the freedesktop.org file manager interface.
pub const FILE_MANAGER_PATH: &str = "/org/freedesktop/FileManager1";

/// Shows each path selected in its folder in the file manager, through
/// `org.freedesktop.FileManager1.ShowItems` on the session bus. The activation token is passed
/// as the startup id, so the file manager's window comes to the front. Blocks until the file
/// manager answers, so call it off the UI thread.
pub fn show_items<P: AsRef<Path>>(paths: &[P], activation_token: Option<&str>) -> zbus::Result<()> {
    let uris: Vec<String> = paths.iter().map(|path| file_uri(path.as_ref())).collect();
    Connection::session()?
        .call_method(
            Some(FILE_MANAGER),
            FILE_MANAGER_PATH,
            Some(FILE_MANAGER),
            "ShowItems",
            &(uris, activation_token.unwrap_or_default()),
        )
        .map(drop)
}
