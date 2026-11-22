use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use bande_l10n::tr;
use zbus::blocking::Connection;

const FILE: &str = "bande/proxy";
const HTTP: [&str; 2] = ["http_proxy", "HTTP_PROXY"];
const HTTPS: [&str; 2] = ["https_proxy", "HTTPS_PROXY"];
const BYPASS: [&str; 2] = ["no_proxy", "NO_PROXY"];
const SCHEME: &str = "http://";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProxyMode {
    #[default]
    None,
    Manual,
}

impl ProxyMode {
    pub const ALL: [Self; 2] = [Self::None, Self::Manual];
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProxySettings {
    pub mode: ProxyMode,
    pub http: String,
    pub https: String,
    pub bypass: String,
}

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait SystemdManager {
    fn set_environment(&self, assignments: &[&str]) -> zbus::Result<()>;

    fn unset_environment(&self, names: &[&str]) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.DBus",
    default_service = "org.freedesktop.DBus",
    default_path = "/org/freedesktop/DBus"
)]
trait Bus {
    fn update_activation_environment(&self, environment: HashMap<&str, &str>) -> zbus::Result<()>;
}

fn path() -> Option<PathBuf> {
    bande_xdg::config_home().map(|config| config.join(FILE))
}

fn host(url: &str) -> String {
    url.strip_prefix(SCHEME)
        .unwrap_or(url)
        .trim_end_matches('/')
        .to_owned()
}

pub fn read_proxy() -> ProxySettings {
    let text = match path().map(fs::read_to_string) {
        Some(Ok(text)) => text,
        Some(Err(error)) if error.kind() != io::ErrorKind::NotFound => {
            tracing::warn!("cannot read the proxy settings: {error}");
            return ProxySettings::default();
        }
        _ => return ProxySettings::default(),
    };
    let values: HashMap<&str, &str> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    let http = values.get(HTTP[0]).copied().unwrap_or_default();
    let bypass = values
        .get(BYPASS[0])
        .copied()
        .unwrap_or_default()
        .split(',')
        .collect::<Vec<_>>()
        .join(", ");
    if http.is_empty() {
        return ProxySettings::default();
    }
    ProxySettings {
        mode: ProxyMode::Manual,
        http: host(http),
        https: values
            .get(HTTPS[0])
            .map(|url| host(url))
            .unwrap_or_default(),
        bypass,
    }
}

fn plain(value: &str) -> bool {
    !value.is_empty()
        && !value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "\"'\\$`#".contains(c))
}

fn proxy_url(server: &str) -> Option<String> {
    let server = host(server.trim());
    let (name, port) = server.rsplit_once(':')?;
    (plain(&server) && !name.is_empty() && port.parse::<u16>().is_ok_and(|port| port > 0))
        .then(|| format!("{SCHEME}{server}"))
}

fn assignments(settings: &ProxySettings) -> Result<Vec<(&'static str, String)>, String> {
    let mut assigned = Vec::new();
    match settings.mode {
        ProxyMode::None => {}
        ProxyMode::Manual => {
            let invalid =
                || tr!("Type the proxy as a name and port, such as proxy.example.org:3128.");
            let http = proxy_url(&settings.http).ok_or_else(invalid)?;
            let https = match settings.https.trim() {
                "" => http.clone(),
                https => proxy_url(https).ok_or_else(invalid)?,
            };
            let bypass: Vec<&str> = settings
                .bypass
                .split([',', ' '])
                .filter(|host| !host.is_empty())
                .collect();
            if !bypass.iter().all(|host| plain(host)) {
                return Err(tr!(
                    "Separate the names to reach directly with commas, such as localhost, .example.org."
                ));
            }
            for name in HTTP {
                assigned.push((name, http.clone()));
            }
            for name in HTTPS {
                assigned.push((name, https.clone()));
            }
            if !bypass.is_empty() {
                for name in BYPASS {
                    assigned.push((name, bypass.join(",")));
                }
            }
        }
    }
    Ok(assigned)
}

fn apply(session: &Connection, assigned: &[(&str, String)]) -> zbus::Result<()> {
    let names: Vec<&str> = HTTP.into_iter().chain(HTTPS).chain(BYPASS).collect();
    let manager = SystemdManagerProxyBlocking::new(session)?;
    manager.unset_environment(&names)?;
    let lines: Vec<String> = assigned
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    if !lines.is_empty() {
        manager.set_environment(&lines)?;
    }
    let mut activation: HashMap<&str, &str> = names.iter().map(|name| (*name, "")).collect();
    activation.extend(assigned.iter().map(|(name, value)| (*name, value.as_str())));
    BusProxyBlocking::new(session)?.update_activation_environment(activation)
}

pub fn write_proxy(session: &Connection, settings: &ProxySettings) -> Result<(), String> {
    let assigned = assignments(settings)?;
    let file = path().ok_or_else(|| tr!("This account has no configuration folder."))?;
    let written = if assigned.is_empty() {
        match fs::remove_file(&file) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    } else {
        let text: String = assigned
            .iter()
            .map(|(name, value)| format!("{name}={value}\n"))
            .collect();
        bande_xdg::replace_file(&file, text)
    };
    written.map_err(|error| error.to_string())?;
    apply(session, &assigned).map_err(|error| error.to_string())
}
