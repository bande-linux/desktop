mod badge;
mod idle;
mod stale;
pub mod versions;

use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use bande_l10n::{text, tr, trn};
use bande_packages::offline::{self, Results};
use bande_packages::{
    Class, Client, Error, ErrorCode, Failure, Mode, Progress, Restart, Status, flatpak,
    running_release,
};
use bande_xdg::{replace_file, state_home};
use zbus::MatchRule;
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator};
use zbus::fdo::RequestNameFlags;
use zbus::message;

use crate::client::ConfigProxyBlocking;
use crate::failure::explained;
use crate::follow::{Update, Updates, spawn};
use crate::privilege::may_update_live;
use bande_notify::Urgency;
use bande_notify::client::{Request, close, notify};

const FRESH_SECONDS: u32 = 60 * 60;
const REBOOT_REQUIRED: &str = "/run/reboot-required";
const BOOT_ID: &str = "/proc/sys/kernel/random/boot_id";
const RESTART_FILE: &str = "bande/restart-pending";
const INSTALLING_FILE: &str = "bande-installing-updates";
const WENT_BACK_FILE: &str = "bande/went-back-told";
const INTERRUPTED: &str = "dpkg --configure -a";
const CHECK_SLACK: Duration = Duration::from_secs(6 * 60 * 60);
const NEVER_REFRESHED: u32 = u32::MAX;
const IDLE_FOR: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub version: String,
    pub summary: String,
    pub size: u64,
    pub class: Class,
}

impl Package {
    pub(crate) fn needs_restart(&self) -> bool {
        matches!(self.class, Class::Boot | Class::Offline)
    }
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
    Never,
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
            } => Some(tr!("Restart to install updates")),
            Self::Installed {
                reboot: Some(Reboot::Finish),
                ..
            } => Some(tr!("Restart to finish updating")),
            Self::Installed { logout: true, .. } => {
                Some(tr!("Log out to finish updating the desktop"))
            }
            Self::Interrupted => Some(tr!("Updates didn't finish installing")),
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
    Glance,
    Open,
    Check,
    Update(Vec<Package>),
    Repair,
    Restore(u32),
}

fn explain(failure: &Failure) -> String {
    tracing::warn!("PackageKit: {:?}: {}", failure.code, failure.details);
    match failure.code {
        ErrorCode::NoNetwork => tr!("You're offline. Connect to the internet and try again."),
        ErrorCode::Locked => tr!("Another installation is running. Try again when it finishes."),
        ErrorCode::NoSpace => {
            tr!("There isn't enough free space. Free up some space and try again.")
        }
        ErrorCode::NotAuthorized => tr!("Only an administrator can change this."),
        ErrorCode::Cancelled => tr!("The update was cancelled."),
        _ => tr!("Something went wrong. Try again later."),
    }
}

fn broken(during: Stage, error: &Error) -> SoftwareState {
    let reason = match error {
        Error::Missing => return SoftwareState::Unavailable,
        Error::Failed(failure) if failure.details.contains(INTERRUPTED) => {
            return SoftwareState::Interrupted;
        }
        Error::Vanished => tr!("The update service stopped. Try again in a moment."),
        Error::Failed(failure) => explain(failure),
        Error::Bus(error) => explained(
            error,
            text!("Software Update"),
            text!("The update service stopped. Try again in a moment."),
        ),
    };
    SoftwareState::Failed { during, reason }
}

fn flatpak_updates() -> Vec<Package> {
    flatpak::updates()
        .unwrap_or_else(|error| {
            if error != flatpak::Error::Missing {
                tracing::warn!("cannot look for Flatpak updates: {error}");
            }
            Vec::new()
        })
        .into_iter()
        .map(|update| Package {
            id: update.reference,
            name: match update.name.is_empty() {
                true => update.id,
                false => update.name,
            },
            version: update.version,
            summary: String::new(),
            size: 0,
            class: Class::Flatpak,
        })
        .collect()
}

fn refresh_flatpak_catalogue() {
    if let Err(error) = flatpak::refresh_catalogue()
        && error != flatpak::Error::Missing
    {
        tracing::warn!("cannot refresh the Flatpak catalogue: {error}");
    }
}

