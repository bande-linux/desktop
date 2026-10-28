use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::thread;

use unbound_config::{ConfigService, ConfigStore, ErrorNotifier, config_path};
use zbus::blocking::Connection;

fn open_in_editor(path: &Path) {
    match Command::new("xdg-open").arg(path).spawn() {
        Ok(mut editor) => {
            thread::spawn(move || editor.wait());
        }
        Err(error) => eprintln!("unbound-config: cannot run xdg-open: {error}"),
    }
}

fn serve(path: &Path) -> Result<ConfigService, Box<dyn Error>> {
    let connection = Connection::session()?;
    let notifier = ErrorNotifier::start(&connection, path, open_in_editor)?;
    let service = ConfigService::start(
        &connection,
        path,
        Arc::new(ConfigStore::default()),
        move |result| notifier.report(&result),
    )?;
    Ok(service)
}

fn main() -> ExitCode {
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let Some(path) = config_path(config_home.as_deref(), std::env::home_dir().as_deref()) else {
        eprintln!("unbound-config: neither XDG_CONFIG_HOME nor HOME is set");
        return ExitCode::FAILURE;
    };
    match serve(&path) {
        Ok(_service) => loop {
            thread::park();
        },
        Err(error) => {
            eprintln!("unbound-config: {error}");
            ExitCode::FAILURE
        }
    }
}
