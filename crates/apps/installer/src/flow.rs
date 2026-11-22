use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use bande_packages::{Applied, Client, Error, ErrorCode, Info, Mode, Package, Progress};
use bande_xdg::DesktopEntry;

use crate::deb::{self, Blockers, Deb, Machine};

const INSTALLING: [Info; 3] = [Info::Installing, Info::Updating, Info::Downgrading];
const FRESH_LISTS: u32 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Install,
    Upgrade(String),
    Downgrade(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub id: String,
    pub origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub action: Action,
    pub signed: Option<Signed>,
    pub also: Vec<String>,
    pub download: u64,
    pub removes: Vec<String>,
    pub unrefreshed: Option<Unrefreshed>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    Locked,
    SystemPackage,
    Unavailable,
    NoNetwork,
    NoSpace,
    Copy,
    Dependencies,
    Install,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unrefreshed {
    Skipped,
    NoNetwork,
    NotAllowed,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Reading,
    Invalid,
    WrongArchitecture {
        arch: String,
        native: String,
    },
    Present,
    NeedsX11,
    Refreshing(Progress),
    Ready(Preview),
    Blocked {
        missing: Vec<String>,
        conflicts: Vec<String>,
        details: String,
        unrefreshed: Option<Unrefreshed>,
    },
    Installing(Progress),
    Installed {
        apps: Vec<DesktopEntry>,
        restart: bool,
    },
    ConfirmRemove(Vec<String>),
    Removing(Progress),
    Removed,
    Failed {
        problem: Problem,
        details: String,
    },
}

fn other_packages<'a>(
    applied: &'a Applied,
    own: &'a str,
    wanted: &'a [Info],
) -> impl Iterator<Item = &'a Package> {
    applied
        .packages
        .iter()
        .filter(move |package| package.name != own && wanted.contains(&package.info))
}

fn others(applied: &Applied, own: &str, wanted: &[Info]) -> Vec<String> {
    let mut names: Vec<String> = other_packages(applied, own, wanted)
        .map(|package| package.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn download_size(client: &Client, applied: &Applied, own: &str) -> u64 {
    let ids: Vec<String> = other_packages(applied, own, &INSTALLING)
        .map(|package| package.id.clone())
        .collect();
    if ids.is_empty() {
        return 0;
    }
    client
        .details(&ids)
        .map(|details| details.iter().map(|details| details.download).sum())
        .unwrap_or_else(|error| {
            tracing::warn!("cannot read the download size: {error}");
            0
        })
}

fn failed(error: &Error, attempted: Problem) -> State {
    let (problem, details) = match error {
        Error::Missing => (Problem::Unavailable, String::new()),
        Error::Vanished => (attempted, error.to_string()),
        Error::Bus(bus) => (attempted, bus.to_string()),
        Error::Failed(failure) => {
            let problem = match failure.code {
                ErrorCode::Locked => Problem::Locked,
                ErrorCode::SystemPackage => Problem::SystemPackage,
                ErrorCode::NoNetwork => Problem::NoNetwork,
                ErrorCode::NoSpace => Problem::NoSpace,
                ErrorCode::Dependencies => Problem::Dependencies,
                _ => attempted,
            };
            (problem, failure.details.clone())
        }
    };
    tracing::warn!("package installer: {error}");
    State::Failed { problem, details }
}

pub fn installed(client: &Client, deb: &Deb) -> Result<Option<Package>, Error> {
    Ok(client
        .resolve_installed(&deb.name)?
        .into_iter()
        .find(|package| package.arch == deb.arch || deb.arch == "all" || package.arch == "all"))
}

pub fn inspect(
    client: &Client,
    path: &Path,
    machine: &Machine,
    deb: Option<&Deb>,
    x11_accepted: bool,
    refreshing: &dyn Fn(Progress),
) -> State {
    let Some(deb) = deb else {
        return State::Invalid;
    };
    if !machine.fits(&deb.arch) {
        return State::WrongArchitecture {
            arch: deb.arch.clone(),
            native: machine.native.clone(),
        };
    }
    if deb.needs_x11 && !x11_accepted {
        return State::NeedsX11;
    }
    preview(client, path, deb, refreshing)
}

fn preview(client: &Client, path: &Path, deb: &Deb, refreshing: &dyn Fn(Progress)) -> State {
    let present = match installed(client, deb) {
        Ok(present) => present,
        Err(error) => return failed(&error, Problem::Install),
    };
    let action = match present {
        None => Action::Install,
        Some(package) => match deb::compare(&deb.version, &package.version) {
            Ordering::Equal => return State::Present,
            Ordering::Greater => Action::Upgrade(package.version),
            Ordering::Less => Action::Downgrade(package.version),
        },
    };
    let simulated = simulate(client, path, deb, &action);
    let needs_lists = match &simulated {
        State::Blocked { .. } => true,
        State::Ready(preview) => !preview.also.is_empty(),
        _ => false,
    };
    if !needs_lists || lists_fresh(client) {
        return simulated;
    }
    refreshing(Progress::default());
    match client.refresh(false, refreshing) {
        Ok(()) => simulate(client, path, deb, &action),
        Err(error) => {
            tracing::warn!("cannot refresh the package lists: {error}");
            not_refreshed(simulated, &error)
        }
    }
}

fn simulate(client: &Client, path: &Path, deb: &Deb, action: &Action) -> State {
    match client
        .untranslated()
        .install_files(path, Mode::Simulate, &|_| {})
    {
        Ok(applied) => State::Ready(Preview {
            signed: match action {
                Action::Downgrade(_) => None,
                Action::Install | Action::Upgrade(_) => signed(client, deb),
            },
            action: action.clone(),
            also: others(&applied, &deb.name, &INSTALLING),
            download: download_size(client, &applied, &deb.name),
            removes: others(&applied, &deb.name, &[Info::Removing, Info::Obsoleting]),
            unrefreshed: None,
        }),
        Err(Error::Failed(failure)) if failure.code == ErrorCode::Dependencies => {
            let Blockers { missing, conflicts } = deb::blockers(&failure.details, &deb.name);
            State::Blocked {
                missing,
                conflicts,
                details: failure.details,
                unrefreshed: None,
            }
        }
        Err(error) => failed(&error, Problem::Install),
    }
}

fn not_refreshed(simulated: State, error: &Error) -> State {
    let (code, reason) = match error {
        Error::Failed(failure) => (Some(failure.code), failure.details.clone()),
        other => (None, other.to_string()),
    };
    let unrefreshed = match code {
        Some(ErrorCode::Cancelled) => Unrefreshed::Skipped,
        Some(ErrorCode::NoNetwork) => Unrefreshed::NoNetwork,
        Some(ErrorCode::NotAuthorized) => Unrefreshed::NotAllowed,
        _ => Unrefreshed::Failed,
    };
    match simulated {
        State::Ready(preview) => State::Ready(Preview {
            unrefreshed: Some(unrefreshed),
            ..preview
        }),
        State::Blocked {
            missing,
            conflicts,
            details,
            ..
        } => State::Blocked {
            missing,
            conflicts,
            details: match unrefreshed {
                Unrefreshed::Skipped => details,
                _ => format!("{details}\n\n{reason}"),
            },
            unrefreshed: Some(unrefreshed),
        },
        other => other,
    }
}

fn lists_fresh(client: &Client) -> bool {
    match client.seconds_since_refresh() {
        Ok(age) => age < FRESH_LISTS,
        Err(error) => {
            tracing::warn!("cannot read when the package lists were refreshed: {error}");
            false
        }
    }
}

pub fn apps(files: &[String]) -> Vec<DesktopEntry> {
    files
        .iter()
        .map(PathBuf::from)
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "desktop")
                && path
                    .parent()
                    .is_some_and(|parent| parent.ends_with("share/applications"))
        })
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let id = path.file_name()?.to_string_lossy().into_owned();
            DesktopEntry::parse(&id, &path, &text, None, &bande_xdg::current_desktops())
        })
        .collect()
}