pub fn check(client: &Client, refresh: Refresh) -> SoftwareState {
    let checked = || -> Result<SoftwareState, Error> {
        let may_fetch = refresh != Refresh::Never;
        if refresh == Refresh::Always
            || (may_fetch && client.seconds_since_refresh()? >= FRESH_SECONDS)
        {
            client.refresh(false)?;
            refresh_flatpak_catalogue();
        }
        let found = client.updates()?;
        let flatpaks = may_fetch.then(flatpak_updates).unwrap_or_default();
        if found.is_empty() && flatpaks.is_empty() {
            return Ok(SoftwareState::UpToDate);
        }
        let ids: Vec<String> = found.iter().map(|package| package.id.clone()).collect();
        if ids.is_empty() {
            return Ok(SoftwareState::Available(flatpaks));
        }
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
        packages.extend(flatpaks);
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
    if versions::available() {
        return Ok(versions::prepared().map(SoftwareState::Prepared));
    }
    if !offline::triggered(system)? {
        return Ok(None);
    }
    let prepared = offline::prepared(system)?;
    Ok((!prepared.is_empty()).then_some(SoftwareState::Prepared(prepared.len())))
}

fn check_interval(session: &Connection) -> Option<Duration> {
    let seconds = ConfigProxyBlocking::new(session)
        .and_then(|config| config.update_check_every())
        .unwrap_or_else(|error| {
            tracing::warn!("cannot read how often to check for updates: {error}");
            0
        });
    (seconds > 0).then(|| Duration::from_secs(seconds))
}

fn check_due(session: &Connection, client: &Client) -> bool {
    let Some(every) = check_interval(session) else {
        return true;
    };
    client
        .seconds_since_refresh()
        .map(|since| u64::from(since) + CHECK_SLACK.as_secs() >= every.as_secs())
        .unwrap_or(true)
}

pub fn last_check(client: &Client) -> Option<i64> {
    let since = client.seconds_since_refresh().ok()?;
    (since != NEVER_REFRESHED).then(|| jiff::Timestamp::now().as_second() - i64::from(since))
}

pub fn open_updates(session: &Connection, system: &Connection, client: &Client) -> SoftwareState {
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
        Ok(None) => {
            let refresh = match check_due(session, client) {
                true => Refresh::WhenStale,
                false => Refresh::Never,
            };
            match check(client, refresh) {
                SoftwareState::UpToDate => installed_earlier(),
                state => state,
            }
        }
        Err(error) => broken(Stage::Check, &error),
    }
}

fn installed_earlier() -> SoftwareState {
    let stale = stale::look();
    if !stale.logout && stale.apps.is_empty() {
        return SoftwareState::UpToDate;
    }
    SoftwareState::Installed {
        count: 0,
        reboot: None,
        logout: stale.logout,
        apps: stale.apps,
    }
}

fn hold_installing() -> Option<fs::File> {
    let file = fs::File::create(runtime_dir()?.join(INSTALLING_FILE))
        .inspect_err(|error| tracing::warn!("cannot mark updates as installing: {error}"))
        .ok()?;
    file.try_lock().ok()?;
    Some(file)
}

fn settings_is_installing() -> bool {
    runtime_dir()
        .and_then(|directory| fs::File::create(directory.join(INSTALLING_FILE)).ok())
        .is_some_and(|file| file.try_lock().is_err())
}

fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
}

pub fn finish_update(session: &Connection) -> bool {
    stale::finish_in_session(session).logout && !settings_is_installing()
}

pub fn announce_log_out(session: &Connection) -> zbus::Result<Option<u32>> {
    announce(
        session,
        &tr!("Log out to finish updating the desktop"),
        &tr!("The desktop keeps running its old version until you log out"),
    )
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
    if versions::available() {
        report(SoftwareState::Installing { percent: None });
        return versions::prepare(system).map_err(|error| (Stage::Install, error));
    }
    client
        .download(ids, &|progress| {
            report(SoftwareState::Downloading {
                percent: progress.percent,
            });
        })
        .and_then(|()| offline::trigger(system))
        .map_err(|error| (Stage::Download, error))
}

