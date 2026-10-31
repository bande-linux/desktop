use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;

use unbound_xdg::{config_home, data_home};
use xdg_desktop_portal_unbound::capture::{ScreenshotService, ScreenshotTool};
use xdg_desktop_portal_unbound::dialog_process::{DIALOG_PROGRAM, DialogProcess};
use xdg_desktop_portal_unbound::file_chooser::FileChooser;
use xdg_desktop_portal_unbound::screencast::{ScreenCast, close_sessions_streaming};
use xdg_desktop_portal_unbound::screenshot::Screenshot;
use xdg_desktop_portal_unbound::settings::{CursorSettings, Settings};
use xdg_desktop_portal_unbound::wallpaper::{BACKGROUNDS, Wallpaper};
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
    let system = Connection::system()
        .map_err(|error| tracing::warn!("cannot reach the system bus: {error}"))
        .ok();
    Settings::serve(
        &connection,
        system.as_ref(),
        CursorSettings::from_environment(),
        config_home(),
    )?;
    connection
        .object_server()
        .at(OBJECT_PATH, ScreenCast::new(tool.clone()))?;
    connection
        .object_server()
        .at(OBJECT_PATH, Screenshot { ui: tool })?;
    let dialogs = Arc::new(DialogProcess {
        program: PathBuf::from(DIALOG_PROGRAM),
    });
    connection
        .object_server()
        .at(OBJECT_PATH, FileChooser::new(dialogs.clone()))?;
    if let Some(data_home) = data_home() {
        connection.object_server().at(
            OBJECT_PATH,
            Wallpaper {
                ui: dialogs,
                folder: data_home.join(BACKGROUNDS),
                session: connection.clone(),
            },
        )?;
    }
    close_sessions_when_sharing_ends(&connection, &tool_connection)?;
    connection.request_name(BUS_NAME)?;
    Ok(connection)
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
