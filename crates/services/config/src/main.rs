use std::error::Error;
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};

use bande_config::{
    ConfigService, ConfigStore, ErrorNotifier, config_path, login_look, share_login_look,
};
use zbus::blocking::Connection;

fn open_in_editor(path: &Path) {
    if let Err(error) = bande_xdg::open(&path.to_string_lossy(), None) {
        tracing::warn!("cannot open {}: {error}", path.display());
    }
}

fn serve(path: &Path) -> Result<(ConfigService, Connection), Box<dyn Error>> {
    let connection = Connection::session()?;
    let notifier = ErrorNotifier::start(&connection, path, open_in_editor)?;
    let shared = Mutex::new(None);
    let service = ConfigService::start(
        &connection,
        path,
        Arc::new(ConfigStore::default()),
        move |result| {
            if let Ok(loaded) = &result {
                let look = login_look(&loaded.config, std::env::home_dir().as_deref());
                let mut shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
                if shared.as_ref() != Some(&look) {
                    *shared = Some(look.clone());
                    share_login_look(look);
                }
            }
            notifier.report(result.as_ref().map(|loaded| loaded.problems.as_slice()));
        },
    )?;
    Ok((service, connection))
}

fn main() -> ExitCode {
    bande_log::init("bande-config");
    let Some(path) = config_path() else {
        tracing::error!("neither XDG_CONFIG_HOME nor HOME is set");
        return ExitCode::FAILURE;
    };
    match serve(&path) {
        Ok((_service, connection)) => {
            bande_bus::serve_until_closed(&connection);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
