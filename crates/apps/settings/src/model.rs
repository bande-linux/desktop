use crate::L10N;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Appearance,
    DockPosition,
    DimAfter,
    LockAfter,
    SleepAfter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Keyword(String),
    Seconds(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub label: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    pub key: Key,
    pub label: String,
    pub keywords: String,
    pub choices: Vec<Choice>,
}

impl Control {
    pub fn position(&self, value: &Value) -> Option<usize> {
        self.choices
            .iter()
            .position(|choice| choice.value == *value)
    }

    fn haystack(&self, pane: &str) -> String {
        let mut haystack = format!("{pane} {} {}", self.label, self.keywords);
        for choice in &self.choices {
            haystack.push(' ');
            haystack.push_str(&choice.label);
        }
        haystack.to_lowercase()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    pub name: String,
    pub controls: Vec<Control>,
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

fn duration_label(seconds: u64) -> String {
    match seconds {
        0 => L10N.tr("never"),
        1..60 => L10N.tr_args("seconds", [("count", seconds.into())]),
        _ if seconds.is_multiple_of(3600) => {
            L10N.tr_args("hours", [("count", (seconds / 3600).into())])
        }
        _ => L10N.tr_args("minutes", [("count", (seconds / 60).into())]),
    }
}

pub fn panes() -> Vec<Pane> {
    vec![
        Pane {
            name: L10N.tr("appearance"),
            controls: vec![Control {
                key: Key::Appearance,
                label: L10N.tr("appearance"),
                keywords: L10N.tr("appearance-keywords"),
                choices: keyword_choices(vec![
                    (L10N.tr("appearance-light"), "light"),
                    (L10N.tr("appearance-dark"), "dark"),
                    (L10N.tr("appearance-automatic"), "automatic"),
                ]),
            }],
        },
        Pane {
            name: L10N.tr("desktop-and-dock"),
            controls: vec![Control {
                key: Key::DockPosition,
                label: L10N.tr("dock-position"),
                keywords: L10N.tr("dock-position-keywords"),
                choices: keyword_choices(vec![
                    (L10N.tr("edge-left"), "left"),
                    (L10N.tr("edge-bottom"), "bottom"),
                    (L10N.tr("edge-right"), "right"),
                    (L10N.tr("edge-top"), "top"),
                ]),
            }],
        },
        Pane {
            name: L10N.tr("lock-screen"),
            controls: vec![
                Control {
                    key: Key::DimAfter,
                    label: L10N.tr("dim-after"),
                    keywords: L10N.tr("dim-after-keywords"),
                    choices: durations(&[30, 60, 120, 300, 600]),
                },
                Control {
                    key: Key::LockAfter,
                    label: L10N.tr("lock-after"),
                    keywords: L10N.tr("lock-after-keywords"),
                    choices: durations(&[60, 300, 600, 1200, 3600]),
                },
                Control {
                    key: Key::SleepAfter,
                    label: L10N.tr("sleep-after"),
                    keywords: L10N.tr("sleep-after-keywords"),
                    choices: durations(&[600, 1800, 3600, 10800, 0]),
                },
            ],
        },
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub pane: usize,
    pub controls: Vec<usize>,
}

pub fn search(panes: &[Pane], query: &str) -> Vec<Match> {
    let words = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    panes
        .iter()
        .enumerate()
        .filter_map(|(pane_index, pane)| {
            let controls = pane
                .controls
                .iter()
                .enumerate()
                .filter(|(_, control)| {
                    let haystack = control.haystack(&pane.name);
                    words.iter().all(|word| haystack.contains(word.as_str()))
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            (!controls.is_empty()).then_some(Match {
                pane: pane_index,
                controls,
            })
        })
        .collect()
}
