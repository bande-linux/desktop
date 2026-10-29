use crate::L10N;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneId {
    Appearance,
    Dock,
    LockScreen,
    Keyboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingId {
    Appearance,
    DockEdge,
    DimAfter,
    LockAfter,
    SleepAfter,
    KeyRepeatRate,
    KeyRepeatDelay,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Keyword(String),
    Seconds(u64),
    Number(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub label: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Choices(Vec<Choice>),
    Stops {
        stops: Vec<Choice>,
        low: String,
        high: String,
    },
}

impl Kind {
    pub fn choices(&self) -> &[Choice] {
        match self {
            Self::Choices(choices) | Self::Stops { stops: choices, .. } => choices,
        }
    }

    pub fn position(&self, value: &Value) -> Option<usize> {
        let choices = self.choices();
        match (self, value) {
            (Self::Stops { .. }, Value::Number(number)) => choices
                .iter()
                .enumerate()
                .min_by_key(|(_, choice)| match choice.value {
                    Value::Number(stop) => stop.abs_diff(*number),
                    _ => u32::MAX,
                })
                .map(|(index, _)| index),
            _ => choices.iter().position(|choice| choice.value == *value),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    pub id: SettingId,
    pub label: String,
    pub keywords: Vec<String>,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: Option<String>,
    pub settings: Vec<Setting>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pane {
    pub id: PaneId,
    pub name: String,
    pub keywords: Vec<String>,
    pub sections: Vec<Section>,
}

impl Pane {
    pub fn settings(&self) -> impl Iterator<Item = &Setting> {
        self.sections
            .iter()
            .flat_map(|section| section.settings.iter())
    }
}

fn words(list: &str) -> Vec<String> {
    list.split(',')
        .map(|keyword| keyword.trim().to_owned())
        .filter(|keyword| !keyword.is_empty())
        .collect()
}

fn setting(id: SettingId, label: String, keywords: String, kind: Kind) -> Setting {
    Setting {
        id,
        label,
        keywords: words(&keywords),
        kind,
    }
}

fn section(title: Option<String>, settings: Vec<Setting>) -> Section {
    Section { title, settings }
}

fn pane(id: PaneId, name: String, keywords: String, sections: Vec<Section>) -> Pane {
    Pane {
        id,
        name,
        keywords: words(&keywords),
        sections,
    }
}

fn keyword_choices(choices: Vec<(String, &str)>) -> Vec<Choice> {
    choices
        .into_iter()
        .map(|(label, keyword)| Choice {
            label,
            value: Value::Keyword(keyword.to_owned()),
        })
        .collect()
}

fn durations(choices: &[u64]) -> Vec<Choice> {
    choices
        .iter()
        .map(|&seconds| Choice {
            label: duration_label(seconds),
            value: Value::Seconds(seconds),
        })
        .collect()
}

pub const WITH_DISPLAY: &str = "with-display";

fn sleep_choices() -> Vec<Choice> {
    let mut choices = keyword_choices(vec![(L10N.tr("with-display"), WITH_DISPLAY)]);
    choices.extend(durations(&SLEEP_CHOICES));
    choices
}

pub fn duration_label(seconds: u64) -> String {
    match seconds {
        0 => L10N.tr("never"),
        1..60 => L10N.tr_args("seconds", [("count", seconds.into())]),
        _ if seconds.is_multiple_of(3600) => {
            L10N.tr_args("hours", [("count", (seconds / 3600).into())])
        }
        _ => L10N.tr_args("minutes", [("count", (seconds / 60).into())]),
    }
}

fn numbers(values: &[u32]) -> Vec<Choice> {
    values
        .iter()
        .map(|&number| Choice {
            label: number.to_string(),
            value: Value::Number(number),
        })
        .collect()
}

pub const DIM_CHOICES: [u64; 5] = [30, 60, 120, 300, 600];
pub const LOCK_CHOICES: [u64; 5] = [60, 300, 600, 1200, 3600];
pub const SLEEP_CHOICES: [u64; 4] = [1800, 3600, 10800, 0];
const REPEAT_RATES: [u32; 9] = [2, 5, 10, 15, 20, 25, 30, 45, 60];
const REPEAT_DELAYS: [u32; 8] = [2000, 1500, 1000, 800, 600, 450, 300, 200];

pub fn panes() -> Vec<Pane> {
    vec![
        pane(
            PaneId::Appearance,
            L10N.tr("appearance"),
            L10N.tr("appearance-keywords"),
            vec![section(
                None,
                vec![setting(
                    SettingId::Appearance,
                    L10N.tr("appearance-mode"),
                    L10N.tr("appearance-mode-keywords"),
                    Kind::Choices(keyword_choices(vec![
                        (L10N.tr("appearance-light"), "light"),
                        (L10N.tr("appearance-dark"), "dark"),
                        (L10N.tr("appearance-automatic"), "automatic"),
                    ])),
                )],
            )],
        ),
        pane(
            PaneId::Dock,
            L10N.tr("desktop-and-dock"),
            L10N.tr("desktop-and-dock-keywords"),
            vec![section(
                Some(L10N.tr("dock")),
                vec![setting(
                    SettingId::DockEdge,
                    L10N.tr("dock-position"),
                    L10N.tr("dock-position-keywords"),
                    Kind::Choices(keyword_choices(vec![
                        (L10N.tr("edge-left"), "left"),
                        (L10N.tr("edge-bottom"), "bottom"),
                        (L10N.tr("edge-right"), "right"),
                        (L10N.tr("edge-top"), "top"),
                    ])),
                )],
            )],
        ),
        pane(
            PaneId::LockScreen,
            L10N.tr("lock-screen"),
            L10N.tr("lock-screen-keywords"),
            vec![section(
                Some(L10N.tr("when-inactive")),
                vec![
                    setting(
                        SettingId::DimAfter,
                        L10N.tr("dim-after"),
                        L10N.tr("dim-after-keywords"),
                        Kind::Choices(durations(&DIM_CHOICES)),
                    ),
                    setting(
                        SettingId::LockAfter,
                        L10N.tr("lock-after"),
                        L10N.tr("lock-after-keywords"),
                        Kind::Choices(durations(&LOCK_CHOICES)),
                    ),
                    setting(
                        SettingId::SleepAfter,
                        L10N.tr("sleep-after"),
                        L10N.tr("sleep-after-keywords"),
                        Kind::Choices(sleep_choices()),
                    ),
                ],
            )],
        ),
        pane(
            PaneId::Keyboard,
            L10N.tr("keyboard"),
            L10N.tr("keyboard-keywords"),
            vec![section(
                Some(L10N.tr("key-repeat")),
                vec![
                    setting(
                        SettingId::KeyRepeatRate,
                        L10N.tr("repeat-rate"),
                        L10N.tr("repeat-rate-keywords"),
                        Kind::Stops {
                            stops: numbers(&REPEAT_RATES),
                            low: L10N.tr("slow"),
                            high: L10N.tr("fast"),
                        },
                    ),
                    setting(
                        SettingId::KeyRepeatDelay,
                        L10N.tr("repeat-delay"),
                        L10N.tr("repeat-delay-keywords"),
                        Kind::Stops {
                            stops: numbers(&REPEAT_DELAYS),
                            low: L10N.tr("long"),
                            high: L10N.tr("short"),
                        },
                    ),
                ],
            )],
        ),
    ]
}