pub enum Outcome {
    Done(State),
    Back,
    Refused,
}

fn interrupted(error: &Error, attempted: Problem) -> Outcome {
    match error {
        Error::Failed(failure) if failure.code == ErrorCode::Cancelled => Outcome::Back,
        Error::Failed(failure) if failure.code == ErrorCode::NotAuthorized => Outcome::Refused,
        _ => Outcome::Done(failed(error, attempted)),
    }
}

fn signed(client: &Client, deb: &Deb) -> Option<Signed> {
    let found = client
        .resolve(std::slice::from_ref(&deb.name))
        .inspect_err(|error| tracing::warn!("cannot look in the software sources: {error}"))
        .ok()?;
    found
        .into_iter()
        .find(|package| {
            package.name == deb.name
                && package.arch == deb.arch
                && package.version == deb.version
                && package.info != Info::Installed
        })
        .map(|package| Signed {
            origin: package
                .origin
                .rsplit(':')
                .next()
                .unwrap_or_default()
                .to_owned(),
            id: package.id,
        })
}

pub fn install(
    client: &Client,
    path: &Path,
    signed: Option<&str>,
    progress: &dyn Fn(Progress),
) -> Outcome {
    let applied = match signed {
        Some(id) => client.install(&[id.to_owned()], Mode::Apply, progress),
        None => client.install_files(path, Mode::Apply, progress),
    };
    match applied {
        Ok(applied) => {
            let files = client.files_local(path).unwrap_or_else(|error| {
                tracing::warn!("cannot list the installed files: {error}");
                Vec::new()
            });
            Outcome::Done(State::Installed {
                apps: apps(&files),
                restart: applied.restart,
            })
        }
        Err(error) => interrupted(&error, Problem::Install),
    }
}

fn installed_ids(client: &Client, deb: &Deb) -> Result<Vec<String>, Error> {
    Ok(installed(client, deb)?
        .map(|package| vec![package.id])
        .unwrap_or_default())
}

pub fn removal(client: &Client, deb: &Deb) -> State {
    let simulated =
        installed_ids(client, deb).and_then(|ids| client.remove(&ids, Mode::Simulate, &|_| {}));
    match simulated {
        Ok(applied) => State::ConfirmRemove(others(
            &applied,
            &deb.name,
            &[Info::Removing, Info::Obsoleting],
        )),
        Err(error) => failed(&error, Problem::Remove),
    }
}

pub fn remove(client: &Client, deb: &Deb, progress: &dyn Fn(Progress)) -> Outcome {
    let removed =
        installed_ids(client, deb).and_then(|ids| client.remove(&ids, Mode::Apply, progress));
    match removed {
        Ok(_) => Outcome::Done(State::Removed),
        Err(error) => interrupted(&error, Problem::Remove),
    }
}
