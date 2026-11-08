use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use bande_xdg::{replace_file, state_home};

use super::{CHECK_SLACK, Reboot, SoftwareState};
use crate::L10N;

const FILE: &str = "bande/software-badge";
const PACKAGES: &str = "/var/lib/dpkg/status";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Remembered {
    UpToDate,
    Available(usize),
    RestartToUpdate,
    Interrupted,
}

impl Remembered {
    fn of(state: &SoftwareState) -> Option<Self> {
        Some(match state {
            SoftwareState::UpToDate => Self::UpToDate,
            SoftwareState::Available(packages) if packages.is_empty() => Self::UpToDate,
            SoftwareState::Available(packages) => Self::Available(packages.len()),
            SoftwareState::Prepared(_)
            | SoftwareState::Installed {
                reboot: Some(Reboot::Install(_)),
                ..
            } => Self::RestartToUpdate,
            SoftwareState::Installed { .. } => Self::UpToDate,
            SoftwareState::Interrupted => Self::Interrupted,
            _ => return None,
        })
    }

    fn line(self, boot: &str) -> String {
        let (kind, count) = match self {
            Self::UpToDate => ("up-to-date", 0),
            Self::Available(count) => ("available", count),
            Self::RestartToUpdate => ("restart-to-update", 0),
            Self::Interrupted => ("interrupted", 0),
        };
        format!("{boot} {kind} {count}\n")
    }

    fn parse(line: &str) -> Option<(&str, Self)> {
        let mut words = line.split_whitespace();
        let boot = words.next()?;
        let kind = words.next()?;
        let count = words.next()?.parse().ok()?;
        let remembered = match kind {
            "up-to-date" => Self::UpToDate,
            "available" => Self::Available(count),
            "restart-to-update" => Self::RestartToUpdate,
            "interrupted" => Self::Interrupted,
            _ => return None,
        };
        Some((boot, remembered))
    }
}

pub(crate) enum Glance {
    Due,
    Known(Option<String>),
}

fn file() -> Option<PathBuf> {
    state_home().map(|state| state.join(FILE))
}

fn boot_id() -> String {
    super::boot_id()
        .map(|boot| boot.trim().to_owned())
        .unwrap_or_default()
}

fn modified(path: &str) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

pub(crate) fn remember(state: &SoftwareState) {
    let Some((remembered, file)) = Remembered::of(state).zip(file()) else {
        return;
    };
    if let Err(error) = replace_file(&file, remembered.line(&boot_id())) {
        tracing::warn!("cannot remember the update state: {error}");
    }
}

pub(crate) fn glance(every: Option<Duration>, restart_pending: bool) -> Glance {
    let Some(file) = file() else {
        return Glance::Due;
    };
    let Ok(text) = fs::read_to_string(&file) else {
        return Glance::Due;
    };
    let Some((boot, remembered)) = Remembered::parse(&text) else {
        return Glance::Due;
    };
    let checked = fs::metadata(&file).and_then(|meta| meta.modified()).ok();
    let age = checked.and_then(|checked| SystemTime::now().duration_since(checked).ok());
    if every
        .zip(age)
        .is_some_and(|(every, age)| age + CHECK_SLACK >= every)
    {
        return Glance::Due;
    }
    if restart_pending {
        return Glance::Known(Some(L10N.tr("restart-to-finish")));
    }
    let packages_changed = modified(PACKAGES)
        .zip(checked)
        .is_some_and(|(packages, checked)| packages > checked);
    Glance::Known(match remembered {
        Remembered::RestartToUpdate if boot == boot_id() => Some(L10N.tr("restart-to-update")),
        Remembered::Interrupted if !packages_changed => Some(L10N.tr("updates-interrupted")),
        Remembered::Available(count) if !packages_changed => {
            Some(L10N.tr_args("updates-available", [("count", count.into())]))
        }
        _ => None,
    })
}
