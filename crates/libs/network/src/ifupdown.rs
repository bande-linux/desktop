use std::fmt;
use std::fs;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use crate::{Ipv6, ManualIpv4, ManualIpv6};

pub const INTERFACES: &str = "/etc/network/interfaces";
pub const INTERFACES_FOLDER: &str = "/etc/network/interfaces.d";
const NETWORK_FOLDER: &str = "/etc/network";
const IFUPDOWN_STATE: &str = "/run/network/ifstate";
const NET_CLASS: &str = "/sys/class/net";
const SECRET_OPTIONS: [&str; 2] = ["wpa-psk", "wpa-password"];
const HIDDEN_SECRET: &str = "••••••••";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub path: PathBuf,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Configuration {
    pub files: Vec<Source>,
    pub elsewhere: Vec<String>,
    pub supplicant: Vec<Source>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NotConfigured,
    Continuation(PathBuf),
    Mapping(PathBuf),
    Elsewhere(String),
    Shared(PathBuf, String),
    Repeated(String),
    Method(String),
    Option(String),
    Value(String),
    NoNetwork,
    Service(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "/etc/network/interfaces has no stanza for it"),
            Self::Continuation(path) => {
                write!(f, "{} continues lines with a backslash", path.display())
            }
            Self::Mapping(path) => {
                write!(
                    f,
                    "{} maps interfaces with a mapping stanza",
                    path.display()
                )
            }
            Self::Elsewhere(source) => write!(
                f,
                "the configuration reads “{source}” from outside /etc/network/interfaces.d"
            ),
            Self::Shared(path, line) => {
                write!(f, "another stanza in {} uses it: “{line}”", path.display())
            }
            Self::Repeated(what) => write!(f, "it sets “{what}” more than once"),
            Self::Method(method) => {
                write!(
                    f,
                    "NetworkManager has no equivalent of the “{method}” method"
                )
            }
            Self::Option(option) => {
                write!(
                    f,
                    "NetworkManager has no equivalent of the “{option}” option"
                )
            }
            Self::Value(option) => write!(f, "the value of “{option}” is not understood"),
            Self::NoNetwork => write!(f, "its stanza names no Wi-Fi network"),
            Self::Service(service) => {
                write!(
                    f,
                    "{service} also sets up network interfaces on this computer"
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ipv4Setup {
    Automatic,
    Manual(ManualIpv4),
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiSetup {
    pub ssid: Vec<u8>,
    pub psk: Option<String>,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub ipv4: Ipv4Setup,
    pub ipv6: Ipv6,
    pub ipv6_dns: Vec<Ipv6Addr>,
    pub dns_search: Vec<String>,
    pub dhcp_hostname: Option<String>,
    pub mtu: Option<u32>,
    pub hardware_address: Option<String>,
    pub wifi: Option<WifiSetup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    Keep(String),
    Disable(String),
    Rewrite { original: String, kept: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub path: PathBuf,
    pub original: String,
    lines: Vec<Line>,
}

impl Edit {
    pub fn disabled(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter_map(|line| match line {
                Line::Keep(_) => None,
                Line::Disable(original) | Line::Rewrite { original, .. } => Some(shown(original)),
            })
            .collect()
    }

    pub fn edited(&self, note: &str) -> String {
        let mut out = String::with_capacity(self.original.len() + note.len() + 64);
        let mut noted = false;
        for line in &self.lines {
            let mut note_once = |out: &mut String| {
                if !std::mem::replace(&mut noted, true) {
                    out.push_str("# ");
                    out.push_str(note);
                    out.push('\n');
                }
            };
            match line {
                Line::Keep(text) => out.push_str(text),
                Line::Disable(text) => {
                    note_once(&mut out);
                    out.push('#');
                    out.push_str(text);
                }
                Line::Rewrite { original, kept } => {
                    note_once(&mut out);
                    out.push('#');
                    out.push_str(original);
                    out.push('\n');
                    out.push_str(kept);
                }
            }
            out.push('\n');
        }
        if !self.original.ends_with('\n') {
            out.pop();
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub interface: String,
    pub edits: Vec<Edit>,
    pub setup: Setup,
}

fn shown(line: &str) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.first() {
        Some(option) if SECRET_OPTIONS.contains(option) => format!("{option} {HIDDEN_SECRET}"),
        _ => line.trim().to_owned(),
    }
}

fn line_indent(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

fn run_parts_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn folder_files(folder: &Path, wanted: impl Fn(&str) -> bool) -> io::Result<Vec<Source>> {
    let mut paths: Vec<PathBuf> = match fs::read_dir(folder) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(&wanted)
            })
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            Ok(Source {
                text: fs::read_to_string(&path)?,
                path,
            })
        })
        .collect()
}

struct Layout {
    root: PathBuf,
    network: PathBuf,
    folder: PathBuf,
}

impl Layout {
    fn under(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            network: root.join(NETWORK_FOLDER.trim_start_matches('/')),
            folder: root.join(INTERFACES_FOLDER.trim_start_matches('/')),
        }
    }

    fn resolve(&self, argument: &str) -> PathBuf {
        match argument.strip_prefix('/') {
            Some(absolute) => self.root.join(absolute),
            None => self.network.join(argument),
        }
    }

    fn sourced(&self, argument: &str) -> Option<(bool, PathBuf)> {
        let path = self.resolve(argument);
        if path == self.folder.join("*") {
            return Some((true, self.folder.clone()));
        }
        (path.parent() == Some(self.folder.as_path()) && !argument.contains(['*', '?', '[']))
            .then_some((false, path))
    }
}

pub fn read() -> io::Result<Configuration> {
    read_under(Path::new("/"))
}

fn read_under(root: &Path) -> io::Result<Configuration> {
    let layout = Layout::under(root);
    let main = layout.resolve(INTERFACES);
    let text = match fs::read_to_string(&main) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Configuration::default());
        }
        Err(error) => return Err(error),
    };
    let mut configuration = Configuration {
        files: vec![Source {
            path: main,
            text: text.clone(),
        }],
        ..Configuration::default()
    };
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["source", argument] => match layout.sourced(argument) {
                Some((true, folder)) => configuration
                    .files
                    .extend(folder_files(&folder, |name| !name.starts_with('.'))?),
                Some((false, file)) => configuration.files.push(Source {
                    text: fs::read_to_string(&file)?,
                    path: file,
                }),
                None => configuration.elsewhere.push(line.trim().to_owned()),
            },
            ["source-directory" | "source-dir", folder]
                if layout.resolve(folder) == layout.folder =>
            {
                configuration
                    .files
                    .extend(folder_files(&layout.folder, run_parts_name)?);
            }
            ["source" | "source-directory" | "source-dir", ..] => {
                configuration.elsewhere.push(line.trim().to_owned());
            }
            _ => {}
        }
    }
    let mut supplicant = Vec::new();
    for (index, file) in configuration.files.iter().enumerate() {
        for line in file.text.lines() {
            match line.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["source" | "source-directory" | "source-dir", ..] if index > 0 => {
                    configuration.elsewhere.push(line.trim().to_owned());
                }
                ["wpa-conf", argument] if argument.starts_with('/') => {
                    match fs::read_to_string(layout.resolve(argument)) {
                        Ok(text) => supplicant.push(Source {
                            path: PathBuf::from(argument),
                            text,
                        }),
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                }
                _ => {}
            }
        }
    }
    configuration.supplicant = supplicant;
    Ok(configuration)
}

