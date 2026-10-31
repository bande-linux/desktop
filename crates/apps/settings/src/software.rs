use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};

use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{MatchRule, message};

use bande_notify::Urgency;
use bande_notify::client::{Request, notify};

use crate::L10N;
use crate::failure::{sentence, unexpected};
use crate::follow::{Update, Updates, spawn};

const PACKAGEKIT: &str = "org.freedesktop.PackageKit";
const TRANSACTION: &str = "org.freedesktop.PackageKit.Transaction";
const FILTER_NONE: u64 = 1 << 1;
const ONLY_TRUSTED: u64 = 1 << 1;
const UNKNOWN_PERCENTAGE: u32 = 101;
const RESTART_SYSTEM: u32 = 4;
const RESTART_SECURITY_SYSTEM: u32 = 6;
const EXIT_SUCCESS: u32 = 1;

#[zbus::proxy(
    interface = "org.freedesktop.PackageKit",
    default_service = "org.freedesktop.PackageKit",
    default_path = "/org/freedesktop/PackageKit"
)]
trait PackageKit {
    fn create_transaction(&self) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login {
    fn reboot(&self, interactive: bool) -> zbus::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub version: String,
    pub summary: String,
    pub size: u64,
}

impl Package {
    pub fn from_id(id: &str, summary: &str) -> Self {
        let mut parts = id.split(';');
        Self {
            id: id.to_owned(),
            name: parts.next().unwrap_or_default().to_owned(),
            version: parts.next().unwrap_or_default().to_owned(),
            summary: summary.to_owned(),
            size: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SoftwareState {
    #[default]
    Unknown,
    Checking,
    UpToDate,
    Available(Vec<Package>),
    Installing {
        percent: Option<u32>,
    },
    Installed {
        restart: bool,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoftwareCommand {
    Check,
    Install(Vec<String>),
    Restart,
}

#[derive(Debug, Default)]
struct Outcome {
    packages: Vec<Package>,
    sizes: HashMap<String, u64>,
    restart: bool,
    error: Option<String>,
}

fn size_of(details: &HashMap<String, OwnedValue>) -> Option<u64> {
    ["download-size", "size"]
        .iter()
        .find_map(|key| u64::try_from(details.get(*key)?).ok())
        .filter(|size| *size > 0)
}

fn run(
    system: &Connection,
    call: &dyn Fn(&zbus::blocking::Proxy<'_>) -> zbus::Result<()>,
    progress: &dyn Fn(u32),
) -> zbus::Result<Outcome> {
    let path = PackageKitProxyBlocking::new(system)?.create_transaction()?;
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .path(path.clone())?
        .build();
    let signals = MessageIterator::for_match_rule(rule, system, None)?;
    let transaction = zbus::blocking::Proxy::new(system, PACKAGEKIT, path, TRANSACTION)?;
    let _: () = transaction.call("SetHints", &(vec!["interactive=true"],))?;
    call(&transaction)?;
    let mut outcome = Outcome::default();
    for message in signals.flatten() {
        let header = message.header();
        let body = message.body();
        match header.member().map(|member| member.as_str()) {
            Some("Package") => {
                if let Ok((_, id, summary)) = body.deserialize::<(u32, String, String)>() {
                    outcome.packages.push(Package::from_id(&id, &summary));
                }
            }
            Some("Details") => {
                if let Ok(details) = body.deserialize::<HashMap<String, OwnedValue>>()
                    && let Some(id) = details
                        .get("package-id")
                        .and_then(|id| String::try_from(id.try_clone().ok()?).ok())
                    && let Some(size) = size_of(&details)
                {
                    outcome.sizes.insert(id, size);
                }
            }
            Some("RequireRestart") => {
                if let Ok((kind, _)) = body.deserialize::<(u32, String)>() {
                    outcome.restart |= matches!(kind, RESTART_SYSTEM | RESTART_SECURITY_SYSTEM);
                }
            }
            Some("ErrorCode") => {
                if let Ok((_, details)) = body.deserialize::<(u32, String)>() {
                    outcome.error = Some(details);
                }
            }
            Some("PropertiesChanged") => {
                if let Ok((_, changed, _)) =
                    body.deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                    && let Some(percent) = changed
                        .get("Percentage")
                        .and_then(|value| u32::try_from(value).ok())
                        .filter(|percent| *percent < UNKNOWN_PERCENTAGE)
                {
                    progress(percent);
                }
            }
            Some("Finished") => {
                if let Ok((exit, _)) = body.deserialize::<(u32, u32)>()
                    && exit != EXIT_SUCCESS
                    && outcome.error.is_none()
                {
                    outcome.error = Some(format!("PackageKit stopped with code {exit}"));
                }
                break;
            }
            _ => {}
        }
    }
    Ok(outcome)
}

fn failed(outcome: &Outcome) -> Option<SoftwareState> {
    outcome.error.clone().map(SoftwareState::Failed)
}

pub fn check(system: &Connection) -> SoftwareState {
    let checked = || -> zbus::Result<SoftwareState> {
        let refresh = run(
            system,
            &|transaction| transaction.call("RefreshCache", &(false,)),
            &|_| {},
        )?;
        if let Some(failure) = failed(&refresh) {
            return Ok(failure);
        }
        let updates = run(
            system,
            &|transaction| transaction.call("GetUpdates", &(FILTER_NONE,)),
            &|_| {},
        )?;
        if let Some(failure) = failed(&updates) {
            return Ok(failure);
        }
        let mut packages = updates.packages;
        if packages.is_empty() {
            return Ok(SoftwareState::UpToDate);
        }
        let ids: Vec<String> = packages.iter().map(|package| package.id.clone()).collect();
        let details = run(
            system,
            &|transaction| transaction.call("GetDetails", &(ids.clone(),)),
            &|_| {},
        )?;
        for package in &mut packages {
            package.size = details.sizes.get(&package.id).copied().unwrap_or(0);
        }
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(SoftwareState::Available(packages))
    };
    checked().unwrap_or_else(|error| SoftwareState::Failed(error.to_string()))
}

fn install(system: &Connection, ids: Vec<String>, updates: &Updates) -> SoftwareState {
    let installed = run(
        system,
        &|transaction| transaction.call("UpdatePackages", &(ONLY_TRUSTED, ids.clone())),
        &|percent| {
            updates.send(Update::Software(SoftwareState::Installing {
                percent: Some(percent),
            }));
        },
    );
    match installed {
        Ok(outcome) => failed(&outcome).unwrap_or(SoftwareState::Installed {
            restart: outcome.restart,
        }),
        Err(error) => SoftwareState::Failed(sentence(&error, "service-updates")),
    }
}

pub fn announce_updates(session: &Connection, system: &Connection) -> zbus::Result<bool> {
    let SoftwareState::Available(packages) = check(system) else {
        return Ok(false);
    };
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface("org.freedesktop.Notifications")?
        .build();
    let signals = MessageIterator::for_match_rule(rule, session, None)?;
    let title = L10N.tr("updates-notification");
    let body = L10N.tr_args(
        "updates-notification-body",
        [("count", packages.len().into())],
    );
    let show = L10N.tr("show-updates");
    let app_name = L10N.tr("title");
    let id = notify(
        session,
        &Request {
            app_name: &app_name,
            replaces_id: 0,
            app_icon: "org.bande.Settings",
            summary: &title,
            body: &body,
            actions: &[("default", &show)],
            urgency: Urgency::Normal,
            sound: None,
        },
    )?;
    for message in signals.flatten() {
        let header = message.header();
        match header.member().map(|member| member.as_str()) {
            Some("ActionInvoked") if message.body().deserialize::<(u32, String)>()?.0 == id => {
                return Ok(true);
            }
            Some("NotificationClosed") if message.body().deserialize::<(u32, u32)>()?.0 == id => {
                return Ok(false);
            }
            _ => {}
        }
    }
    Ok(false)
}

pub fn follow_software(system: &Connection, updates: Updates) -> Sender<SoftwareCommand> {
    let (sender, receiver) = mpsc::channel();
    let system = system.clone();
    spawn("software", move || {
        for command in receiver {
            let state = match command {
                SoftwareCommand::Check => {
                    updates.send(Update::Software(SoftwareState::Checking));
                    check(&system)
                }
                SoftwareCommand::Install(ids) => {
                    updates.send(Update::Software(SoftwareState::Installing {
                        percent: None,
                    }));
                    install(&system, ids, &updates)
                }
                SoftwareCommand::Restart => {
                    if let Err(error) =
                        LoginProxyBlocking::new(&system).and_then(|login| login.reboot(true))
                    {
                        updates.send(unexpected(error));
                    }
                    continue;
                }
            };
            if !updates.send(Update::Software(state)) {
                break;
            }
        }
    });
    sender
}
