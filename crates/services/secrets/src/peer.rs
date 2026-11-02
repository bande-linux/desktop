use std::path::Path;

use zbus::fdo::DBusProxy;
use zbus::names::BusName;

const FLATPAK: &str = "flatpak:";
const DELETED: &str = " (deleted)";

pub async fn identity(connection: &zbus::Connection, sender: &str) -> Option<String> {
    let bus = DBusProxy::new(connection).await.ok()?;
    let name = BusName::try_from(sender).ok()?;
    let credentials = bus.get_connection_credentials(name).await.ok()?;
    of_process(credentials.process_id()?)
}

pub fn of_process(pid: u32) -> Option<String> {
    let process = Path::new("/proc").join(pid.to_string());
    if let Ok(info) = std::fs::read_to_string(process.join("root/.flatpak-info")) {
        return flatpak_app(&info).map(|app| format!("{FLATPAK}{app}"));
    }
    let executable = std::fs::read_link(process.join("exe")).ok()?;
    let executable = executable.to_str()?;
    Some(
        executable
            .strip_suffix(DELETED)
            .unwrap_or(executable)
            .to_owned(),
    )
}

fn flatpak_app(info: &str) -> Option<&str> {
    let mut in_application = false;
    for line in info.lines().map(str::trim) {
        if line.starts_with('[') {
            in_application = line == "[Application]";
        } else if in_application && let Some(name) = line.strip_prefix("name=") {
            return Some(name.trim());
        }
    }
    None
}

pub fn flatpak_id(identity: &str) -> Option<&str> {
    identity.strip_prefix(FLATPAK)
}