fn brought_up(state: &str, interface: &str) -> bool {
    state.lines().any(|line| {
        line.split_once('=')
            .is_some_and(|(name, _)| name == interface)
    })
}

/// Whether Settings can hand `interface` to NetworkManager: its stanza is one the helper carries
/// over, or, while the files are readable only by root, ifupdown brought it up.
pub fn can_hand_over(interface: &str) -> bool {
    let wireless = Path::new(NET_CLASS)
        .join(interface)
        .join("wireless")
        .exists();
    match read() {
        Ok(configuration) => plan(&configuration, interface, wireless).is_ok(),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            fs::read_to_string(IFUPDOWN_STATE).is_ok_and(|state| brought_up(&state, interface))
        }
        Err(error) => {
            tracing::debug!("cannot read the interfaces files: {error}");
            false
        }
    }
}

fn mentions(words: &[&str], interface: &str) -> bool {
    words.iter().any(|word| {
        *word == interface
            || word
                .strip_prefix(interface)
                .is_some_and(|rest| rest.starts_with(['.', ':']))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Inet,
    Inet6,
}

#[derive(Debug, Default)]
struct Collected {
    inet: Option<String>,
    inet6: Option<String>,
    address: Option<String>,
    netmask: Option<String>,
    gateway: Option<String>,
    address6: Option<String>,
    netmask6: Option<String>,
    gateway6: Option<String>,
    dns: Vec<IpAddr>,
    dns_search: Vec<String>,
    hostname: Option<String>,
    mtu: Option<u32>,
    hardware_address: Option<String>,
    ssid: Option<Vec<u8>>,
    psk: Option<String>,
    open: bool,
    hidden: bool,
    supplicant: Option<PathBuf>,
}

const STANZAS: [&str; 7] = [
    "iface",
    "mapping",
    "auto",
    "source",
    "source-directory",
    "source-dir",
    "rename",
];

fn starts_stanza(word: &str) -> bool {
    STANZAS.contains(&word)
        || word.starts_with("allow-")
        || word == "no-auto-down"
        || word == "no-scripts"
}

fn unquoted(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}

fn valid_psk(psk: &str) -> bool {
    (psk.len() == 64 && psk.chars().all(|c| c.is_ascii_hexdigit()))
        || ((8..=63).contains(&psk.len()) && psk.chars().all(|c| (' '..='~').contains(&c)))
}

impl Collected {
    fn option(&mut self, family: Family, method: &str, line: &str) -> Result<(), Refusal> {
        let trimmed = line.trim();
        let (option, value) = trimmed
            .split_once(char::is_whitespace)
            .map_or((trimmed, ""), |(option, value)| (option, value.trim()));
        let refuse = || Err(Refusal::Option(option.to_owned()));
        let bad = || Refusal::Value(option.to_owned());
        match (option, family, method) {
            ("address", Family::Inet, "static") => self.address = Some(value.to_owned()),
            ("netmask", Family::Inet, "static") => self.netmask = Some(value.to_owned()),
            ("gateway", Family::Inet, "static") => self.gateway = Some(value.to_owned()),
            ("broadcast" | "network", Family::Inet, "static") => {}
            ("address", Family::Inet6, "static") => self.address6 = Some(value.to_owned()),
            ("netmask", Family::Inet6, "static") => self.netmask6 = Some(value.to_owned()),
            ("gateway", Family::Inet6, "static") => self.gateway6 = Some(value.to_owned()),
            ("hostname", Family::Inet, "dhcp") => self.hostname = Some(value.to_owned()),
            ("dns-nameservers", ..) => {
                for server in value.split_whitespace() {
                    self.dns.push(server.parse().map_err(|_| bad())?);
                }
            }
            ("dns-search", ..) => self
                .dns_search
                .extend(value.split_whitespace().map(str::to_owned)),
            ("mtu", ..) => self.mtu = Some(value.parse().map_err(|_| bad())?),
            ("hwaddress", ..) => {
                let address = value.split_whitespace().last().unwrap_or_default();
                let valid = address.split(':').count() == 6
                    && address
                        .split(':')
                        .all(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_hexdigit()));
                if !valid {
                    return Err(bad());
                }
                self.hardware_address = Some(address.to_owned());
            }
            ("wpa-ssid", ..) => {
                let ssid = unquoted(value).as_bytes().to_vec();
                if ssid.is_empty() || ssid.len() > 32 {
                    return Err(bad());
                }
                self.ssid = Some(ssid);
            }
            ("wpa-psk", ..) => {
                let psk = unquoted(value);
                if !valid_psk(psk) {
                    return Err(bad());
                }
                self.psk = Some(psk.to_owned());
            }
            ("wpa-key-mgmt", ..) => match value {
                "NONE" => self.open = true,
                "WPA-PSK" => {}
                _ => return Err(bad()),
            },
            ("wpa-scan-ssid", ..) => match value {
                "1" => self.hidden = true,
                "0" => {}
                _ => return Err(bad()),
            },
            ("wpa-conf", ..) => self.supplicant = Some(PathBuf::from(value)),
            ("wpa-driver" | "wpa-proto" | "wpa-pairwise" | "wpa-group", ..) => {}
            ("wpa-ap-scan", ..) if value == "1" => {}
            _ => return refuse(),
        }
        Ok(())
    }

    fn ipv4(&self) -> Result<Ipv4Setup, Refusal> {
        match self.inet.as_deref() {
            None => Ok(Ipv4Setup::Off),
            Some("dhcp") => Ok(Ipv4Setup::Automatic),
            Some("static") => {
                let address = self
                    .address
                    .as_deref()
                    .ok_or_else(|| Refusal::Value("address".to_owned()))?;
                let (address, prefix) = match (address.split_once('/'), self.netmask.as_deref()) {
                    (Some((address, prefix)), _) => (address, prefix.parse().ok()),
                    (None, Some(mask)) => (address, prefix_of(mask)),
                    (None, None) => (address, Some(24)),
                };
                let ipv4 = |text: &str, option: &str| -> Result<Ipv4Addr, Refusal> {
                    text.parse().map_err(|_| Refusal::Value(option.to_owned()))
                };
                Ok(Ipv4Setup::Manual(ManualIpv4 {
                    address: ipv4(address, "address")?,
                    prefix: prefix
                        .filter(|prefix| (1..=32).contains(prefix))
                        .ok_or_else(|| Refusal::Value("netmask".to_owned()))?,
                    router: self
                        .gateway
                        .as_deref()
                        .map(|gateway| ipv4(gateway, "gateway"))
                        .transpose()?,
                    dns: self
                        .dns
                        .iter()
                        .filter_map(|server| match server {
                            IpAddr::V4(server) => Some(*server),
                            IpAddr::V6(_) => None,
                        })
                        .collect(),
                }))
            }
            Some(method) => Err(Refusal::Method(method.to_owned())),
        }
    }

    fn ipv6(&self) -> Result<Ipv6, Refusal> {
        match self.inet6.as_deref() {
            None | Some("auto" | "dhcp") => Ok(Ipv6::Automatic),
            Some("static") => {
                let address = self
                    .address6
                    .as_deref()
                    .ok_or_else(|| Refusal::Value("address".to_owned()))?;
                let (address, prefix) = match address.split_once('/') {
                    Some((address, prefix)) => (address, prefix.to_owned()),
                    None => (
                        address,
                        self.netmask6.clone().unwrap_or_else(|| "64".to_owned()),
                    ),
                };
                Ok(Ipv6::Manual(ManualIpv6 {
                    address: address
                        .parse()
                        .map_err(|_| Refusal::Value("address".to_owned()))?,
                    prefix: prefix
                        .parse()
                        .ok()
                        .filter(|prefix| (1..=128).contains(prefix))
                        .ok_or_else(|| Refusal::Value("netmask".to_owned()))?,
                    router: self
                        .gateway6
                        .as_deref()
                        .map(|gateway| {
                            gateway
                                .parse()
                                .map_err(|_| Refusal::Value("gateway".to_owned()))
                        })
                        .transpose()?,
                }))
            }
            Some(method) => Err(Refusal::Method(method.to_owned())),
        }
    }

    fn setup(self, wireless: bool) -> Result<Setup, Refusal> {
        let wifi = match (wireless, self.ssid.clone()) {
            (true, None) => return Err(Refusal::NoNetwork),
            (false, Some(_)) => return Err(Refusal::Option("wpa-ssid".to_owned())),
            (false, None) => None,
            (true, Some(ssid)) => {
                if !self.open && self.psk.is_none() {
                    return Err(Refusal::Value("wpa-psk".to_owned()));
                }
                Some(WifiSetup {
                    ssid,
                    psk: self.psk.clone().filter(|_| !self.open),
                    hidden: self.hidden,
                })
            }
        };
        Ok(Setup {
            ipv4: self.ipv4()?,
            ipv6: self.ipv6()?,
            ipv6_dns: self
                .dns
                .iter()
                .filter_map(|server| match server {
                    IpAddr::V6(server) => Some(*server),
                    IpAddr::V4(_) => None,
                })
                .collect(),
            dns_search: self.dns_search,
            dhcp_hostname: self.hostname,
            mtu: self.mtu,
            hardware_address: self.hardware_address,
            wifi,
        })
    }
}

fn supplicant_ssid(value: &str) -> Option<Vec<u8>> {
    if let Some(quoted) = value.strip_prefix('"') {
        return Some(quoted.strip_suffix('"')?.as_bytes().to_vec());
    }
    if !value.len().is_multiple_of(2) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(value.get(at..at + 2)?, 16).ok())
        .collect()
}

