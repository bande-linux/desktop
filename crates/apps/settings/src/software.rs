mod badge;
mod firmware;
mod idle;
mod stale;

use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use bande_l10n::{text, tr, trn};
use bande_packages::offline::{self, Results};
use bande_packages::schedule::{INSTALL, check_due, check_interval, software_updates};
use bande_packages::{
    Applies, Class, Client, Error, ErrorCode, Failure, Mode, Progress, Restart, Status, flatpak,
    running_release,
};
use bande_toolkit::SessionEnd;
use bande_xdg::{replace_file, state_home};
use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator};
use zbus::fdo::RequestNameFlags;
use zbus::message;

use crate::failure::explained;
use crate::follow::{Update, Updates, spawn};
use crate::privilege::may_update_live;
use bande_notify::Urgency;
use bande_notify::client::{Listener, Request, Signal, close, notify};

const FRESH_SECONDS: u32 = 60 * 60;
const REBOOT_REQUIRED: &str = "/run/reboot-required";
const BOOT_ID: &str = "/proc/sys/kernel/random/boot_id";
const RESTART_FILE: &str = "bande/restart-pending";
const INTERRUPTED: &str = "dpkg --configure -a";
const NEVER_REFRESHED: u32 = u32::MAX;
const BUS: &str = "org.freedesktop.DBus";
const FWUPD: &str = "fwupd";
const IDLE_FOR: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub from: String,
    pub version: String,
    pub summary: String,
    pub size: u64,
    pub class: Class,
}

impl Package {
    pub(crate) fn needs_restart(&self) -> bool {
        matches!(
            self.class,
            Class::Boot
                | Class::Offline
                | Class::Firmware(Applies::AtRestart | Applies::AtShutdown)
        )
    }

