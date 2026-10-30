use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;

use xdg_desktop_portal_unbound::capture::{ScreenshotService, ScreenshotTool};
use xdg_desktop_portal_unbound::screencast::{ScreenCast, close_sessions_streaming};
use xdg_desktop_portal_unbound::screenshot::Screenshot;
use xdg_desktop_portal_unbound::settings::{CursorSettings, Settings};
use xdg_desktop_portal_unbound::{BUS_NAME, OBJECT_PATH};
use zbus::blocking::Connection;

fn close_sessions_when_sharing_ends(
    connection: &Connection,
    tool: &Connection,
) -> zbus::Result<()> {
    let ended = ScreenshotService::new(tool)?.receive_streams_ended()?;
    let connection = connection.clone();
    thread::spawn(move || {
        for signal in ended {
            let Ok(arguments) = signal.args() else {
                continue;
            };
            if let Err(error) = close_sessions_streaming(&connection, &arguments.nodes) {
                tracing::warn!("cannot close the sharing session: {error}");
            }
        }
    });
    Ok(())
}

fn serve() -> zbus::Result<Connection> {
    let connection = Connection::session()?;
    let tool_connection = Connection::session()?;
    let tool = Arc::new(ScreenshotTool::new(tool_connection.clone()));
    Settings::serve(
        &connection,
        CursorSettings::from_environment(),
        config_home(),
    )?;
    connection
        .object_server()
        .at(OBJECT_PATH, ScreenCast::new(tool.clone()))?;
    connection
        .object_server()
        .at(OBJECT_PATH, Screenshot { ui: tool })?;
    close_sessions_when_sharing_ends(&connection, &tool_connection)?;
    connection.request_name(BUS_NAME)?;
    Ok(connection)
}

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

fn main() -> ExitCode {
    unbound_log::init("xdg-desktop-portal-unbound");
    match serve() {
        Ok(connection) => {
            unbound_bus::serve_until_closed(&connection);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