fn pulls_offline(client: &Client, ids: &[String]) -> Result<bool, Error> {
    let release = running_release();
    Ok(!ids.is_empty()
        && client
            .update(ids, Mode::Simulate, &|_| {})?
            .packages
            .iter()
            .any(|package| Class::of(package, Restart::None, &release) == Class::Offline))
}

fn flatpak_reason(error: &flatpak::Error) -> String {
    tracing::warn!("cannot update Flatpak apps: {error}");
    let flatpak::Error::Failed(details) = error else {
        return tr!("Something went wrong. Try again later.");
    };
    let details = details.to_lowercase();
    if ["resolve", "connect", "network"]
        .iter()
        .any(|word| details.contains(word))
    {
        tr!("You're offline. Connect to the internet and try again.")
    } else if details.contains("space") {
        tr!("There isn't enough free space. Free up some space and try again.")
    } else {
        tr!("Something went wrong. Try again later.")
    }
}

fn update_flatpaks(
    flatpaks: &[&Package],
    download_only: bool,
    report: &dyn Fn(SoftwareState),
) -> Result<(), flatpak::Error> {
    if flatpaks.is_empty() {
        return Ok(());
    }
    flatpak::update(
        &ids(flatpaks),
        download_only,
        &|percent| {
            report(match download_only {
                true => SoftwareState::Downloading { percent },
                false => SoftwareState::Installing { percent },
            })
        },
        &flatpak::Canceller::default(),
    )
}

pub fn update(
    system: &Connection,
    client: &Client,
    packages: &[Package],
    report: &dyn Fn(SoftwareState),
) -> SoftwareState {
    let _installing = hold_installing();
    let (flatpaks, debian): (Vec<&Package>, Vec<&Package>) = packages
        .iter()
        .partition(|package| package.class == Class::Flatpak);
    if let Err(error) = update_flatpaks(&flatpaks, false, report) {
        return SoftwareState::Failed {
            during: Stage::Install,
            reason: flatpak_reason(&error),
        };
    }
    if debian.is_empty() {
        return SoftwareState::Installed {
            count: flatpaks.len(),
            reboot: None,
            logout: false,
            apps: Vec::new(),
        };
    }
    let (offline, now): (Vec<&Package>, Vec<&Package>) =
        debian.iter().copied().partition(|package| {
            package.class == Class::Offline
                || (package.class == Class::Boot && versions::available())
        });
    let installing = Cell::new(false);
    let installed = || -> Result<SoftwareState, (Stage, Error)> {
        let now_ids = ids(&now);
        let pulls_offline =
            pulls_offline(client, &now_ids).map_err(|error| (Stage::Install, error))?;
        if now.is_empty() || pulls_offline {
            prepare(system, client, &ids(&debian), report)?;
            return Ok(SoftwareState::Prepared(debian.len()));
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
            count: now.len() + flatpaks.len(),
            reboot,
            logout: false,
            apps: Vec::new(),
        })
    };
    installed().unwrap_or_else(|(stage, error)| broken(stage, &error))
}

fn settle(state: SoftwareState) -> SoftwareState {
    let SoftwareState::Installed { count, reboot, .. } = state else {
        return state;
    };
    if reboot == Some(Reboot::Finish) {
        remember_restart();
    }
    let stale = stale::look();
    SoftwareState::Installed {
        count,
        reboot,
        logout: stale.logout,
        apps: stale.apps,
    }
}

