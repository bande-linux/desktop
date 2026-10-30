use crate::L10N;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneId {
    Wifi,
    Bluetooth,
    Notifications,
    Sound,
    Appearance,
    Wallpaper,
    Dock,
    Displays,
    LockScreen,
    Keyboard,
    TrackpadMouse,
    General,
    About,
    Storage,
    DateTime,
    LoginItems,
}

impl PaneId {
    const KEYWORDS: [(Self, &'static str); 16] = [
        (Self::Wifi, "wifi"),
        (Self::Bluetooth, "bluetooth"),
        (Self::Notifications, "notifications"),
        (Self::Sound, "sound"),
        (Self::Appearance, "appearance"),
        (Self::Wallpaper, "wallpaper"),
        (Self::Dock, "dock"),
        (Self::Displays, "displays"),
        (Self::LockScreen, "lock-screen"),
        (Self::Keyboard, "keyboard"),
        (Self::TrackpadMouse, "trackpad-mouse"),
        (Self::General, "general"),
        (Self::About, "about"),
        (Self::Storage, "storage"),
        (Self::DateTime, "date-time"),
        (Self::LoginItems, "login-items"),
    ];

    pub fn keyword(self) -> &'static str {
        Self::KEYWORDS
            .iter()
            .find(|(pane, _)| *pane == self)
            .map_or("", |(_, keyword)| keyword)
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::KEYWORDS
            .iter()
            .find(|(_, candidate)| *candidate == keyword)
            .map(|(pane, _)| *pane)
    }

