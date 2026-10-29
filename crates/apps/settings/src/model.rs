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
    pub label: &'static str,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    pub key: Key,
    pub label: &'static str,
    pub keywords: &'static [&'static str],
    pub choices: Vec<Choice>,
}

impl Control {
    pub fn position(&self, value: &Value) -> Option<usize> {
        self.choices
            .iter()
            .position(|choice| choice.value == *value)
    }

    fn haystack(&self, pane: &str) -> String {
        let mut haystack = format!("{pane} {} {}", self.label, self.keywords.join(" "));
        for choice in &self.choices {
            haystack.push(' ');
            haystack.push_str(choice.label);
        }
        haystack.to_lowercase()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    pub name: &'static str,
    pub controls: Vec<Control>,
}

fn keyword_choices(choices: &[(&'static str, &str)]) -> Vec<Choice> {
    choices
        .iter()
        .map(|(label, keyword)| Choice {
            label,
            value: Value::Keyword((*keyword).to_owned()),
        })
        .collect()
}

fn minutes(choices: &[(&'static str, u64)]) -> Vec<Choice> {
    choices
        .iter()
        .map(|(label, seconds)| Choice {
            label,
            value: Value::Seconds(*seconds),
        })
        .collect()
}

pub fn panes() -> Vec<Pane> {
    vec![
        Pane {
            name: "Appearance",
            controls: vec![Control {
                key: Key::Appearance,
                label: "Appearance",
                keywords: &["theme", "dark mode", "light mode", "colour", "color"],
                choices: keyword_choices(&[
                    ("Light", "light"),
                    ("Dark", "dark"),
                    ("Auto", "automatic"),
                ]),
            }],
        },
        Pane {
            name: "Desktop & Dock",
            controls: vec![Control {
                key: Key::DockPosition,
                label: "Position on screen",
                keywords: &["dock", "panel", "taskbar", "edge"],
                choices: keyword_choices(&[
                    ("Left", "left"),
                    ("Bottom", "bottom"),
                    ("Right", "right"),
                    ("Top", "top"),
                ]),
            }],
        },
        Pane {
            name: "Lock Screen",
            controls: vec![
                Control {
                    key: Key::DimAfter,
                    label: "Dim the display when inactive",
                    keywords: &["idle", "brightness", "screen"],
                    choices: minutes(&[
                        ("30 seconds", 30),
                        ("1 minute", 60),
                        ("2 minutes", 120),
                        ("5 minutes", 300),
                        ("10 minutes", 600),
                    ]),
                },
                Control {
                    key: Key::LockAfter,
                    label: "Turn display off and lock when inactive",
                    keywords: &["idle", "password", "screen saver", "security"],
                    choices: minutes(&[
                        ("1 minute", 60),
                        ("5 minutes", 300),
                        ("10 minutes", 600),
                        ("20 minutes", 1200),
                        ("1 hour", 3600),
                    ]),
                },
                Control {
                    key: Key::SleepAfter,
                    label: "Sleep when inactive",
                    keywords: &["idle", "suspend", "energy", "battery", "power"],
                    choices: minutes(&[
                        ("10 minutes", 600),
                        ("30 minutes", 1800),
                        ("1 hour", 3600),
                        ("3 hours", 10800),
                        ("Never", 0),
                    ]),
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
                    let haystack = control.haystack(pane.name);
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
