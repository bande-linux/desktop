use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};

use bande_packages::offline::{self, Results};
use bande_packages::{Client, Error, ErrorCode, Failure};
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

const FRESH_SECONDS: u32 = 60 * 60;

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
    Prepared(usize),
    Installed(usize),
    Failed {
        during: Stage,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoftwareCommand {
    Open,
    Check,
    Download(Vec<String>),
    Restart,
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
        let mut packages: Vec<Package> = found
            .into_iter()
            .map(|package| Package {
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
        Results::Installed(ids) => SoftwareState::Installed(ids.len()),
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
        Ok(None) => check(client, Refresh::WhenStale),
        Err(error) => broken(Stage::Check, &error),
    }
}

fn download(
    system: &Connection,
    client: &Client,
    ids: &[String],
    updates: &Updates,
) -> SoftwareState {
    let prepared = client
        .download(ids, &|progress| {
            updates.send(Update::Software(SoftwareState::Downloading {
                percent: progress.percent,
            }));
        })
        .and_then(|()| offline::trigger(system));
    match prepared {
        Ok(()) => SoftwareState::Prepared(ids.len()),
        Err(error) => broken(Stage::Download, &error),
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
        Some(SoftwareState::Installed(count)) => {
            let summary = L10N.tr_args("updates-were-installed", [("count", count.into())]);
            return announce(session, &summary, "");
        }
        Some(SoftwareState::Failed { reason, .. }) => {
            return announce(session, &L10N.tr("install-failed"), &reason);
        }
        _ => {}
    }
    if matches!(waiting_offline(system), Ok(Some(_))) {
        return Ok(None);
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

pub fn follow_software(system: &Connection, updates: Updates) -> Sender<SoftwareCommand> {
    let (sender, receiver) = mpsc::channel();
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
                SoftwareCommand::Download(ids) => {
                    updates.send(Update::Software(SoftwareState::Downloading {
                        percent: None,
                    }));
                    download(&system, &client, &ids, &updates)
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
