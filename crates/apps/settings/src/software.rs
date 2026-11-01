mod stale;

use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};

use bande_packages::offline::{self, Results};
use bande_packages::{
    Class, Client, Error, ErrorCode, Failure, Mode, Progress, Restart, Status, running_release,
};
use bande_status::devices::end_session;
use bande_status::session::Ending;
use bande_xdg::{replace_file, state_home};
use zbus::MatchRule;
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator};
use zbus::fdo::RequestNameFlags;
use zbus::message;

use bande_notify::Urgency;
use bande_notify::client::{Request, close, notify};

use crate::L10N;
use crate::failure::{sentence, unexpected};
use crate::follow::{Update, Updates, spawn};
use crate::privilege::may_update_live;

const FRESH_SECONDS: u32 = 60 * 60;
const REBOOT_REQUIRED: &str = "/run/reboot-required";
const BOOT_ID: &str = "/proc/sys/kernel/random/boot_id";
const RESTART_FILE: &str = "bande/restart-pending";
const INTERRUPTED: &str = "dpkg --configure -a";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub version: String,
    pub summary: String,
    pub size: u64,
    pub class: Class,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Check,
    Download,
    Install,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    Always,
    WhenStale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reboot {
    Finish,
    Install(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    Reboot,
    LogOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SoftwareState {
    #[default]
    Unknown,
    Unavailable,
    Checking,
    UpToDate,
    Available(Vec<Package>),
    Downloading {
        percent: Option<u32>,
    },
    Installing {
        percent: Option<u32>,
    },
    Prepared(usize),
    Installed {
        count: usize,
        reboot: Option<Reboot>,
        logout: bool,
        apps: Vec<String>,
    },
    Interrupted,
    Failed {
        during: Stage,
        reason: String,
    },
}

impl SoftwareState {
    pub(crate) fn pending(&self) -> Option<String> {
        match self {
            Self::Prepared(_)
            | Self::Installed {
                reboot: Some(Reboot::Install(_)),
                ..
            } => Some(L10N.tr("restart-to-update")),
            Self::Installed {
                reboot: Some(Reboot::Finish),
                ..
            } => Some(L10N.tr("restart-to-finish")),
            Self::Installed { logout: true, .. } => Some(L10N.tr("log-out-to-finish")),
            Self::Interrupted => Some(L10N.tr("updates-interrupted")),
            _ => None,
        }
    }

    pub(crate) fn later(&mut self, need: Need) -> bool {
        let Self::Installed {
            count,
            reboot,
            logout,
            ..
        } = self
        else {
            return false;
        };
        match need {
            Need::Reboot => *reboot = None,
            Need::LogOut => *logout = false,
        }
        *count == 0 && reboot.is_none() && !*logout
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoftwareCommand {
    Open,
    Check,
    Update(Vec<Package>),
    Restart,
    LogOut,
    Repair,
}

fn explain(failure: &Failure) -> String {
    tracing::warn!("PackageKit: {:?}: {}", failure.code, failure.details);
    match failure.code {
        ErrorCode::NoNetwork => L10N.tr("updates-no-network"),
        ErrorCode::Locked => L10N.tr("updates-locked"),
        ErrorCode::NoSpace => L10N.tr("updates-no-space"),
        ErrorCode::NotAuthorized => L10N.tr("administrator-only"),
        ErrorCode::Cancelled => L10N.tr("updates-cancelled"),
        _ => L10N.tr("updates-went-wrong"),
    }
}

fn broken(during: Stage, error: &Error) -> SoftwareState {
    let reason = match error {
        Error::Missing => return SoftwareState::Unavailable,
        Error::Failed(failure) if failure.details.contains(INTERRUPTED) => {
            return SoftwareState::Interrupted;
        }
        Error::Vanished => L10N.tr("updates-service-stopped"),
        Error::Failed(failure) => explain(failure),
        Error::Bus(error) => sentence(error, "service-updates"),
    };
    SoftwareState::Failed { during, reason }
}

pub fn check(client: &Client, refresh: Refresh) -> SoftwareState {
    let checked = || -> Result<SoftwareState, Error> {
        if refresh == Refresh::Always || client.seconds_since_refresh()? >= FRESH_SECONDS {
            client.refresh(false)?;
        }
        let found = client.updates()?;
        if found.is_empty() {
            return Ok(SoftwareState::UpToDate);
        }
        let ids: Vec<String> = found.iter().map(|package| package.id.clone()).collect();
        let sizes: HashMap<String, u64> = client
            .details(&ids)?
            .into_iter()
            .map(|details| (details.id, details.size))
            .collect();
        let restarts: HashMap<String, Restart> = client
            .update_details(&ids)?
            .into_iter()
            .map(|detail| (detail.id, detail.restart))
            .collect();
        let release = running_release();
        let mut packages: Vec<Package> = found
            .into_iter()
            .map(|package| Package {
                class: Class::of(
                    &package,
                    restarts.get(&package.id).copied().unwrap_or_default(),
                    &release,
                ),
                size: sizes.get(&package.id).copied().unwrap_or(0),
                id: package.id,
                name: package.name,
                version: package.version,
                summary: package.summary,
            })
            .collect();
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(SoftwareState::Available(packages))
    };
    checked().unwrap_or_else(|error| broken(Stage::Check, &error))
}

fn finished_offline(system: &Connection) -> Result<Option<SoftwareState>, Error> {
    let Some(results) = offline::results(system)? else {
        return Ok(None);
    };
    if let Err(error) = offline::clear_results(system) {
        tracing::warn!("cannot clear the offline update results: {error}");
    }
    Ok(Some(match results {
        Results::Installed(ids) => SoftwareState::Installed {
            count: ids.len(),
            reboot: None,
            logout: false,
            apps: Vec::new(),
        },
        Results::Failed(failure) => SoftwareState::Failed {
            during: Stage::Install,
            reason: explain(&failure),
        },
    }))
}

fn waiting_offline(system: &Connection) -> Result<Option<SoftwareState>, Error> {
    if !offline::triggered(system)? {
        return Ok(None);
    }
    let prepared = offline::prepared(system)?;
    Ok((!prepared.is_empty()).then_some(SoftwareState::Prepared(prepared.len())))
}

pub fn open_updates(system: &Connection, client: &Client) -> SoftwareState {
    let pending = finished_offline(system).and_then(|finished| match finished {
        Some(state) => Ok(Some(state)),
        None => waiting_offline(system),
    });
    match pending {
        Ok(Some(state)) => state,
        Ok(None) if restart_pending() => SoftwareState::Installed {
            count: 0,
            reboot: Some(Reboot::Finish),
            logout: false,
            apps: Vec::new(),
        },
        Ok(None) => check(client, Refresh::WhenStale),
        Err(error) => broken(Stage::Check, &error),
    }
}

fn restart_file() -> Option<PathBuf> {
    state_home().map(|state| state.join(RESTART_FILE))
}

fn boot_id() -> Option<String> {
    fs::read_to_string(BOOT_ID).ok()
}

fn remember_restart() {
    if let Some((file, boot)) = restart_file().zip(boot_id())
        && let Err(error) = replace_file(&file, &boot)
    {
        tracing::warn!("cannot remember the pending restart: {error}");
    }
}

fn restart_pending() -> bool {
    Path::new(REBOOT_REQUIRED).exists()
        || restart_file()
            .and_then(|file| fs::read_to_string(file).ok())
            .zip(boot_id())
            .is_some_and(|(remembered, boot)| remembered == boot)
}

fn ids(packages: &[&Package]) -> Vec<String> {
    packages.iter().map(|package| package.id.clone()).collect()
}

fn prepare(
    system: &Connection,
    client: &Client,
    ids: &[String],
    report: &dyn Fn(SoftwareState),
) -> Result<(), (Stage, Error)> {
    client
        .download(ids, &|progress| {
            report(SoftwareState::Downloading {
                percent: progress.percent,
            });
        })
        .and_then(|()| offline::trigger(system))
        .map_err(|error| (Stage::Download, error))
}

pub fn update(
    system: &Connection,
    client: &Client,
    packages: &[Package],
    live: bool,
    report: &dyn Fn(SoftwareState),
) -> SoftwareState {
    let (offline, now): (Vec<&Package>, Vec<&Package>) = packages
        .iter()
        .partition(|package| !live || package.class == Class::Offline);
    let installing = Cell::new(false);
    let installed = || -> Result<SoftwareState, (Stage, Error)> {
        let now_ids = ids(&now);
        let release = running_release();
        let pulls_offline = !now.is_empty()
            && client
                .update(&now_ids, Mode::Simulate, &|_| {})
                .map_err(|error| (Stage::Install, error))?
                .packages
                .iter()
                .any(|package| Class::of(package, Restart::None, &release) == Class::Offline);
        if now.is_empty() || pulls_offline {
            let all: Vec<&Package> = packages.iter().collect();
            prepare(system, client, &ids(&all), report)?;
            return Ok(SoftwareState::Prepared(packages.len()));
        }
        let applied = client
            .update(&now_ids, Mode::Apply, &|progress: Progress| {
                installing.set(
                    installing.get()
                        || matches!(progress.status, Status::Installing | Status::Removing),
                );
                let percent = progress.percent;
                report(if installing.get() {
                    SoftwareState::Installing { percent }
                } else {
                    SoftwareState::Downloading { percent }
                });
            })
            .map_err(|error| (Stage::Install, error))?;
        let reboot = if offline.is_empty() {
            (applied.restart
                || now.iter().any(|package| package.class == Class::Boot)
                || Path::new(REBOOT_REQUIRED).exists())
            .then_some(Reboot::Finish)
        } else {
            prepare(system, client, &ids(&offline), report)?;
            Some(Reboot::Install(offline.len()))
        };
        Ok(SoftwareState::Installed {
            count: now.len(),
            reboot,
            logout: false,
            apps: Vec::new(),
        })
    };
    installed().unwrap_or_else(|(stage, error)| broken(stage, &error))
}

fn settle(session: &Connection, state: SoftwareState) -> SoftwareState {
    let SoftwareState::Installed { count, reboot, .. } = state else {
        return state;
    };
    if reboot == Some(Reboot::Finish) {
        remember_restart();
    }
    let stale = stale::finish_in_session(session);
    SoftwareState::Installed {
        count,
        reboot,
        logout: stale.logout,
        apps: stale.apps,
    }
}

fn announce(session: &Connection, summary: &str, body: &str) -> zbus::Result<Option<u32>> {
    let show = L10N.tr("show-updates");
    let app_name = L10N.tr("title");
    notify(
        session,
        &Request {
            app_name: &app_name,
            replaces_id: 0,
            app_icon: "org.bande.Settings",
            summary,
            body,
            actions: &[("default", &show)],
            urgency: Urgency::Normal,
            sound: None,
        },
    )
    .map(Some)
}

const NOTICE_NAME: &str = "org.bande.Settings.UpdateNotice";

pub struct UpdateNotice {
    session: Connection,
    messages: MessageIterator,
}

impl UpdateNotice {
    pub fn listen(session: &Connection) -> zbus::Result<Self> {
        let messages = MessageIterator::from(session);
        let rule = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .interface("org.freedesktop.Notifications")?
            .build();
        DBusProxy::new(session)?.add_match_rule(rule)?;
        session.request_name_with_flags(
            NOTICE_NAME,
            RequestNameFlags::ReplaceExisting | RequestNameFlags::AllowReplacement,
        )?;
        Ok(Self {
            session: session.clone(),
            messages,
        })
    }

    pub fn clicked(self, id: u32) -> zbus::Result<bool> {
        for message in self.messages.flatten() {
            let header = message.header();
            match header.member().map(|member| member.as_str()) {
                Some("ActionInvoked") if message.body().deserialize::<(u32, String)>()?.0 == id => {
                    return Ok(true);
                }
                Some("NotificationClosed")
                    if message.body().deserialize::<(u32, u32)>()?.0 == id =>
                {
                    return Ok(false);
                }
                Some("NameLost") if message.body().deserialize::<&str>()? == NOTICE_NAME => {
                    close(&self.session, id)?;
                    return Ok(false);
                }
                _ => {}
            }
        }
        Ok(false)
    }
}

pub fn announce_updates(session: &Connection, system: &Connection) -> zbus::Result<Option<u32>> {
    let finished = finished_offline(system).unwrap_or_else(|error| {
        tracing::warn!("cannot read the offline update results: {error}");
        None
    });
    match finished {
        Some(SoftwareState::Installed { count, .. }) => {
            let summary = L10N.tr_args("updates-were-installed", [("count", count.into())]);
            return announce(session, &summary, "");
        }
        Some(SoftwareState::Failed { reason, .. }) => {
            return announce(session, &L10N.tr("install-failed"), &reason);
        }
        _ => {}
    }
    if let Ok(Some(SoftwareState::Prepared(count))) = waiting_offline(system) {
        let summary = L10N.tr_args("restart-to-install", [("count", count.into())]);
        return announce(session, &summary, &L10N.tr("installs-on-restart"));
    }
    if restart_pending() {
        return announce(
            session,
            &L10N.tr("restart-to-finish"),
            &L10N.tr("restart-to-finish-hint"),
        );
    }
    let SoftwareState::Available(packages) = check(&Client::background(system), Refresh::Always)
    else {
        return Ok(None);
    };
    let body = L10N.tr_args(
        "updates-notification-body",
        [("count", packages.len().into())],
    );
    announce(session, &L10N.tr("updates-notification"), &body)
}

fn end(system: &Connection, ending: Ending, updates: &Updates) {
    if let Err(error) = end_session(system, ending) {
        updates.send(unexpected(error));
    }
}

pub fn follow_software(
    session: &Connection,
    system: &Connection,
    updates: Updates,
) -> Sender<SoftwareCommand> {
    let (sender, receiver) = mpsc::channel();
    let session = session.clone();
    let system = system.clone();
    spawn("software", move || {
        let client = Client::new(&system);
        for command in receiver {
            let state = match command {
                SoftwareCommand::Open => {
                    updates.send(Update::Software(SoftwareState::Checking));
                    open_updates(&system, &client)
                }
                SoftwareCommand::Check => {
                    updates.send(Update::Software(SoftwareState::Checking));
                    check(&client, Refresh::Always)
                }
                SoftwareCommand::Update(packages) => {
                    updates.send(Update::Software(SoftwareState::Downloading {
                        percent: None,
                    }));
                    let live = may_update_live(&system);
                    let state = update(&system, &client, &packages, live, &|state| {
                        updates.send(Update::Software(state));
                    });
                    settle(&session, state)
                }
                SoftwareCommand::Repair => {
                    updates.send(Update::Software(SoftwareState::Installing {
                        percent: None,
                    }));
                    let repaired = client.repair(&|progress| {
                        updates.send(Update::Software(SoftwareState::Installing {
                            percent: progress.percent,
                        }));
                    });
                    match repaired {
                        Ok(()) => check(&client, Refresh::WhenStale),
                        Err(error) => broken(Stage::Install, &error),
                    }
                }
                SoftwareCommand::Restart => {
                    end(&system, Ending::Restart, &updates);
                    continue;
                }
                SoftwareCommand::LogOut => {
                    end(&system, Ending::LogOut, &updates);
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