impl Collected {
    fn supplicant(&mut self, text: &str) -> Result<(), Refusal> {
        let bad = |key: &str| Refusal::Value(key.to_owned());
        let (mut found, mut inside) = (false, false);
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if !inside {
                if line.replace(' ', "") == "network={" {
                    if std::mem::replace(&mut found, true) {
                        return Err(bad("wpa-conf"));
                    }
                    inside = true;
                }
                continue;
            }
            if line == "}" {
                inside = false;
                continue;
            }
            let (key, value) = line.split_once('=').ok_or_else(|| bad("wpa-conf"))?;
            let (key, value) = (key.trim(), value.trim());
            match key {
                "ssid" => {
                    let ssid = supplicant_ssid(value)
                        .filter(|ssid| (1..=32).contains(&ssid.len()))
                        .ok_or_else(|| bad("ssid"))?;
                    if self.ssid.replace(ssid).is_some() {
                        return Err(Refusal::Repeated("ssid".to_owned()));
                    }
                }
                "psk" => {
                    let psk = unquoted(value);
                    if !valid_psk(psk) || (psk == value && psk.len() != 64) {
                        return Err(bad("psk"));
                    }
                    self.psk = Some(psk.to_owned());
                }
                "key_mgmt" => match value {
                    "NONE" => self.open = true,
                    "WPA-PSK" | "WPA-PSK WPA-PSK-SHA256" | "WPA-PSK-SHA256 WPA-PSK" => {}
                    _ => return Err(bad("key_mgmt")),
                },
                "scan_ssid" => match value {
                    "1" => self.hidden = true,
                    "0" => {}
                    _ => return Err(bad("scan_ssid")),
                },
                "priority" | "id_str" | "proto" | "pairwise" | "group" => {}
                "auth_alg" if value == "OPEN" => {}
                "disabled" if value == "0" => {}
                _ => return Err(Refusal::Option(key.to_owned())),
            }
        }
        if !found || inside {
            return Err(bad("wpa-conf"));
        }
        Ok(())
    }
}

