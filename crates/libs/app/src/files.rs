use std::path::PathBuf;

use bande_toolkit::Window;

const FILES_APP_ID: &str = "org.bande.Files";

/// Shows each path selected in its folder in Files (`org.freedesktop.FileManager1.ShowItems`),
/// on a thread of its own. With a window, Files comes to the front through an activation token
/// from it. Failures are logged.
pub fn show_in_files(window: Option<&mut Window>, paths: Vec<PathBuf>) {
    let token = window.and_then(|window| {
        window
            .activation_token(FILES_APP_ID)
            .map_err(|error| tracing::warn!("cannot get an activation token for Files: {error}"))
            .ok()
    });
    let spawned = std::thread::Builder::new()
        .name("show-in-files".to_owned())
        .spawn(move || {
            if let Err(error) = bande_xdg::show_items(&paths, token.as_deref()) {
                tracing::warn!("cannot show the items in Files: {error}");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot show the items in Files: {error}");
    }
}
