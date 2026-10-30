use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use unbound_config::{
    ConfigService, ConfigStore, DRM_CLASS, ErrorNotifier, config_path, connected_displays,
    migrate_file, share_login_picture,
};
use zbus::blocking::Connection;

fn open_in_editor(path: &Path) {
    match Command::new("xdg-open").arg(path).spawn() {
        Ok(mut editor) => {
            thread::spawn(move || editor.wait());
        }
        Err(error) => tracing::warn!("cannot run xdg-open: {error}"),
    }
}

fn serve(path: &Path) -> Result<(ConfigService, Connection), Box<dyn Error>> {
    match migrate_file(path, &connected_displays(Path::new(DRM_CLASS))) {
        Ok(true) => tracing::info!("updated {} to the current settings names", path.display()),
        Ok(false) => {}
        Err(error) => tracing::warn!("cannot update {}: {error}", path.display()),
    }
    let connection = Connection::session()?;
    let notifier = ErrorNotifier::start(&connection, path, open_in_editor)?;
    let shared = Mutex::new(None);
    let service = ConfigService::start(
        &connection,
        path,
        Arc::new(ConfigStore::default()),
        move |result| {
            if let Ok(loaded) = &result {
                let config = &loaded.config;
                let mut shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
                if shared.as_ref() != Some(&config.wallpaper) {
                    share_login_picture(&config.wallpaper);
                    *shared = Some(config.wallpaper.clone());
                }
            }
            notifier.report(result.as_ref().map(|loaded| loaded.problems.as_slice()));
        },
    )?;
    Ok((service, connection))
}

fn main() -> ExitCode {
    unbound_log::init("unbound-config");
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let Some(path) = config_path(config_home.as_deref(), std::env::home_dir().as_deref()) else {
        tracing::error!("neither XDG_CONFIG_HOME nor HOME is set");
        return ExitCode::FAILURE;
    };
    match serve(&path) {
        Ok((_service, connection)) => {
            unbound_bus::serve_until_closed(&connection);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
