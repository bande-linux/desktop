use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};

use zbus::blocking::Connection;

use crate::CursorTheme;

pub const DESKTOP_NAME: &str = "Unbound";

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait SystemdManager {
    fn set_environment(&self, assignments: &[&str]) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.DBus",
    default_service = "org.freedesktop.DBus",
    default_path = "/org/freedesktop/DBus"
)]
trait Bus {
    fn update_activation_environment(&self, environment: HashMap<&str, &str>) -> zbus::Result<()>;
}

pub fn cursor_environment(cursor: &CursorTheme) -> [(&'static str, String); 2] {
    [
        ("XCURSOR_THEME", cursor.name.clone()),
        ("XCURSOR_SIZE", cursor.size.to_string()),
    ]
}

pub fn session_environment(socket_name: &str, cursor: &CursorTheme) -> Vec<(&'static str, String)> {
    [
        ("WAYLAND_DISPLAY", socket_name.to_owned()),
        ("XDG_CURRENT_DESKTOP", DESKTOP_NAME.to_owned()),
        ("XDG_SESSION_TYPE", "wayland".to_owned()),
    ]
    .into_iter()
    .chain(cursor_environment(cursor))
    .collect()
}

pub fn export_session_environment(
    session: &Connection,
    socket_name: &str,
    cursor: &CursorTheme,
) -> zbus::Result<()> {
    let environment = session_environment(socket_name, cursor);
    let assignments: Vec<String> = environment
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    let assignments: Vec<&str> = assignments.iter().map(String::as_str).collect();
    BusProxyBlocking::new(session)?.update_activation_environment(
        environment
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect(),
    )?;
    SystemdManagerProxyBlocking::new(session)?.set_environment(&assignments)
}

pub fn notify_ready(notify_socket: Option<&OsStr>) -> io::Result<()> {
    let Some(notify_socket) = notify_socket else {
        return Ok(());
    };
    let address = match notify_socket.as_bytes().strip_prefix(b"@") {
        Some(name) => SocketAddr::from_abstract_name(name)?,
        None => SocketAddr::from_pathname(notify_socket)?,
    };
    UnixDatagram::unbound()?.send_to_addr(b"READY=1", &address)?;
    Ok(())
}
