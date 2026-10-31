use std::time::{Duration, Instant};

use crate::L10N;

pub const COUNTDOWN: Duration = Duration::from_secs(60);
pub const QUIT_PATIENCE: Duration = Duration::from_secs(3);

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

    pub fn blocked(self, app: &str) -> String {
        let app = [("app", app.to_owned().into())];
        match self {
            Self::Restart => L10N.tr_args("blocked-restart", app),
            Self::ShutDown => L10N.tr_args("blocked-shut-down", app),
            Self::LogOut => L10N.tr_args("blocked-log-out", app),
        }
    }

    pub fn anyway(self) -> String {
        L10N.tr(match self {
            Self::Restart => "anyway-restart",
            Self::ShutDown => "anyway-shut-down",
            Self::LogOut => "anyway-log-out",
        })
    }
}

pub fn quitting_apps() -> String {
    L10N.tr("quitting-apps")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    Asking {
        until: Instant,
    },
    Quitting {
        since: Instant,
        running: Option<String>,
    },
    Blocked {
        app_id: String,
    },
    Ending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    QuitApps,
    Activate(String),
    End,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEnding {
    pub ending: Ending,
    stage: Stage,
}

impl SessionEnding {
    pub fn new(ending: Ending, now: Instant) -> Self {
        Self {
            ending,
            stage: Stage::Asking {
                until: now + COUNTDOWN,
            },
        }
    }

    pub fn stage(&self) -> &Stage {
        &self.stage
    }

    pub fn confirm(&mut self, now: Instant) -> Option<Effect> {
        match self.stage {
            Stage::Asking { .. } => {
                self.stage = Stage::Quitting {
                    since: now,
                    running: None,
                };
                Some(Effect::QuitApps)
            }
            Stage::Quitting { .. } | Stage::Blocked { .. } => {
                self.stage = Stage::Ending;
                Some(Effect::End)
            }
            Stage::Ending => None,
        }
    }

    pub fn apps_running(&mut self, app_id: String, count: u32, now: Instant) -> Option<Effect> {
        match &mut self.stage {
            Stage::Quitting { .. } | Stage::Blocked { .. } if count == 0 => {
                self.stage = Stage::Ending;
                Some(Effect::End)
            }
            Stage::Quitting { running, .. } => {
                *running = Some(app_id);
                self.tick(now)
            }
            Stage::Blocked { app_id: blocking } if *blocking != app_id => {
                blocking.clone_from(&app_id);
                Some(Effect::Activate(app_id))
            }
            _ => None,
        }
    }

    pub fn tick(&mut self, now: Instant) -> Option<Effect> {
        match &self.stage {
            Stage::Asking { until } if *until <= now => self.confirm(now),
            Stage::Quitting {
                since,
                running: Some(app_id),
            } if *since + QUIT_PATIENCE <= now => {
                let app_id = app_id.clone();
                self.stage = Stage::Blocked {
                    app_id: app_id.clone(),
                };
                Some(Effect::Activate(app_id))
            }
            _ => None,
        }
    }

    pub fn left(&self, now: Instant) -> Duration {
        match self.stage {
            Stage::Asking { until } => until.saturating_duration_since(now),
            _ => Duration::ZERO,
        }
    }

    pub fn deadline(&self, now: Instant) -> Option<Instant> {
        match &self.stage {
            Stage::Asking { until } => {
                let left = until.saturating_duration_since(now);
                let part = left.subsec_nanos();
                Some(if part == 0 {
                    now + Duration::from_secs(1).min(left)
                } else {
                    now + Duration::from_nanos(u64::from(part))
                })
            }
            Stage::Quitting {
                since,
                running: Some(_),
            } => Some(*since + QUIT_PATIENCE),
            _ => None,
        }
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

pub fn full_name() -> String {
    let login = std::env::var("USER").unwrap_or_default();
    std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|passwd| {
            passwd.lines().find_map(|line| {
                let fields: Vec<&str> = line.split(':').collect();
                (fields.first() == Some(&login.as_str()))
                    .then(|| fields.get(4)?.split(',').next().map(str::to_owned))
                    .flatten()
                    .filter(|name| !name.is_empty())
            })
        })
        .unwrap_or(login)
}
