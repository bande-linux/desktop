use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::os::fd::AsFd;
use std::path::Path;
use std::time::Duration;

use bande_https::ureq;
use roxmltree::{Document, Node};
use rustix::fs::{MemfdFlags, memfd_create};
use sha2::{Digest, Sha256};
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::{self, Flags, Message};
use zbus::zvariant::{Fd, OwnedValue, Value};
use zbus::{MatchRule, OwnedMatchRule};

const SERVICE: &str = "org.freedesktop.fwupd";
const PATH: &str = "/";
const INTERFACE: &str = "org.freedesktop.fwupd";
const ERROR_PREFIX: &str = "org.freedesktop.fwupd.";
const BUS: &str = "org.freedesktop.DBus";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const UPDATABLE: u64 = 1 << 1;
const REMOTE_ENABLED: u64 = 1 << 0;
const NEEDS_REBOOT: u64 = 1 << 8;
const NEEDS_SHUTDOWN: u64 = 1 << 17;
const STATE_PENDING: u32 = 1;
const STATE_NEEDS_REBOOT: u32 = 4;
const REMOTE_DOWNLOAD: u32 = 1;
const REMOTE_LOCAL: u32 = 2;
const STATUS_DOWNLOADING: u32 = 8;
const UNKNOWN_PERCENTAGE: u32 = 101;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const UNSIZED_LIMIT: u64 = 512 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
const PROBLEMS: [(u64, Problem); 9] = [
    (1 << 0, Problem::BatteryLow),
    (1 << 1, Problem::NotReachable),
    (1 << 2, Problem::DeviceBatteryLow),
    (1 << 3, Problem::AlreadyPending),
    (1 << 4, Problem::AcPowerRequired),
    (1 << 5, Problem::LidClosed),
    (1 << 8, Problem::Busy),
    (1 << 9, Problem::Busy),
    (1 << 10, Problem::Busy),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Update {
    pub(crate) device: String,
    pub(crate) name: String,
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) notes: String,
    pub(crate) size: u64,
    pub(crate) at_restart: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Problem {
    BatteryLow,
    DeviceBatteryLow,
    AcPowerRequired,
    LidClosed,
    AlreadyPending,
    Busy,
    NotReachable,
    NeedsUserAction,
    NotAuthorized,
    Invalid,
    UpToDate,
    Other,
}

impl Problem {
    fn named(name: &str) -> Self {
        match name.strip_prefix(ERROR_PREFIX).unwrap_or(name) {
            "BatteryLevelTooLow" => Self::BatteryLow,
            "AcPowerRequired" => Self::AcPowerRequired,
            "AuthFailed" | "AuthExpired" | "PermissionDenied" => Self::NotAuthorized,
            "AlreadyPending" => Self::AlreadyPending,
            "Busy" | "TimedOut" => Self::Busy,
            "NotReachable" => Self::NotReachable,
            "NeedsUserAction" => Self::NeedsUserAction,
            "SignatureInvalid" | "InvalidFile" | "InvalidData" => Self::Invalid,
            "NothingToDo" | "VersionSame" | "VersionNewer" => Self::UpToDate,
            _ => Self::Other,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Error {
    Missing,
    Vanished,
    Failed(Problem, String),
    Offline(String),
    Download(String),
    Bus(zbus::Error),
}

impl From<zbus::Error> for Error {
    fn from(error: zbus::Error) -> Self {
        if bande_bus::is_unowned(&error) {
            return Self::Missing;
        }
        match &error {
            zbus::Error::MethodError(name, details, _) if name.starts_with(ERROR_PREFIX) => {
                Self::Failed(Problem::named(name), details.clone().unwrap_or_default())
            }
            _ => Self::Bus(error),
        }
    }
}

impl From<zbus::fdo::Error> for Error {
    fn from(error: zbus::fdo::Error) -> Self {
        Self::from(zbus::Error::from(error))
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Download(error.to_string())
    }
}

impl From<ureq::Error> for Error {
    fn from(error: ureq::Error) -> Self {
        match error {
            ureq::Error::StatusCode(_) | ureq::Error::BadUri(_) => {
                Self::Download(error.to_string())
            }
            other => Self::Offline(other.to_string()),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "fwupd is not installed"),
            Self::Vanished => write!(f, "fwupd stopped"),
            Self::Failed(problem, details) => write!(f, "{problem:?}: {details}"),
            Self::Offline(details) | Self::Download(details) => write!(f, "{details}"),
            Self::Bus(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Downloading(Option<u32>),
    Installing(Option<u32>),
}

type Record = HashMap<String, OwnedValue>;

fn text(record: &Record, key: &str) -> String {
    record
        .get(key)
        .and_then(|value| String::try_from(value.try_clone().ok()?).ok())
        .unwrap_or_default()
}

fn number<T: TryFrom<OwnedValue>>(record: &Record, key: &str) -> Option<T> {
    T::try_from(record.get(key)?.try_clone().ok()?).ok()
}

fn texts(record: &Record, key: &str) -> Vec<String> {
    record
        .get(key)
        .and_then(|value| Vec::<String>::try_from(value.try_clone().ok()?).ok())
        .unwrap_or_default()
}

fn flags(record: &Record) -> u64 {
    number(record, "Flags").unwrap_or(0)
}

fn daemon(system: &Connection) -> zbus::Result<Proxy<'static>> {
    Proxy::new(system, SERVICE, PATH, INTERFACE)
}

fn devices(system: &Connection) -> Result<Vec<Record>, Error> {
    Ok(daemon(system)?.call("GetDevices", &())?)
}

fn upgrades(system: &Connection, device: &str) -> Result<Vec<Record>, Error> {
    match daemon(system)?.call("GetUpgrades", &(device,)) {
        Ok(releases) => Ok(releases),
        Err(error) => match Error::from(error) {
            Error::Failed(_, details) => {
                tracing::debug!("no firmware upgrades for {device}: {details}");
                Ok(Vec::new())
            }
            other => Err(other),
        },
    }
}

fn plain(markup: &str) -> String {
    fn words(node: Node) -> String {
        node.descendants()
            .filter(Node::is_text)
            .filter_map(|node| node.text())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
    let wrapped = format!("<description>{markup}</description>");
    let Ok(document) = Document::parse(&wrapped) else {
        return markup.split_whitespace().collect::<Vec<_>>().join(" ");
    };
    let mut lines = Vec::new();
    for block in document.root_element().children().filter(Node::is_element) {
        match block.tag_name().name() {
            "ul" | "ol" => lines.extend(
                block
                    .children()
                    .filter(|item| item.has_tag_name("li"))
                    .map(|item| format!("• {}", words(item))),
            ),
            _ => lines.push(words(block)),
        }
    }
    lines.retain(|line| !line.is_empty());
    lines.join("\n")
}

fn restarts(device: &Record) -> bool {
    flags(device) & (NEEDS_REBOOT | NEEDS_SHUTDOWN) != 0
}

fn waiting(device: &Record) -> bool {
    matches!(
        number::<u32>(device, "UpdateState"),
        Some(STATE_PENDING | STATE_NEEDS_REBOOT)
    )
}

pub(crate) fn updates(system: &Connection) -> Result<Vec<Update>, Error> {
    let mut found = Vec::new();
    for device in devices(system)? {
        if flags(&device) & UPDATABLE == 0 || waiting(&device) {
            continue;
        }
        let id = text(&device, "DeviceId");
        let Some(release) = upgrades(system, &id)?.into_iter().next() else {
            continue;
        };
        found.push(Update {
            name: text(&device, "Name"),
            from: text(&device, "Version"),
            to: text(&release, "Version"),
            notes: plain(&text(&release, "Description")),
            size: number(&release, "Size").unwrap_or(0),
            at_restart: restarts(&device),
            device: id,
        });
    }
    Ok(found)
}

pub(crate) fn pending(system: &Connection) -> Result<usize, Error> {
    Ok(devices(system)?
        .iter()
        .filter(|device| waiting(device))
        .count())
}

fn agent() -> ureq::Agent {
    bande_https::config("settings")
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .build()
        .into()
}

fn download(
    agent: &ureq::Agent,
    url: &str,
    expected: u64,
    report: &dyn Fn(Option<u32>),
) -> Result<Vec<u8>, Error> {
    let response = agent.get(url).call()?;
    let total = match expected {
        0 => response.body().content_length().unwrap_or(0),
        size => size,
    };
    let limit = match total {
        0 => UNSIZED_LIMIT,
        size => size.saturating_mul(2),
    };
    let mut reader = response.into_body().into_reader();
    let mut bytes = Vec::new();
    let mut chunk = vec![0; CHUNK];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() as u64 > limit {
            return Err(Error::Download(format!("{url} is larger than expected")));
        }
        report((total > 0).then(|| (bytes.len() as u64 * 100 / total).min(100) as u32));
    }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn memory_file(bytes: &[u8]) -> Result<File, Error> {
    let mut file = File::from(
        memfd_create("bande-firmware", MemfdFlags::CLOEXEC)
            .map_err(|error| Error::Download(error.to_string()))?,
    );
    file.write_all(bytes)?;
    file.rewind()?;
    Ok(file)
}

fn remotes(system: &Connection) -> Result<Vec<Record>, Error> {
    Ok(daemon(system)?.call("GetRemotes", &())?)
}

fn enabled(remote: &Record) -> bool {
    flags(remote) & REMOTE_ENABLED != 0 || number(remote, "Enabled").unwrap_or(false)
}

pub(crate) fn refresh(system: &Connection) -> Result<(), Error> {
    let agent = agent();
    for remote in remotes(system)? {
        let id = text(&remote, "RemoteId");
        let uri = text(&remote, "Uri");
        if !enabled(&remote)
            || number::<u32>(&remote, "Type") != Some(REMOTE_DOWNLOAD)
            || uri.is_empty()
        {
            continue;
        }
        if !text(&remote, "Username").is_empty() {
            tracing::info!("not refreshing the firmware remote {id}: it needs a login");
            continue;
        }
        let signature = download(&agent, &format!("{uri}.jcat"), 0, &|_| {})?;
        if sha256(&signature) == text(&remote, "Checksum") {
            continue;
        }
        let metadata = download(&agent, &uri, 0, &|_| {})?;
        let (metadata, signature) = (memory_file(&metadata)?, memory_file(&signature)?);
        let () = daemon(system)?.call(
            "UpdateMetadata",
            &(
                id.as_str(),
                Fd::from(metadata.as_fd()),
                Fd::from(signature.as_fd()),
            ),
        )?;
    }
    Ok(())
}

fn basename(location: &str) -> &str {
    let path = location.split(['?', '#']).next().unwrap_or(location);
    path.rsplit('/').next().unwrap_or(path)
}

fn directory(uri: &str) -> &str {
    uri.rsplit_once('/').map_or(uri, |(directory, _)| directory)
}

enum Source {
    Web(String),
    File(String),
}

fn source(location: &str, remote: Option<&Record>) -> Source {
    if let Some(path) = location.strip_prefix("file://") {
        return Source::File(path.to_owned());
    }
    let web = location.starts_with("https://") || location.starts_with("http://");
    let Some(remote) = remote else {
        return Source::Web(location.to_owned());
    };
    let cache = text(remote, "FilenameCache");
    if number::<u32>(remote, "Type") == Some(REMOTE_LOCAL) && !web && !cache.is_empty() {
        return Source::File(format!("{}/{location}", directory(&cache)));
    }
    let base = text(remote, "FirmwareBaseUri");
    if web && !base.is_empty() {
        return Source::Web(format!(
            "{}/{}",
            base.trim_end_matches('/'),
            basename(location)
        ));
    }
    if !location.contains('/') {
        return Source::Web(format!("{}/{location}", directory(&text(remote, "Uri"))));
    }
    Source::Web(location.to_owned())
}

fn refusal(device: &Record) -> Option<Problem> {
    let problems: u64 = number(device, "Problems").unwrap_or(0);
    PROBLEMS
        .iter()
        .find(|(bit, _)| problems & bit != 0)
        .map(|(_, problem)| *problem)
}

fn fetch(system: &Connection, release: &Record, report: &dyn Fn(Step)) -> Result<Vec<u8>, Error> {
    let remote_id = text(release, "RemoteId");
    let remote = match remote_id.is_empty() {
        true => None,
        false => remotes(system)?
            .into_iter()
            .find(|remote| text(remote, "RemoteId") == remote_id),
    };
    let location = texts(release, "Locations")
        .into_iter()
        .next()
        .or_else(|| Some(text(release, "Uri")).filter(|uri| !uri.is_empty()))
        .ok_or_else(|| Error::Failed(Problem::Invalid, "the release has no location".into()))?;
    let bytes = match source(&location, remote.as_ref()) {
        Source::File(path) => fs::read(Path::new(&path))?,
        Source::Web(url) => download(
            &agent(),
            &url,
            number(release, "Size").unwrap_or(0),
            &|percent| report(Step::Downloading(percent)),
        )?,
    };
    let checksum = text(release, "Checksum");
    let expected = checksum.split(',').find(|sum| sum.len() == 64);
    if let Some(expected) = expected
        && !expected.eq_ignore_ascii_case(&sha256(&bytes))
    {
        return Err(Error::Failed(
            Problem::Invalid,
            format!("{location} does not match its checksum"),
        ));
    }
    Ok(bytes)
}

fn rules() -> zbus::Result<[OwnedMatchRule; 2]> {
    Ok([
        MatchRule::builder()
            .msg_type(message::Type::Signal)
            .sender(SERVICE)?
            .path(PATH)?
            .interface(PROPERTIES)?
            .member("PropertiesChanged")?
            .build()
            .into(),
        MatchRule::builder()
            .msg_type(message::Type::Signal)
            .sender(BUS)?
            .interface(BUS)?
            .member("NameOwnerChanged")?
            .arg(0, SERVICE)?
            .build()
            .into(),
    ])
}

fn left(message: &Message) -> bool {
    let header = message.header();
    header
        .member()
        .is_some_and(|member| member == "NameOwnerChanged")
        && message
            .body()
            .deserialize::<(String, String, String)>()
            .is_ok_and(|(name, old, _)| name == SERVICE && !old.is_empty())
}

fn installing(message: &Message) -> Option<Option<u32>> {
    let header = message.header();
    if header.member()? != "PropertiesChanged" {
        return None;
    }
    let (interface, changed, _) = message
        .body()
        .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
        .ok()?;
    if interface != INTERFACE {
        return None;
    }
    if number::<u32>(&changed, "Status") == Some(STATUS_DOWNLOADING) {
        return None;
    }
    let percent: u32 = number(&changed, "Percentage")?;
    Some((percent < UNKNOWN_PERCENTAGE && percent > 0).then_some(percent))
}

fn wait_for_install(
    system: &Connection,
    messages: MessageIterator,
    call: &Message,
    report: &dyn Fn(Step),
) -> Result<(), Error> {
    let serial = call.primary_header().serial_num();
    system.send(call)?;
    for message in messages.flatten() {
        if left(&message) {
            return Err(Error::Vanished);
        }
        let header = message.header();
        if header.reply_serial() == Some(serial) {
            return match header.message_type() {
                message::Type::Error => {
                    let name = header
                        .error_name()
                        .map(|name| name.to_string())
                        .unwrap_or_default();
                    let details = message.body().deserialize::<String>().unwrap_or_default();
                    Err(Error::Failed(Problem::named(&name), details))
                }
                _ => Ok(()),
            };
        }
        if let Some(percent) = installing(&message) {
            report(Step::Installing(percent));
        }
    }
    Err(Error::Vanished)
}

fn send_install(
    system: &Connection,
    device: &str,
    file: &File,
    filename: &str,
    report: &dyn Fn(Step),
) -> Result<(), Error> {
    let options: HashMap<&str, Value> = HashMap::from([
        ("reason", Value::from("user-action")),
        ("filename", Value::from(filename)),
        ("install-flags", Value::from(0_u64)),
    ]);
    let call = Message::method_call(PATH, "Install")?
        .destination(SERVICE)?
        .interface(INTERFACE)?
        .with_flags(Flags::AllowInteractiveAuth)?
        .build(&(device, Fd::from(file.as_fd()), options))?;
    let bus = DBusProxy::new(system)?;
    let rules = rules()?;
    let messages = MessageIterator::from(system);
    let installed = (|| -> Result<(), Error> {
        for rule in &rules {
            bus.add_match_rule(rule.inner().clone())?;
        }
        wait_for_install(system, messages, &call, report)
    })();
    for rule in rules {
        if let Err(error) = bus.remove_match_rule(rule.into_inner()) {
            tracing::warn!("cannot remove an fwupd match rule: {error}");
        }
    }
    installed
}

pub(crate) fn install(
    system: &Connection,
    device: &str,
    report: &dyn Fn(Step),
) -> Result<bool, Error> {
    let found = devices(system)?
        .into_iter()
        .find(|record| text(record, "DeviceId") == device)
        .ok_or_else(|| Error::Failed(Problem::NotReachable, format!("{device} is gone")))?;
    if let Some(problem) = refusal(&found) {
        return Err(Error::Failed(problem, "fwupd reports a problem".into()));
    }
    let Some(release) = upgrades(system, device)?.into_iter().next() else {
        return Ok(false);
    };
    report(Step::Downloading(None));
    let bytes = fetch(system, &release, report)?;
    let file = memory_file(&bytes)?;
    let location = texts(&release, "Locations").into_iter().next();
    let filename = location.as_deref().map(basename).unwrap_or("firmware.cab");
    report(Step::Installing(None));
    match send_install(system, device, &file, filename, report) {
        Err(Error::Failed(Problem::UpToDate, _)) => return Ok(false),
        result => result?,
    }
    let after = devices(system)?
        .into_iter()
        .find(|record| text(record, "DeviceId") == device);
    Ok(after.is_some_and(|record| waiting(&record)) || restarts(&found))
}