fn prefix_of(mask: &str) -> Option<u32> {
    if let Ok(prefix) = mask.parse::<u32>() {
        return Some(prefix);
    }
    let bits = u32::from(mask.parse::<Ipv4Addr>().ok()?);
    (bits.leading_ones() + bits.trailing_zeros() == 32).then(|| bits.leading_ones())
}

pub fn plan(
    configuration: &Configuration,
    interface: &str,
    wireless: bool,
) -> Result<Plan, Refusal> {
    if let Some(source) = configuration.elsewhere.first() {
        return Err(Refusal::Elsewhere(source.clone()));
    }
    let mut collected = Collected::default();
    let mut edits = Vec::new();
    for file in &configuration.files {
        let mut lines = Vec::new();
        let mut ours: Option<(Family, String)> = None;
        let mut changed = false;
        for raw in file.text.lines() {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                lines.push(Line::Keep(raw.to_owned()));
                continue;
            }
            if trimmed.ends_with('\\') {
                return Err(Refusal::Continuation(file.path.clone()));
            }
            let words: Vec<&str> = trimmed.split_whitespace().collect();
            let shared = || Refusal::Shared(file.path.clone(), trimmed.to_owned());
            let first = words[0];
            if !starts_stanza(first) {
                match &ours {
                    Some((family, method)) => {
                        collected.option(*family, method, raw)?;
                        lines.push(Line::Disable(raw.to_owned()));
                        changed = true;
                    }
                    None if mentions(&words, interface) => return Err(shared()),
                    None => lines.push(Line::Keep(raw.to_owned())),
                }
                continue;
            }
            ours = None;
            match first {
                "mapping" => return Err(Refusal::Mapping(file.path.clone())),
                "iface" if words.get(1) == Some(&interface) => {
                    let [_, _, family, method] = words.as_slice() else {
                        return Err(Refusal::Method(words[2..].join(" ")));
                    };
                    let (family, slot) = match *family {
                        "inet" => (Family::Inet, &mut collected.inet),
                        "inet6" => (Family::Inet6, &mut collected.inet6),
                        other => return Err(Refusal::Method(other.to_owned())),
                    };
                    if slot.replace((*method).to_owned()).is_some() {
                        return Err(Refusal::Repeated(words[2].to_owned()));
                    }
                    ours = Some((family, (*method).to_owned()));
                    lines.push(Line::Disable(raw.to_owned()));
                    changed = true;
                }
                "iface" | "rename" | "source" | "source-directory" | "source-dir" => {
                    if mentions(&words[1..], interface) {
                        return Err(shared());
                    }
                    lines.push(Line::Keep(raw.to_owned()));
                }
                _ => {
                    let names = &words[1..];
                    let others: Vec<&str> = names
                        .iter()
                        .copied()
                        .filter(|name| *name != interface)
                        .collect();
                    if others.len() == names.len() {
                        if mentions(names, interface) {
                            return Err(shared());
                        }
                        lines.push(Line::Keep(raw.to_owned()));
                    } else if others.is_empty() {
                        lines.push(Line::Disable(raw.to_owned()));
                        changed = true;
                    } else {
                        if mentions(&others, interface) {
                            return Err(shared());
                        }
                        lines.push(Line::Rewrite {
                            original: raw.to_owned(),
                            kept: format!("{}{first} {}", line_indent(raw), others.join(" ")),
                        });
                        changed = true;
                    }
                }
            }
        }
        if changed {
            edits.push(Edit {
                path: file.path.clone(),
                original: file.text.clone(),
                lines,
            });
        }
    }
    if collected.inet.is_none() && collected.inet6.is_none() {
        return Err(Refusal::NotConfigured);
    }
    if let Some(path) = collected.supplicant.clone() {
        let file = configuration
            .supplicant
            .iter()
            .find(|file| file.path == path)
            .ok_or_else(|| Refusal::Value("wpa-conf".to_owned()))?;
        collected.supplicant(&file.text)?;
    }
    Ok(Plan {
        interface: interface.to_owned(),
        edits,
        setup: collected.setup(wireless)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configuration(files: &[(&str, &str)]) -> Configuration {
        Configuration {
            files: files
                .iter()
                .map(|(path, text)| Source {
                    path: PathBuf::from(path),
                    text: (*text).to_owned(),
                })
                .collect(),
            ..Configuration::default()
        }
    }

    const INSTALLER: &str = "\
# This file describes the network interfaces available on your system
source /etc/network/interfaces.d/*

# The loopback network interface
auto lo wlan0
iface lo inet loopback

# The primary network interface
allow-hotplug wlan0
iface wlan0 inet dhcp
\twpa-ssid \"Home Net\"
\twpa-psk  correct horse battery
iface wlan0 inet6 static
    address 2001:db8::20/64
    gateway 2001:db8::1
    dns-nameservers 2001:db8::53 192.0.2.53

iface eth1 inet static
    address 192.168.7.2
    netmask 255.255.255.0
";

    #[test]
    fn hands_over_only_the_interfaces_own_lines() {
        let plan = plan(&configuration(&[(INTERFACES, INSTALLER)]), "wlan0", true)
            .expect("the installer's stanza is understood");
        let setup = &plan.setup;
        assert_eq!(setup.ipv4, Ipv4Setup::Automatic);
        assert_eq!(
            setup.ipv6,
            Ipv6::Manual(ManualIpv6 {
                address: "2001:db8::20".parse().expect("address"),
                prefix: 64,
                router: Some("2001:db8::1".parse().expect("router")),
            })
        );
        assert_eq!(
            setup.ipv6_dns,
            ["2001:db8::53".parse::<Ipv6Addr>().expect("dns")]
        );
        assert_eq!(
            setup.wifi,
            Some(WifiSetup {
                ssid: b"Home Net".to_vec(),
                psk: Some("correct horse battery".to_owned()),
                hidden: false,
            })
        );
        let [edit] = plan.edits.as_slice() else {
            panic!("one file changes");
        };
        assert!(
            !edit.disabled().iter().any(|line| line.contains("horse")),
            "the password is never shown"
        );
        let edited = edit.edited("handed over");
        let expected = INSTALLER
            .replace(
                "auto lo wlan0\n",
                "# handed over\n#auto lo wlan0\nauto lo\n",
            )
            .replace("allow-hotplug wlan0", "#allow-hotplug wlan0")
            .replace("iface wlan0 inet dhcp", "#iface wlan0 inet dhcp")
            .replace("\twpa-", "#\twpa-")
            .replace("iface wlan0 inet6", "#iface wlan0 inet6")
            .replace("    address 2001", "#    address 2001")
            .replace("    gateway 2001", "#    gateway 2001")
            .replace("    dns-nameservers", "#    dns-nameservers");
        assert_eq!(edited, expected);
        let eth1 = super::plan(&configuration(&[(INTERFACES, INSTALLER)]), "eth1", false)
            .expect("a static stanza is understood");
        assert!(matches!(
            eth1.setup.ipv4,
            Ipv4Setup::Manual(ManualIpv4 { prefix: 24, .. })
        ));
    }

    #[test]
    fn refuses_what_it_cannot_carry_over() {
        let refused = |text: &str, interface: &str| {
            plan(
                &configuration(&[(INTERFACES, text), ("/etc/network/interfaces.d/vlan", "")]),
                interface,
                false,
            )
            .err()
        };
        assert_eq!(
            refused(
                "iface eth0 inet dhcp\n  post-up /usr/local/bin/route-hack\n",
                "eth0"
            ),
            Some(Refusal::Option("post-up".to_owned()))
        );
        assert_eq!(
            refused(
                "iface eth0 inet manual\nauto br0\niface br0 inet dhcp\n bridge_ports eth0\n",
                "eth0"
            ),
            Some(Refusal::Shared(
                PathBuf::from(INTERFACES),
                "bridge_ports eth0".to_owned()
            ))
        );
        assert_eq!(
            refused("iface eth0 inet dhcp\niface eth0.7 inet dhcp\n", "eth0"),
            Some(Refusal::Shared(
                PathBuf::from(INTERFACES),
                "iface eth0.7 inet dhcp".to_owned()
            ))
        );
        assert_eq!(
            refused("iface eth0 inet dhcp \\\n  hostname box\n", "eth0"),
            Some(Refusal::Continuation(PathBuf::from(INTERFACES)))
        );
        assert_eq!(
            refused("iface eth0 inet dhcp\niface eth0 inet static\n", "eth0"),
            Some(Refusal::Repeated("inet".to_owned()))
        );
        assert_eq!(refused("auto eth0\n", "eth0"), Some(Refusal::NotConfigured));
        assert_eq!(
            refused("iface wlan0 inet dhcp\n wpa-roam /etc/wpa.conf\n", "wlan0"),
            Some(Refusal::Option("wpa-roam".to_owned()))
        );
    }

    #[test]
    fn follows_sources_and_supplicant_files() {
        let root = tempfile::tempdir().expect("a folder");
        let write = |path: &str, text: &str| {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
            fs::write(path, text).expect("a file");
        };
        write(
            "etc/network/interfaces",
            "source-directory /etc/network/interfaces.d\nauto lo\niface lo inet loopback\n",
        );
        write(
            "etc/network/interfaces.d/wlan",
            "allow-hotplug wlan0\niface wlan0 inet dhcp\n  wpa-conf /etc/wpa_supplicant/home.conf\n",
        );
        write(
            "etc/network/interfaces.d/old.bak",
            "iface wlan0 inet static\n",
        );
        write(
            "etc/wpa_supplicant/home.conf",
            "ctrl_interface=/run/wpa_supplicant\nnetwork={\n\tssid=\"Home Net\"\n\tpsk=\"correct horse\"\n\tscan_ssid=1\n\tkey_mgmt=WPA-PSK\n}\n",
        );
        let configuration = read_under(root.path()).expect("the files are read");
        assert_eq!(configuration.files.len(), 2, "run-parts names only");
        let handed = plan(&configuration, "wlan0", true).expect("the stanza is understood");
        assert_eq!(
            handed.setup.wifi,
            Some(WifiSetup {
                ssid: b"Home Net".to_vec(),
                psk: Some("correct horse".to_owned()),
                hidden: true,
            })
        );
        assert_eq!(handed.edits.len(), 1, "the main file is left alone");
        assert!(brought_up("lo=lo\nwlan0=wlan0\n", "wlan0"));
        assert!(!brought_up("lo=lo\nwlan0.7=wlan0.7\n", "wlan0"));

        write(
            "etc/network/interfaces",
            "source interfaces.d/*\nsource /etc/network/more/*\n",
        );
        write(
            "etc/wpa_supplicant/home.conf",
            "network={\n ssid=486f6d65\n key_mgmt=NONE\n}\nnetwork={\n ssid=\"Work\"\n}\n",
        );
        let configuration = read_under(root.path()).expect("the files are read");
        assert_eq!(
            configuration.files.len(),
            3,
            "every file not starting with a dot"
        );
        assert_eq!(
            plan(&configuration, "wlan0", true).err(),
            Some(Refusal::Elsewhere("source /etc/network/more/*".to_owned()))
        );
        let mut only_ours = configuration.clone();
        only_ours.elsewhere.clear();
        only_ours
            .files
            .retain(|file| !file.path.ends_with("old.bak"));
        assert_eq!(
            plan(&only_ours, "wlan0", true).err(),
            Some(Refusal::Value("wpa-conf".to_owned())),
            "two networks in one file"
        );
        write(
            "etc/wpa_supplicant/home.conf",
            "network={\n ssid=486f6d65\n key_mgmt=NONE\n}\n",
        );
        let mut open = read_under(root.path()).expect("the files are read");
        open.elsewhere.clear();
        open.files.retain(|file| !file.path.ends_with("old.bak"));
        let wifi = plan(&open, "wlan0", true)
            .expect("an open network")
            .setup
            .wifi;
        assert_eq!(
            wifi,
            Some(WifiSetup {
                ssid: b"Home".to_vec(),
                psk: None,
                hidden: false,
            })
        );
    }
}