    pub fn parent(self) -> Option<Self> {
        match self {
            Self::About | Self::Storage | Self::DateTime | Self::LoginItems => Some(Self::General),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingId {
    WifiPower,
    Networks,
    BluetoothPower,
    PairedDevices,
    NearbyDevices,
    DoNotDisturb,
    OutputVolume,
    OutputMuted,
    OutputDevice,
    InputVolume,
    InputMuted,
    InputDevice,
    Appearance,
    ReduceMotion,
    Wallpaper,
    WallpaperFit,
    DockEdge,
    DockIconSize,
    DockAutoHide,
    DisplayScale,
    RefreshRate,
    VariableRefreshRate,
    DimAfter,
    LockAfter,
    SleepAfter,
    KeyRepeatRate,
    KeyRepeatDelay,
    InputSources,
    CapsLock,
    Shortcuts,
    TrackpadTapToClick,
    TrackpadNaturalScrolling,
    TrackpadSpeed,
    TrackpadSecondaryClick,
    TrackpadDisableWhileTyping,
    MouseNaturalScrolling,
    MouseSpeed,
    ComputerName,
    AboutThisComputer,
    StorageUsage,
    LargeFiles,
    AutomaticTime,
    DateAndTime,
    TimeZone,
    Clock24Hour,
    LoginItems,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Keyword(String),
    Seconds(u64),
    Number(u32),
    Flag(bool),
    Fraction(f32),
    Pictures { light: String, dark: String },
    Codes(Vec<String>),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    Networks,
    PairedDevices,
    NearbyDevices,
    OutputDevices,
    InputDevices,
    InputSources,
    Shortcuts,
    About,
    Storage,
    LargeFiles,
    Clock,
    TimeZones,
    LoginItems,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub label: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Choices(Vec<Choice>),
    Menu(Vec<Choice>),
    Stops {
        stops: Vec<Choice>,
        low: String,
        high: String,
    },
    Switch,
    Level,
    Range {
        range: (u32, u32),
        step: u32,
        low: String,
        high: String,
    },
    RefreshRates,
    Pictures,
    Text,
    List(List),
}

impl Kind {
    pub fn choices(&self) -> &[Choice] {
        match self {
            Self::Choices(choices) | Self::Menu(choices) | Self::Stops { stops: choices, .. } => {
                choices
            }
            Self::Switch
            | Self::Level
            | Self::Range { .. }
            | Self::RefreshRates
            | Self::Pictures
            | Self::Text
            | Self::List(_) => &[],
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
pub const POINTER_SPEEDS: u32 = 10;

fn scale_choices() -> Vec<Choice> {
    let mut choices = keyword_choices(vec![(L10N.tr("scale-automatic"), AUTOMATIC)]);
    choices.extend(SCALES.iter().map(|&percent| Choice {
        label: L10N.tr_args("percent", [("value", percent.into())]),
        value: Value::Number(percent),
    }));
    choices
}

pub const AUTOMATIC: &str = "automatic";
const DOCK_SIZES: (u32, u32) = (24, 96);

const SCALES: [u32; 5] = [100, 125, 150, 175, 200];

fn connectivity_panes() -> Vec<Pane> {
    vec![
        pane(
            PaneId::Wifi,
            L10N.tr("wifi"),
            L10N.tr("wifi-keywords"),
            vec![
                section(
                    None,
                    vec![setting(
                        SettingId::WifiPower,
                        L10N.tr("wifi"),
                        L10N.tr("wifi-power-keywords"),
                        Kind::Switch,
                    )],
                ),
                section(
                    Some(L10N.tr("networks")),
                    vec![setting(
                        SettingId::Networks,
                        L10N.tr("networks"),
                        L10N.tr("networks-keywords"),
                        Kind::List(List::Networks),
                    )],
                ),
            ],
        ),
        pane(
            PaneId::Bluetooth,
            L10N.tr("bluetooth"),
            L10N.tr("bluetooth-keywords"),
            vec![
                section(
                    None,
                    vec![setting(
                        SettingId::BluetoothPower,
                        L10N.tr("bluetooth"),
                        L10N.tr("bluetooth-power-keywords"),
                        Kind::Switch,
                    )],
                ),
                section(
                    Some(L10N.tr("my-devices")),
                    vec![setting(
                        SettingId::PairedDevices,
                        L10N.tr("my-devices"),
                        L10N.tr("my-devices-keywords"),
                        Kind::List(List::PairedDevices),
                    )],
                ),
                section(
                    Some(L10N.tr("nearby-devices")),
                    vec![setting(
                        SettingId::NearbyDevices,
                        L10N.tr("nearby-devices"),
                        L10N.tr("nearby-devices-keywords"),
                        Kind::List(List::NearbyDevices),
                    )],
                ),
            ],
        ),
        pane(
            PaneId::Notifications,
            L10N.tr("notifications"),
            L10N.tr("notifications-keywords"),
            vec![section(
                None,
                vec![setting(
                    SettingId::DoNotDisturb,
                    L10N.tr("do-not-disturb"),
                    L10N.tr("do-not-disturb-keywords"),
                    Kind::Switch,
                )],
            )],
        ),
        pane(
            PaneId::Sound,
            L10N.tr("sound"),
            L10N.tr("sound-keywords"),
            vec![
                section(
                    Some(L10N.tr("output")),
                    vec![
                        setting(
                            SettingId::OutputVolume,
                            L10N.tr("output-volume"),
                            L10N.tr("output-volume-keywords"),
                            Kind::Level,
                        ),
                        setting(
                            SettingId::OutputMuted,
                            L10N.tr("mute"),
                            L10N.tr("mute-keywords"),
                            Kind::Switch,
                        ),
                    ],
                ),
                section(
                    Some(L10N.tr("output-device")),
                    vec![setting(
                        SettingId::OutputDevice,
                        L10N.tr("output-device"),
                        L10N.tr("output-device-keywords"),
                        Kind::List(List::OutputDevices),
                    )],
                ),
                section(
                    Some(L10N.tr("input")),
                    vec![
                        setting(
                            SettingId::InputVolume,
                            L10N.tr("input-volume"),
                            L10N.tr("input-volume-keywords"),
                            Kind::Level,
                        ),
                        setting(
                            SettingId::InputMuted,
                            L10N.tr("mute-input"),
                            L10N.tr("mute-input-keywords"),
                            Kind::Switch,
                        ),
                    ],
                ),
                section(
                    Some(L10N.tr("input-device")),
                    vec![setting(
                        SettingId::InputDevice,
                        L10N.tr("input-device"),
                        L10N.tr("input-device-keywords"),
                        Kind::List(List::InputDevices),
                    )],
                ),
            ],
        ),
    ]
}

fn displays_pane() -> Pane {
    pane(
        PaneId::Displays,
        L10N.tr("displays"),
        L10N.tr("displays-keywords"),
        vec![section(
            None,
            vec![
                setting(
                    SettingId::DisplayScale,
                    L10N.tr("display-scale"),
                    L10N.tr("display-scale-keywords"),
                    Kind::Choices(scale_choices()),
                ),
                setting(
                    SettingId::RefreshRate,
                    L10N.tr("refresh-rate"),
                    L10N.tr("refresh-rate-keywords"),
                    Kind::RefreshRates,
                ),
                setting(
                    SettingId::VariableRefreshRate,
                    L10N.tr("variable-refresh-rate"),
                    L10N.tr("variable-refresh-rate-keywords"),
                    Kind::Switch,
                ),
            ],
        )],
    )
}

fn general_panes() -> Vec<Pane> {
    vec![
        pane(
            PaneId::General,
            L10N.tr("general"),
            L10N.tr("general-keywords"),
            Vec::new(),
        ),
        pane(
            PaneId::About,
            L10N.tr("about"),
            L10N.tr("about-keywords"),
            vec![
                section(
                    None,
                    vec![setting(
                        SettingId::ComputerName,
                        L10N.tr("computer-name"),
                        L10N.tr("computer-name-keywords"),
                        Kind::Text,
                    )],
                ),
                section(
                    None,
                    vec![setting(
                        SettingId::AboutThisComputer,
                        L10N.tr("about"),
                        L10N.tr("about-this-computer-keywords"),
                        Kind::List(List::About),
                    )],
                ),
            ],
        ),
        pane(
            PaneId::Storage,
            L10N.tr("storage"),
            L10N.tr("storage-keywords"),
            vec![
                section(
                    None,
                    vec![setting(
                        SettingId::StorageUsage,
                        L10N.tr("storage"),
                        L10N.tr("storage-usage-keywords"),
                        Kind::List(List::Storage),
                    )],
                ),
                section(
                    Some(L10N.tr("large-files")),
                    vec![setting(
                        SettingId::LargeFiles,
                        L10N.tr("large-files"),
                        L10N.tr("large-files-keywords"),
                        Kind::List(List::LargeFiles),
                    )],
                ),
            ],
        ),
        date_time_pane(),
        pane(
            PaneId::LoginItems,
            L10N.tr("login-items"),
            L10N.tr("login-items-keywords"),
            vec![section(
                None,
                vec![setting(
                    SettingId::LoginItems,
                    L10N.tr("open-at-login"),
                    L10N.tr("open-at-login-keywords"),
                    Kind::List(List::LoginItems),
                )],
            )],
        ),
    ]
}

fn date_time_pane() -> Pane {
    pane(
        PaneId::DateTime,
        L10N.tr("date-time"),
        L10N.tr("date-time-keywords"),
        vec![
            section(
                None,
                vec![setting(
                    SettingId::AutomaticTime,
                    L10N.tr("automatic-time"),
                    L10N.tr("automatic-time-keywords"),
                    Kind::Switch,
                )],
            ),
            section(
                None,
                vec![setting(
                    SettingId::DateAndTime,
                    L10N.tr("date-and-time"),
                    L10N.tr("date-and-time-keywords"),
                    Kind::List(List::Clock),
                )],
            ),
            section(
                Some(L10N.tr("time-zone")),
                vec![setting(
                    SettingId::TimeZone,
                    L10N.tr("time-zone"),
                    L10N.tr("time-zone-keywords"),
                    Kind::List(List::TimeZones),
                )],
            ),
            section(
                Some(L10N.tr("clock")),
                vec![setting(
                    SettingId::Clock24Hour,
                    L10N.tr("clock-24-hour"),
                    L10N.tr("clock-24-hour-keywords"),
                    Kind::Switch,
                )],
            ),
        ],
    )
}

pub fn panes() -> Vec<Pane> {
    let mut panes = connectivity_panes();
    panes.extend(general_panes());
    panes.extend(desktop_panes());
    panes.push(keyboard_pane());
    panes.push(trackpad_mouse_pane());
    panes
}

fn desktop_panes() -> Vec<Pane> {
    vec![
        pane(
            PaneId::Appearance,
            L10N.tr("appearance"),
            L10N.tr("appearance-keywords"),
            vec![
                section(
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
                ),
                section(
                    Some(L10N.tr("motion")),
                    vec![setting(
                        SettingId::ReduceMotion,
                        L10N.tr("reduce-motion"),
                        L10N.tr("reduce-motion-keywords"),
                        Kind::Switch,
                    )],
                ),
            ],
        ),
        pane(
            PaneId::Wallpaper,
            L10N.tr("wallpaper"),
            L10N.tr("wallpaper-keywords"),
            vec![
                section(
                    None,
                    vec![setting(
                        SettingId::Wallpaper,
                        L10N.tr("wallpaper"),
                        L10N.tr("wallpaper-picture-keywords"),
                        Kind::Pictures,
                    )],
                ),
                section(
                    None,
                    vec![setting(
                        SettingId::WallpaperFit,
                        L10N.tr("wallpaper-fit"),
                        L10N.tr("wallpaper-fit-keywords"),
                        Kind::Choices(keyword_choices(vec![
                            (L10N.tr("wallpaper-fill"), "fill"),
                            (L10N.tr("wallpaper-fit-whole"), "fit"),
                        ])),
                    )],
                ),
            ],
        ),
        pane(
            PaneId::Dock,
            L10N.tr("dock"),
            L10N.tr("dock-keywords"),
            vec![section(
                None,
                vec![
                    setting(
                        SettingId::DockIconSize,
                        L10N.tr("dock-size"),
                        L10N.tr("dock-size-keywords"),
                        Kind::Range {
                            range: DOCK_SIZES,
                            step: 4,
                            low: L10N.tr("small"),
                            high: L10N.tr("large"),
                        },
                    ),
                    setting(
                        SettingId::DockEdge,
                        L10N.tr("dock-position"),
                        L10N.tr("dock-position-keywords"),
                        Kind::Choices(keyword_choices(vec![
                            (L10N.tr("edge-left"), "left"),
                            (L10N.tr("edge-bottom"), "bottom"),
                            (L10N.tr("edge-right"), "right"),
                            (L10N.tr("edge-top"), "top"),
                        ])),
                    ),
                    setting(
                        SettingId::DockAutoHide,
                        L10N.tr("dock-auto-hide"),
                        L10N.tr("dock-auto-hide-keywords"),
                        Kind::Switch,
                    ),
                ],
            )],
        ),
        displays_pane(),
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
    ]
}

pub fn pointer_speed_choices() -> Vec<Choice> {
    numbers(&(0..=POINTER_SPEEDS).collect::<Vec<_>>())
}

fn speed(id: SettingId, keywords: String) -> Setting {
    setting(
        id,
        L10N.tr("pointer-speed"),
        keywords,
        Kind::Stops {
            stops: pointer_speed_choices(),
            low: L10N.tr("slow"),
            high: L10N.tr("fast"),
        },
    )
}

fn keyboard_pane() -> Pane {
    pane(
        PaneId::Keyboard,
        L10N.tr("keyboard"),
        L10N.tr("keyboard-keywords"),
        vec![
            section(
                Some(L10N.tr("input-sources")),
                vec![setting(
                    SettingId::InputSources,
                    L10N.tr("input-sources"),
                    L10N.tr("input-sources-keywords"),
                    Kind::List(List::InputSources),
                )],
            ),
            section(
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
            ),
            section(
                Some(L10N.tr("modifier-keys")),
                vec![setting(
                    SettingId::CapsLock,
                    L10N.tr("caps-lock"),
                    L10N.tr("caps-lock-keywords"),
                    Kind::Menu(keyword_choices(vec![
                        (L10N.tr("caps-lock-caps-lock"), "caps-lock"),
                        (L10N.tr("caps-lock-escape"), "escape"),
                        (L10N.tr("caps-lock-control"), "control"),
                        (L10N.tr("caps-lock-compose"), "compose"),
                        (L10N.tr("caps-lock-off"), "off"),
                    ])),
                )],
            ),
            section(
                Some(L10N.tr("shortcuts")),
                vec![setting(
                    SettingId::Shortcuts,
                    L10N.tr("shortcuts"),
                    L10N.tr("shortcuts-keywords"),
                    Kind::List(List::Shortcuts),
                )],
            ),
        ],
    )
}

fn trackpad_mouse_pane() -> Pane {
    pane(
        PaneId::TrackpadMouse,
        L10N.tr("trackpad-mouse"),
        L10N.tr("trackpad-mouse-keywords"),
        vec![
            section(
                Some(L10N.tr("trackpad")),
                vec![
                    setting(
                        SettingId::TrackpadTapToClick,
                        L10N.tr("tap-to-click"),
                        L10N.tr("tap-to-click-keywords"),
                        Kind::Switch,
                    ),
                    setting(
                        SettingId::TrackpadSecondaryClick,
                        L10N.tr("secondary-click"),
                        L10N.tr("secondary-click-keywords"),
                        Kind::Choices(keyword_choices(vec![
                            (L10N.tr("secondary-click-two-fingers"), "two-fingers"),
                            (L10N.tr("secondary-click-corner"), "corner"),
                        ])),
                    ),
                    setting(
                        SettingId::TrackpadNaturalScrolling,
                        L10N.tr("natural-scrolling"),
                        L10N.tr("natural-scrolling-keywords"),
                        Kind::Switch,
                    ),
                    speed(SettingId::TrackpadSpeed, L10N.tr("trackpad-speed-keywords")),
                    setting(
                        SettingId::TrackpadDisableWhileTyping,
                        L10N.tr("disable-while-typing"),
                        L10N.tr("disable-while-typing-keywords"),
                        Kind::Switch,
                    ),
                ],
            ),
            section(
                Some(L10N.tr("mouse")),
                vec![
                    setting(
                        SettingId::MouseNaturalScrolling,
                        L10N.tr("natural-scrolling"),
                        L10N.tr("mouse-natural-scrolling-keywords"),
                        Kind::Switch,
                    ),
                    speed(SettingId::MouseSpeed, L10N.tr("mouse-speed-keywords")),
                ],
            ),
        ],
    )
}