    fn is_firmware(&self) -> bool {
        matches!(self.class, Class::Firmware { .. })
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
    Install { count: usize, ending: Ending },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ending {
    Restart,
    FirmwareRestart,
    FirmwareShutDown,
    FirmwareShutDownAndPackages,
}

impl Ending {
    fn join(self, other: Self) -> Self {
        match (self.min(other), self.max(other)) {
            (Self::Restart, Self::FirmwareShutDown) => Self::FirmwareShutDownAndPackages,
            (_, most) => most,
        }
    }

    pub(crate) fn shuts_down(self) -> bool {
        matches!(
            self,
            Self::FirmwareShutDown | Self::FirmwareShutDownAndPackages
        )
    }

    fn of(applies: Applies) -> Self {
        match applies {
            Applies::AtShutdown => Self::FirmwareShutDown,
            Applies::Now | Applies::AtRestart => Self::FirmwareRestart,
        }
    }

    pub(crate) fn session_end(self) -> SessionEnd {
        match self.shuts_down() {
            true => SessionEnd::ShutDown,
            false => SessionEnd::Restart,
        }
    }

    pub(crate) fn action(self) -> String {
        match self.shuts_down() {
            true => tr!("Shut Down Now"),
            false => tr!("Restart Now"),
        }
    }

    pub(crate) fn title(self, count: usize) -> String {
        match self.shuts_down() {
            true => trn!(
                "Shut down to install 1 update",
                "Shut down to install {count} updates",
                count = count
            ),
            false => trn!(
                "Restart to install 1 update",
                "Restart to install {count} updates",
                count = count
            ),
        }
    }

    pub(crate) fn hint(self) -> String {
        match self {
            Self::Restart => tr!("Updates install while the computer restarts"),
            Self::FirmwareRestart => tr!(
                "Updates install while the computer restarts. Keep the computer plugged in until it has started again."
            ),
            Self::FirmwareShutDown => tr!(
                "Firmware installs while the computer is shut down. Keep it plugged in, then turn it on again."
            ),
            Self::FirmwareShutDownAndPackages => tr!(
                "Firmware installs while the computer is shut down, and the other updates when you turn it on again. Keep it plugged in."
            ),
        }
    }
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
    Prepared {
        count: usize,
        ending: Ending,
    },
    Installed {
        count: usize,
        reboot: Option<Reboot>,
        logout: bool,
        apps: Vec<String>,
        failed: Option<String>,
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
            Self::Prepared { ending, .. }
            | Self::Installed {
                reboot: Some(Reboot::Install { ending, .. }),
                ..
            } if ending.shuts_down() => Some(tr!("Shut down to install updates")),
            Self::Prepared { .. }
            | Self::Installed {
                reboot: Some(Reboot::Install { .. }),
                ..
            } => Some(tr!("Restart to install updates")),
            Self::Installed {
                reboot: Some(Reboot::Finish),
                ..
            } => Some(tr!("Restart to finish updating")),
            Self::Installed { logout: true, .. } => {
                Some(tr!("Log out to finish updating the desktop"))
            }
            Self::Interrupted => Some(tr!("Updates didn’t finish installing")),
            _ => None,
        }
    }

    pub(crate) fn later(&mut self, need: Need) -> bool {
        let Self::Installed {
            count,
            reboot,
            logout,
            failed,
            ..
        } = self
        else {
            return false;
        };
        match need {
            Need::Reboot => *reboot = None,
            Need::LogOut => *logout = false,
        }
        *count == 0 && reboot.is_none() && !*logout && failed.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoftwareCommand {
    Glance,
    Open,
    Check,
    Update(Vec<Package>),
    Repair,
    InstallFirmwareSupport,
}

pub(crate) fn explain(failure: &Failure) -> String {
    tracing::warn!("PackageKit: {:?}: {}", failure.code, failure.details);
    match failure.code {
        ErrorCode::NoNetwork => tr!("You’re offline. Connect to the internet and try again."),
        ErrorCode::Locked => tr!("Another installation is running. Try again when it finishes."),
        ErrorCode::NoSpace => {
            tr!("There isn’t enough free space. Free up some space and try again.")
        }
        ErrorCode::NotAuthorized => tr!("Only an administrator can change this."),
        ErrorCode::Cancelled => tr!("The update was canceled."),
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
    flatpak::updates(true)
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
            from: String::new(),
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

fn firmware_updates(system: &Connection, refresh: bool) -> Vec<Package> {
    if refresh
        && let Err(error) = firmware::refresh(system)
        && !matches!(error, firmware::Error::Missing)
    {
        tracing::warn!("cannot refresh the firmware metadata: {error}");
    }
    let updates = firmware::updates(system).unwrap_or_else(|error| {
        if !matches!(error, firmware::Error::Missing) {
            tracing::warn!("cannot look for firmware updates: {error}");
        }
        Vec::new()
    });
    let mut packages: Vec<Package> = updates
        .into_iter()
        .map(|update| Package {
            id: update.device,
            name: update.name,
            from: update.from,
            version: update.to,
            summary: update.notes,
            size: update.size,
            class: Class::Firmware(update.applies),
        })
        .collect();
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    packages
}

pub fn check(system: &Connection, client: &Client, refresh: Refresh) -> SoftwareState {
    let checked = || -> Result<SoftwareState, Error> {
        let may_fetch = refresh != Refresh::Never;
        let refreshing = refresh == Refresh::Always
            || (may_fetch && client.seconds_since_refresh()? >= FRESH_SECONDS);
        if refreshing {
            client.refresh(false, &|_| {})?;
            refresh_flatpak_catalogue();
        }
        let found = client.updates()?;
        let flatpaks = may_fetch.then(flatpak_updates).unwrap_or_default();
        let mut listed = firmware_updates(system, refreshing);
        if found.is_empty() && flatpaks.is_empty() && listed.is_empty() {
            return Ok(SoftwareState::UpToDate);
        }
        let ids: Vec<String> = found.iter().map(|package| package.id.clone()).collect();
        if ids.is_empty() {
            listed.extend(flatpaks);
            return Ok(SoftwareState::Available(listed));
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
                from: String::new(),
                version: package.version,
                summary: package.summary,
            })
            .collect();
        packages.extend(flatpaks);
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        listed.extend(packages);
        Ok(SoftwareState::Available(listed))
    };
    checked().unwrap_or_else(|error| broken(Stage::Check, &error))
}

fn finished_firmware(system: &Connection) -> Vec<firmware::Finished> {
    firmware::finished(system).unwrap_or_else(|error| {
        if !matches!(error, firmware::Error::Missing) {
            tracing::warn!("cannot read the firmware update results: {error}");
        }
        Vec::new()
    })
}

fn finished_offline(system: &Connection) -> Result<Option<SoftwareState>, Error> {
    let results = offline::results(system)?;
    if results.is_some()
        && let Err(error) = offline::clear_results(system)
    {
        tracing::warn!("cannot clear the offline update results: {error}");
    }
    let flashed = finished_firmware(system);
    if results.is_none() && flashed.is_empty() {
        return Ok(None);
    }
    let mut count = 0;
    let mut problems = Vec::new();
    match results {
        Some(Results::Installed(ids)) => count += ids.len(),
        Some(Results::Failed(failure)) => problems.push(explain(&failure)),
        None => {}
    }
    let kept = tr!("The firmware update didn’t install. The device keeps its earlier firmware.");
    for finished in flashed {
        match finished.installed {
            true => count += 1,
            false => problems.push(tr!(
                "{device}: {reason}",
                device = finished.name,
                reason = kept.clone()
            )),
        }
    }
    let failed = (!problems.is_empty()).then(|| problems.join("\n"));
    Ok(Some(match (count, failed) {
        (0, Some(reason)) => SoftwareState::Failed {
            during: Stage::Install,
            reason,
        },
        (count, failed) => SoftwareState::Installed {
            count,
            reboot: None,
            logout: false,
            apps: Vec::new(),
            failed,
        },
    }))
}

fn pending_firmware(system: &Connection) -> Vec<Applies> {
    firmware::pending(system).unwrap_or_else(|error| {
        if !matches!(error, firmware::Error::Missing) {
            tracing::warn!("cannot read the pending firmware updates: {error}");
        }
        Vec::new()
    })
}

fn waiting_offline(system: &Connection) -> Result<Option<SoftwareState>, Error> {
    let prepared = match offline::triggered(system)? {
        true => offline::prepared(system)?.len(),
        false => 0,
    };
    let firmware = pending_firmware(system);
    let firmware_ending = firmware.iter().map(|applies| Ending::of(*applies)).max();
    let ending = match (prepared > 0, firmware_ending) {
        (true, Some(ending)) => Ending::Restart.join(ending),
        (_, ending) => ending.unwrap_or(Ending::Restart),
    };
    let count = prepared + firmware.len();
    Ok((count > 0).then_some(SoftwareState::Prepared { count, ending }))
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
            failed: None,
        },
        Ok(None) => {
            let refresh = match check_due(session, client) {
                true => Refresh::WhenStale,
                false => Refresh::Never,
            };
            match check(system, client, refresh) {
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
        failed: None,
    }
}

fn announce_log_out(session: &Connection) -> zbus::Result<Option<u32>> {
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
        tr!("You’re offline. Connect to the internet and try again.")
    } else if details.contains("space") {
        tr!("There isn’t enough free space. Free up some space and try again.")
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

fn firmware_reason(error: &firmware::Error) -> String {
    tracing::warn!("fwupd: {error}");
    match error {
        firmware::Error::Failed(problem, _) => match problem {
            firmware::Problem::BatteryLow => {
                tr!("The battery is too low to update firmware safely. Charge it and try again.")
            }
            firmware::Problem::DeviceBatteryLow => {
                tr!("The device’s battery is too low. Charge it and try again.")
            }
            firmware::Problem::AcPowerRequired => {
                tr!("Plug in the computer to install this firmware update.")
            }
            firmware::Problem::LidClosed => tr!("Open the lid to install this firmware update."),
            firmware::Problem::AlreadyPending => {
                tr!("A firmware update is already waiting for a restart.")
            }
            firmware::Problem::Busy => tr!("The device is busy. Try again in a moment."),
            firmware::Problem::NotReachable => {
                tr!("The device isn’t connected. Connect it and try again.")
            }
            firmware::Problem::NeedsUserAction => {
                tr!("The device needs your attention. Follow its instructions and try again.")
            }
            firmware::Problem::NotAuthorized => tr!("Only an administrator can change this."),
            firmware::Problem::Cancelled => tr!("The update was canceled."),
            firmware::Problem::Invalid => {
                tr!("The firmware file is damaged or not signed. Try again later.")
            }
            firmware::Problem::UpToDate | firmware::Problem::Other => {
                tr!("Something went wrong. Try again later.")
            }
        },
        firmware::Error::Offline(_) => {
            tr!("You’re offline. Connect to the internet and try again.")
        }
        firmware::Error::Download(_) => tr!("Could not download the firmware. Try again later."),
        firmware::Error::Missing | firmware::Error::Vanished | firmware::Error::Bus(_) => {
            tr!("The firmware update service stopped. Try again in a moment.")
        }
    }
}

#[derive(Default)]
struct Flashed {
    now: usize,
    waiting: usize,
    ending: Option<Ending>,
    failed: Option<String>,
    cancelled: bool,
}

fn install_firmware(
    system: &Connection,
    devices: &[&Package],
    report: &dyn Fn(SoftwareState),
) -> Flashed {
    let mut flashed = Flashed::default();
    for device in devices {
        let installed = firmware::install(system, &device.id, &|step| {
            report(match step {
                firmware::Step::Downloading(percent) => SoftwareState::Downloading { percent },
                firmware::Step::Installing(percent) => SoftwareState::Installing { percent },
            })
        });
        match installed {
            Ok(None) => {}
            Ok(Some(Applies::Now)) => flashed.now += 1,
            Ok(Some(applies)) => {
                flashed.waiting += 1;
                flashed.ending = flashed.ending.max(Some(Ending::of(applies)));
            }
            Err(firmware::Error::Failed(firmware::Problem::Cancelled, _)) => {
                flashed.cancelled = true;
                return flashed;
            }
            Err(error) => {
                flashed.failed = Some(tr!(
                    "{device}: {reason}",
                    device = device.name.clone(),
                    reason = firmware_reason(&error)
                ));
                return flashed;
            }
        }
    }
    flashed
}

fn with_firmware(state: SoftwareState, flashed: Flashed) -> SoftwareState {
    let joined = |ending: Ending| {
        flashed
            .ending
            .map_or(ending, |firmware| ending.join(firmware))
    };
    match state {
        SoftwareState::Prepared { count, ending } if flashed.failed.is_none() => {
            SoftwareState::Prepared {
                count: count + flashed.waiting,
                ending: joined(ending),
            }
        }
        SoftwareState::Prepared { count, ending } => SoftwareState::Installed {
            count: flashed.now,
            reboot: Some(Reboot::Install {
                count: count + flashed.waiting,
                ending: joined(ending),
            }),
            logout: false,
            apps: Vec::new(),
            failed: flashed.failed,
        },
        SoftwareState::Installed {
            count,
            reboot,
            logout,
            apps,
            ..
        } => SoftwareState::Installed {
            count: count + flashed.now,
            reboot: match (reboot, flashed.ending) {
                (Some(Reboot::Install { count, ending }), _) => Some(Reboot::Install {
                    count: count + flashed.waiting,
                    ending: joined(ending),
                }),
                (_, Some(ending)) => Some(Reboot::Install {
                    count: flashed.waiting,
                    ending,
                }),
                (other, None) => other,
            },
            logout,
            apps,
            failed: flashed.failed,
        },
        other => other,
    }
}

pub fn update(
    system: &Connection,
    client: &Client,
    packages: &[Package],
    report: &dyn Fn(SoftwareState),
) -> SoftwareState {
    let (devices, packages): (Vec<&Package>, Vec<&Package>) =
        packages.iter().partition(|package| package.is_firmware());
    let state = update_packages(system, client, &packages, report);
    if devices.is_empty()
        || !matches!(
            state,
            SoftwareState::Installed { .. } | SoftwareState::Prepared { .. }
        )
    {
        return state;
    }
    let flashed = install_firmware(system, &devices, report);
    let cancelled = flashed.cancelled;
    let state = with_firmware(state, flashed);
    let nothing = SoftwareState::Installed {
        count: 0,
        reboot: None,
        logout: false,
        apps: Vec::new(),
        failed: None,
    };
    match cancelled && state == nothing {
        true => check(system, client, Refresh::Never),
        false => state,
    }
}

fn update_packages(
    system: &Connection,
    client: &Client,
    packages: &[&Package],
    report: &dyn Fn(SoftwareState),
) -> SoftwareState {
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
            failed: None,
        };
    }
    let (offline, now): (Vec<&Package>, Vec<&Package>) = debian
        .iter()
        .copied()
        .partition(|package| package.class == Class::Offline);
    let installing = Cell::new(false);
    let installed = || -> Result<SoftwareState, (Stage, Error)> {
        let now_ids = ids(&now);
        let pulls_offline =
            pulls_offline(client, &now_ids).map_err(|error| (Stage::Install, error))?;
        if now.is_empty() || pulls_offline {
            prepare(system, client, &ids(&debian), report)?;
            return Ok(SoftwareState::Prepared {
                count: debian.len(),
                ending: Ending::Restart,
            });
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
            Some(Reboot::Install {
                count: offline.len(),
                ending: Ending::Restart,
            })
        };
        Ok(SoftwareState::Installed {
            count: now.len() + flatpaks.len(),
            reboot,
            logout: false,
            apps: Vec::new(),
            failed: None,
        })
    };
    installed().unwrap_or_else(|(stage, error)| broken(stage, &error))
}

fn settle(session: &Connection, state: SoftwareState) -> SoftwareState {
    let SoftwareState::Installed {
        count,
        reboot,
        failed,
        ..
    } = state
    else {
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
        failed,
    }
}

fn announce(session: &Connection, summary: &str, body: &str) -> zbus::Result<Option<u32>> {
    let show = tr!("Show Updates");
    let app_name = tr!("Settings");
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

pub const UPDATE_NOTICE: &str = "org.bande.Settings.UpdateNotice";

pub struct Notice {
    session: Connection,
    listener: Listener,
    lost: MessageIterator,
}

impl Notice {
    pub fn listen(session: &Connection, name: &'static str) -> zbus::Result<Self> {
        let listener = Listener::new(session)?;
        let rule = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .sender(BUS)?
            .interface(BUS)?
            .member("NameLost")?
            .arg(0, name)?
            .build();
        let lost = MessageIterator::for_match_rule(rule, session, None)?;
        session.request_name_with_flags(
            name,
            RequestNameFlags::ReplaceExisting | RequestNameFlags::AllowReplacement,
        )?;
        Ok(Self {
            session: session.clone(),
            listener,
            lost,
        })
    }

    pub fn clicked(self, id: u32) -> zbus::Result<bool> {
        Ok(self.answer(id)?.is_some())
    }

    pub fn answer(self, id: u32) -> zbus::Result<Option<String>> {
        let session = self.session;
        let lost = self.lost;
        spawn("notice", move || {
            if lost.flatten().next().is_some()
                && let Err(error) = close(&session, id)
            {
                tracing::warn!("cannot close the replaced notification: {error}");
            }
        });
        let mut answer = None;
        self.listener.listen(|signal| match signal {
            Signal::ActionInvoked(invoked, action) if invoked == id => {
                answer = Some(action);
                false
            }
            Signal::Closed(closed) => closed != id,
            _ => true,
        });
        Ok(answer)
    }
}

pub fn announce_updates(session: &Connection, system: &Connection) -> zbus::Result<Option<u32>> {
    let finished = finished_offline(system).unwrap_or_else(|error| {
        tracing::warn!("cannot read the offline update results: {error}");
        None
    });
    match finished {
        Some(SoftwareState::Installed { count, failed, .. }) => {
            let summary = trn!(
                "1 update was installed",
                "{count} updates were installed",
                count = count
            );
            return announce(session, &summary, &failed.unwrap_or_default());
        }
        Some(SoftwareState::Failed { reason, .. }) => {
            return announce(session, &tr!("Could not install updates"), &reason);
        }
        _ => {}
    }
    if let Ok(Some(SoftwareState::Prepared { count, ending })) = waiting_offline(system) {
        return announce(session, &ending.title(count), &ending.hint());
    }
    if restart_pending() {
        return announce(
            session,
            &tr!("Restart to finish updating"),
            &tr!("Some updates take effect after a restart"),
        );
    }
    if stale::finish_in_session(session).logout {
        return announce_log_out(session);
    }
    let client = Client::background(system);
    if !check_due(session, &client) {
        return Ok(None);
    }
    let checked = check(system, &client, Refresh::Always);
    badge::remember(&checked);
    let SoftwareState::Available(packages) = checked else {
        return Ok(None);
    };
    if software_updates(session) == INSTALL {
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

fn download_in_background(client: &Client, packages: &[Package]) {
    let (flatpaks, debian): (Vec<&Package>, Vec<&Package>) = packages
        .iter()
        .filter(|package| !package.is_firmware())
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
    let firmware = packages
        .iter()
        .filter(|package| package.class == Class::Firmware(Applies::Now))
        .count();
    let (mut now, later): (Vec<Package>, Vec<Package>) = packages
        .into_iter()
        .filter(|package| package.class != Class::Firmware(Applies::Now))
        .partition(|package| matches!(package.class, Class::Flatpak | Class::Live));
    let debian: Vec<&Package> = now
        .iter()
        .filter(|package| package.class == Class::Live)
        .collect();
    let count = debian.len();
    let mut sorted = Sorted {
        now: Vec::new(),
        approve: firmware,
        restart: later.len(),
    };
    if pulls_offline(client, &ids(&debian)).unwrap_or(true) {
        sorted.restart += count;
    } else if !live {
        sorted.approve += count;
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
    let SoftwareState::Available(packages) = check(system, client, Refresh::WhenStale) else {
        return Ok(None);
    };
    let Sorted {
        now,
        approve,
        restart,
    } = sort_updates(client, packages, may_update_live(system));
    let installed = match now.is_empty() {
        true => None,
        false => Some(settle(session, update(system, client, &now, &|_| {}))),
    };
    if let Some(state) = &installed {
        badge::remember(state);
    }
    match installed {
        Some(SoftwareState::Failed { reason, .. }) => {
            announce(session, &tr!("Could not install updates"), &reason)
        }
        Some(SoftwareState::Prepared { count, ending }) => {
            announce(session, &ending.title(count), &ending.hint())
        }
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

fn firmware_missing(system: &Connection) -> bool {
    firmware::installed(system).map_or_else(
        |error| {
            tracing::warn!("cannot tell whether fwupd is installed: {error}");
            false
        },
        |installed| !installed,
    )
}

fn install_firmware_support(system: &Connection, client: &Client) -> SoftwareState {
    if let Err(error) = client.install_missing(&[FWUPD]) {
        return broken(Stage::Install, &error);
    }
    match firmware_missing(system) {
        true => SoftwareState::Failed {
            during: Stage::Install,
            reason: tr!("Firmware updates aren’t available from this computer’s software sources."),
        },
        false => check(system, client, Refresh::Always),
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
                    match waiting_offline(&system) {
                        Ok(Some(waiting)) => waiting,
                        Ok(None) => check(&system, &client, Refresh::Always),
                        Err(error) => broken(Stage::Check, &error),
                    }
                }
                SoftwareCommand::Update(packages) => {
                    updates.send(Update::Software(SoftwareState::Downloading {
                        percent: None,
                    }));
                    let state = update(&system, &client, &packages, &|state| {
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
                        Ok(()) => check(&system, &client, Refresh::WhenStale),
                        Err(error) => broken(Stage::Install, &error),
                    }
                }
                SoftwareCommand::InstallFirmwareSupport => {
                    updates.send(Update::Software(SoftwareState::Installing {
                        percent: None,
                    }));
                    install_firmware_support(&system, &client)
                }
            };
            badge::remember(&state);
            updates.send(Update::SoftwareChecked(last_check(&client)));
            updates.send(Update::FirmwareMissing(firmware_missing(&system)));
            if !updates.send(Update::Software(state)) {
                break;
            }
        }
    });
    sender
}
