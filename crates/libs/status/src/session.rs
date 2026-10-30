use std::time::Duration;

use crate::L10N;

pub const COUNTDOWN: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    Restart,
    ShutDown,
    LogOut,
}

impl Ending {
    const KEYWORDS: [(Self, &'static str); 3] = [
        (Self::Restart, "restart"),
        (Self::ShutDown, "shut-down"),
        (Self::LogOut, "log-out"),
    ];

    pub fn keyword(self) -> &'static str {
        Self::KEYWORDS
            .iter()
            .find(|(ending, _)| *ending == self)
            .map_or("", |(_, keyword)| keyword)
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::KEYWORDS
            .iter()
            .find(|(_, found)| *found == keyword)
            .map(|(ending, _)| *ending)
    }

    pub fn question(self) -> String {
        L10N.tr(match self {
            Self::Restart => "confirm-restart",
            Self::ShutDown => "confirm-shut-down",
            Self::LogOut => "confirm-log-out",
        })
    }

    pub fn countdown(self, left: Duration) -> String {
        let seconds = [("seconds", left.as_secs().into())];
        match self {
            Self::Restart => L10N.tr_args("countdown-restart", seconds),
            Self::ShutDown => L10N.tr_args("countdown-shut-down", seconds),
            Self::LogOut => L10N.tr_args("countdown-log-out", seconds),
        }
    }

    pub fn button(self) -> String {
        L10N.tr(match self {
            Self::Restart => "button-restart",
            Self::ShutDown => "button-shut-down",
            Self::LogOut => "button-log-out",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub name: String,
    pub system: String,
    pub chip: String,
    pub memory: String,
}

fn quoted_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.strip_prefix('=')?;
        Some(value.trim().trim_matches('"'))
    })
}

fn colon_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name.trim() == key).then(|| value.trim())
    })
}

pub fn facts(
    hostname: &str,
    os_release: &str,
    model: Option<&str>,
    cpuinfo: &str,
    meminfo: &str,
) -> Facts {
    let system = quoted_value(os_release, "PRETTY_NAME")
        .or_else(|| quoted_value(os_release, "NAME"))
        .unwrap_or("Linux")
        .to_owned();
    let chip = model
        .map(|model| model.trim_end_matches('\0').trim().to_owned())
        .filter(|model| !model.is_empty())
        .or_else(|| colon_value(cpuinfo, "model name").map(str::to_owned))
        .unwrap_or_default();
    let kibibytes: u64 = colon_value(meminfo, "MemTotal")
        .and_then(|value| value.trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0);
    let gigabytes = (kibibytes as f64 / (1024.0 * 1024.0)).round() as u64;
    Facts {
        name: hostname.trim().to_owned(),
        system,
        chip,
        memory: L10N.tr_args("memory-gigabytes", [("gigabytes", gigabytes.into())]),
    }
}