fn announce(session: &Connection, summary: &str, body: &str) -> zbus::Result<Option<u32>> {
    let show = tr!("Show Updates");
    let app_name = tr!("System Settings");
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
            desktop_entry: Some("org.bande.Settings"),
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

pub(crate) fn restart_hint() -> String {
    match versions::available() {
        true => tr!(
            "The update is ready. If it can't start, your computer goes back to this version by itself"
        ),
        false => tr!("Updates install while the computer restarts"),
    }
}

fn went_back_is_news() -> bool {
    let Some((file, boot)) = state_home()
        .map(|state| state.join(WENT_BACK_FILE))
        .zip(boot_id())
    else {
        return false;
    };
    if fs::read_to_string(&file).is_ok_and(|told| told == boot) {
        return false;
    }
    if let Err(error) = replace_file(&file, &boot) {
        tracing::warn!("cannot remember that the system went back: {error}");
    }
    true
}

pub fn announce_updates(session: &Connection, system: &Connection) -> zbus::Result<Option<u32>> {
    if versions::went_back().is_some() && went_back_is_news() {
        return announce(
            session,
            &tr!("The update couldn't start"),
            &tr!(
                "Your computer went back to the previous version. Your files are as you left them"
            ),
        );
    }
    let finished = finished_offline(system).unwrap_or_else(|error| {
        tracing::warn!("cannot read the offline update results: {error}");
        None
    });
    match finished {
        Some(SoftwareState::Installed { count, .. }) => {
            let summary = trn!(
                "1 update was installed",
                "{count} updates were installed",
                count = count
            );
            return announce(session, &summary, "");
        }
        Some(SoftwareState::Failed { reason, .. }) => {
            return announce(session, &tr!("Could not install updates"), &reason);
        }
        _ => {}
    }
    if let Ok(Some(SoftwareState::Prepared(count))) = waiting_offline(system) {
        let summary = trn!(
            "Restart to install 1 update",
            "Restart to install {count} updates",
            count = count
        );
        return announce(session, &summary, &restart_hint());
    }
    if restart_pending() {
        return announce(
            session,
            &tr!("Restart to finish updating"),
            &tr!("Some updates take effect after a restart"),
        );
    }
    if stale::look().logout {
        return announce_log_out(session);
    }
    let client = Client::background(system);
    if !check_due(session, &client) {
        return Ok(None);
    }
    let checked = check(&client, Refresh::Always);
    badge::remember(&checked);
    let SoftwareState::Available(packages) = checked else {
        return Ok(None);
    };
    if automatic(session) {
        return update_automatically(session, system, &client, &packages);
    }
    announce_available(session, &packages)
}

fn announce_available(session: &Connection, packages: &[Package]) -> zbus::Result<Option<u32>> {
    let restart = packages
        .iter()
        .filter(|package| package.needs_restart())
        .count();
    announce_waiting(session, packages.len() - restart, restart)
}

fn announce_waiting(
    session: &Connection,
    approve: usize,
    restart: usize,
) -> zbus::Result<Option<u32>> {
    let body = match (approve, restart) {
        (_, 0) => trn!(
            "1 update is ready to install.",
            "{count} updates are ready to install.",
            count = approve
        ),
        (0, _) => trn!(
            "1 update is ready to install. It needs a restart.",
            "{count} updates are ready to install. They need a restart.",
            count = restart
        ),
        _ => format!(
            "{} {}",
            trn!(
                "1 update is ready to install.",
                "{count} updates are ready to install.",
                count = approve + restart
            ),
            trn!(
                "1 of them needs a restart.",
                "{count} of them need a restart.",
                count = restart
            )
        ),
    };
    announce(session, &tr!("Software updates are available"), &body)
}

fn automatic(session: &Connection) -> bool {
    ConfigProxyBlocking::new(session)
        .and_then(|config| config.automatic_updates())
        .unwrap_or_else(|error| {
            tracing::warn!("cannot read the automatic updates setting: {error}");
            false
        })
}

fn download_in_background(client: &Client, packages: &[Package]) {
    let (flatpaks, debian): (Vec<&Package>, Vec<&Package>) = packages
        .iter()
        .partition(|package| package.class == Class::Flatpak);
    if !debian.is_empty()
        && let Err(error) = client.download(&ids(&debian), &|_| {})
    {
        tracing::warn!("cannot download the updates: {error}");
    }
    if let Err(error) = update_flatpaks(&flatpaks, true, &|_| {}) {
        tracing::warn!("cannot download the Flatpak updates: {error}");
    }
}

struct Sorted {
    now: Vec<Package>,
    approve: usize,
    restart: usize,
}

fn sort_updates(client: &Client, packages: Vec<Package>, live: bool) -> Sorted {
    let (mut now, later): (Vec<Package>, Vec<Package>) = packages
        .into_iter()
        .partition(|package| matches!(package.class, Class::Flatpak | Class::Live));
    let debian: Vec<&Package> = now
        .iter()
        .filter(|package| package.class == Class::Live)
        .collect();
    let count = debian.len();
    let mut sorted = Sorted {
        now: Vec::new(),
        approve: 0,
        restart: later.len(),
    };
    if pulls_offline(client, &ids(&debian)).unwrap_or(true) {
        sorted.restart += count;
    } else if !live {
        sorted.approve = count;
    } else {
        sorted.now = now;
        return sorted;
    }
    now.retain(|package| package.class == Class::Flatpak);
    sorted.now = now;
    sorted
}

fn update_automatically(
    session: &Connection,
    system: &Connection,
    client: &Client,
    packages: &[Package],
) -> zbus::Result<Option<u32>> {
    download_in_background(client, packages);
    if let Err(error) = idle::wait_until_idle(IDLE_FOR) {
        tracing::warn!("cannot wait for the computer to be idle: {error}");
        return announce_available(session, packages);
    }
    let SoftwareState::Available(packages) = check(client, Refresh::WhenStale) else {
        return Ok(None);
    };
    let Sorted {
        now,
        approve,
        restart,
    } = sort_updates(client, packages, may_update_live(system));
    let installed = match now.is_empty() {
        true => None,
        false => Some(settle(update(system, client, &now, &|_| {}))),
    };
    if let Some(state) = &installed {
        badge::remember(state);
    }
    match installed {
        Some(SoftwareState::Failed { reason, .. }) => {
            announce(session, &tr!("Could not install updates"), &reason)
        }
        Some(SoftwareState::Prepared(count)) => announce(
            session,
            &trn!(
                "Restart to install 1 update",
                "Restart to install {count} updates",
                count = count
            ),
            &restart_hint(),
        ),
        _ if approve + restart > 0 => announce_waiting(session, approve, restart),
        Some(SoftwareState::Installed {
            reboot: Some(_), ..
        }) => announce(
            session,
            &tr!("Restart to finish updating"),
            &tr!("Some updates take effect after a restart"),
        ),
        Some(SoftwareState::Installed { logout: true, .. }) => announce(
            session,
            &tr!("Log out to finish updating the desktop"),
            &tr!("The desktop keeps running its old version until you log out"),
        ),
        Some(SoftwareState::Installed { count, .. }) => announce(
            session,
            &trn!(
                "1 update was installed",
                "{count} updates were installed",
                count = count
            ),
            "",
        ),
        _ => Ok(None),
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
                SoftwareCommand::Glance => {
                    match badge::glance(check_interval(&session), restart_pending()) {
                        badge::Glance::Known(hint) => {
                            if !updates.send(Update::SoftwareHint(hint)) {
                                break;
                            }
                            continue;
                        }
                        badge::Glance::Due => {
                            updates.send(Update::Software(SoftwareState::Checking));
                            open_updates(&session, &system, &client)
                        }
                    }
                }
                SoftwareCommand::Open => {
                    updates.send(Update::Software(SoftwareState::Checking));
                    open_updates(&session, &system, &client)
                }
                SoftwareCommand::Check => {
                    updates.send(Update::Software(SoftwareState::Checking));
                    check(&client, Refresh::Always)
                }
                SoftwareCommand::Update(packages) => {
                    updates.send(Update::Software(SoftwareState::Downloading {
                        percent: None,
                    }));
                    let state = update(&system, &client, &packages, &|state| {
                        updates.send(Update::Software(state));
                    });
                    settle(state)
                }
                SoftwareCommand::Restore(id) => {
                    updates.send(Update::Software(SoftwareState::Installing {
                        percent: None,
                    }));
                    match versions::restore(&system, id) {
                        Ok(()) => SoftwareState::Installed {
                            count: 0,
                            reboot: Some(Reboot::Finish),
                            logout: false,
                            apps: Vec::new(),
                        },
                        Err(error) => broken(Stage::Install, &error),
                    }
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
            };
            badge::remember(&state);
            updates.send(Update::SoftwareChecked(last_check(&client)));
            if !updates.send(Update::Software(state)) {
                break;
            }
        }
    });
    sender
}
