use crate::L10N;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Connections,
    Desktop,
    Devices,
    System,
}

impl Group {
    pub fn label(self) -> String {
        L10N.tr(match self {
            Self::Connections => "group-connections",
            Self::Desktop => "group-desktop",
            Self::Devices => "group-devices",
            Self::System => "group-system",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneId {
    Wifi,
    Bluetooth,
    Network,
    Appearance,
    Bar,
    Notifications,
    Search,
    Accessibility,
    Displays,
    NightLight,
    Sound,
    Keyboard,
    KeyboardShortcuts,
    Mouse,
    Trackpad,
    Printers,
    Power,
    PrivacySecurity,
    Camera,
    Microphone,
    Screenshots,
    ScreenSharing,
    Location,
    Apps,
    LanguageTime,
    About,
    Storage,
    SoftwareUpdate,
}

impl PaneId {
    const KEYWORDS: [(Self, &'static str); 28] = [
        (Self::Wifi, "wifi"),
        (Self::Bluetooth, "bluetooth"),
        (Self::Network, "network"),
        (Self::Appearance, "appearance"),
        (Self::Bar, "bar"),
        (Self::Notifications, "notifications"),
        (Self::Search, "search"),
        (Self::Accessibility, "accessibility"),
        (Self::Displays, "displays"),
        (Self::NightLight, "night-light"),
        (Self::Sound, "sound"),
        (Self::Keyboard, "keyboard"),
        (Self::KeyboardShortcuts, "keyboard-shortcuts"),
        (Self::Mouse, "mouse"),
        (Self::Trackpad, "trackpad"),
        (Self::Printers, "printers"),
        (Self::Power, "power"),
        (Self::PrivacySecurity, "privacy-security"),
        (Self::Camera, "camera"),
        (Self::Microphone, "microphone"),
        (Self::Screenshots, "screenshots"),
        (Self::ScreenSharing, "screen-sharing"),
        (Self::Location, "location"),
        (Self::Apps, "apps"),
        (Self::LanguageTime, "language-time"),
        (Self::About, "about"),
        (Self::Storage, "storage"),
        (Self::SoftwareUpdate, "software-update"),
    ];

    pub fn group(self) -> Option<Group> {
        match self {
            Self::Wifi | Self::Bluetooth | Self::Network => Some(Group::Connections),
            Self::Appearance
            | Self::Bar
            | Self::Notifications
            | Self::Search
            | Self::Accessibility => Some(Group::Desktop),
            Self::Displays
            | Self::Sound
            | Self::Keyboard
            | Self::Mouse
            | Self::Trackpad
            | Self::Printers => Some(Group::Devices),
            Self::Power | Self::PrivacySecurity | Self::Apps | Self::LanguageTime | Self::About => {
                Some(Group::System)
            }
            Self::KeyboardShortcuts
            | Self::NightLight
            | Self::Camera
            | Self::Microphone
            | Self::Screenshots
            | Self::ScreenSharing
            | Self::Location
            | Self::Storage
            | Self::SoftwareUpdate => None,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Wifi => "network-wireless-symbolic",
            Self::Bluetooth => "bluetooth-symbolic",
            Self::Network => "network-wired-symbolic",
            Self::Appearance => "weather-clear-night-symbolic",
            Self::Bar => "view-sidebar-start-symbolic",
            Self::Notifications => "preferences-system-notifications-symbolic",
            Self::Search => "system-search-symbolic",
            Self::Accessibility => "avatar-default-symbolic",
            Self::Displays => "video-display-symbolic",
            Self::NightLight => "night-light-symbolic",
            Self::Sound => "audio-volume-high-symbolic",
            Self::Keyboard | Self::KeyboardShortcuts => "input-keyboard-symbolic",
            Self::Mouse => "input-mouse-symbolic",
            Self::Trackpad => "input-touchpad-symbolic",
            Self::Printers => "printer-symbolic",
            Self::Power => "system-shutdown-symbolic",
            Self::PrivacySecurity => "changes-prevent-symbolic",
            Self::Camera => "camera-photo-symbolic",
            Self::Microphone => "audio-input-microphone-symbolic",
            Self::Screenshots => "screenshot-recorded-symbolic",
            Self::ScreenSharing => "emblem-shared-symbolic",
            Self::Location => "find-location-symbolic",
            Self::Apps => "view-app-grid-symbolic",
            Self::LanguageTime => "preferences-system-symbolic",
            Self::About => "computer-symbolic",
            Self::Storage => "drive-harddisk-symbolic",
            Self::SoftwareUpdate => "view-refresh-symbolic",
        }
    }

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
            Self::Storage | Self::SoftwareUpdate => Some(Self::About),
            Self::KeyboardShortcuts => Some(Self::Keyboard),
            Self::NightLight => Some(Self::Displays),
            Self::Camera
            | Self::Microphone
            | Self::Screenshots
            | Self::ScreenSharing
            | Self::Location => Some(Self::PrivacySecurity),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingId {
    WifiPower,
    KnownNetworks,
    Networks,
    WifiAddress,
    WifiRouter,
    WifiDns,
    OtherNetwork,
    BluetoothPower,
    PairedDevices,
    NearbyDevices,
    DoNotDisturb,
    FocusScheduled,
    FocusFrom,
    FocusTo,
    OutputVolume,
    OutputMuted,
    OutputDevice,
    InputVolume,
    InputMuted,
    InputDevice,
    InputLevel,
    AppVolumes,
    AlertSound,
    AlertVolume,
    SoundEffects,
    VolumeFeedback,
    Appearance,
    ScreenReader,
    Zoom,
    IncreaseContrast,
    ReduceTransparency,
    ReduceMotion,
    CursorSize,
    FlashScreen,
    StickyKeys,
    SlowKeys,
    BounceKeys,
    Accent,
    TintedFrame,
    Wallpaper,
    WallpaperFit,
    BarEdge,
    BarIconSize,
    BarAutoHide,
    BarLayout,
    DisplayUse,
    DisplayResolution,
    DisplayScale,
    RefreshRate,
    DisplayRotation,
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
    TrackpadWorkspaceSwipe,
    TrackpadOverviewSwipe,
    TrackpadDesktopSpread,
    TrackpadLauncherPinch,
    TrackpadAcceleration,
    TrackpadDragging,
    TrackpadScrollMethod,
    MousePrimaryButton,
    MouseNaturalScrolling,
    MouseSpeed,
    MouseAcceleration,
    ComputerName,
    AboutThisComputer,
    StorageUsage,
    LargeFiles,
    AutomaticTime,
    DateAndTime,
    TimeZone,
    Clock24Hour,
    LoginItems,
    ReopenApps,
    ReopenAfterCrash,
    BatteryLevel,
    PowerMode,
    LowPowerOnBattery,
    LidOnPower,
    AppNotifications,
    SoftwareUpdates,
    Ethernet,
    EthernetAddress,
    EthernetRouter,
    EthernetDns,
    Vpns,
    Language,
    Region,
    StoragePage,
    SoftwareUpdatePage,
    ShortcutsPage,
    ScreenLock,
    ClipboardHistory,
    DefaultApps,
    Brightness,
    NightLightPage,
    NightLight,
    NightSchedule,
    NightFrom,
    NightTo,
    NightWarmth,
    TextSize,
    TextSizeLink,
    CameraPage,
    MicrophonePage,
    ScreenshotsPage,
    ScreenSharingPage,
    LocationPage,
    AppPermissions,
    SearchApplications,
    SearchRecentApps,
    SearchRecentDocuments,
    SearchDocuments,
    SearchFolders,
    SearchSettings,
    SearchSystem,
    SearchCalculator,
    SearchConversion,
    SearchExcluded,
    Printers,
    NearbyPrinters,
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
    Printers,
    NearbyPrinters,
    KnownNetworks,
    Networks,
    PairedDevices,
    NearbyDevices,
    OutputDevices,
    InputDevices,
    InputLevel,
    AppVolumes,
    InputSources,
    Shortcuts,
    About,
    Storage,
    LargeFiles,
    Clock,
    TimeZones,
    LoginItems,
    BatteryHistory,
    NotificationApps,
    SoftwareUpdates,
    Ethernet,
    Vpns,
    Languages,
    Regions,
    DefaultApps,
    Clipboard,
    SearchExcluded,
    Permissions(Permission),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    Camera,
    Microphone,
    Screenshots,
    ScreenSharing,
    Location,
}

impl Permission {
    pub const ALL: [Self; 5] = [
        Self::Camera,
        Self::Microphone,
        Self::Screenshots,
        Self::ScreenSharing,
        Self::Location,
    ];

    pub fn pane(self) -> PaneId {
        match self {
            Self::Camera => PaneId::Camera,
            Self::Microphone => PaneId::Microphone,
            Self::Screenshots => PaneId::Screenshots,
            Self::ScreenSharing => PaneId::ScreenSharing,
            Self::Location => PaneId::Location,
        }
    }

    pub fn of(pane: PaneId) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|permission| permission.pane() == pane)
    }
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
    Resolutions,
    DisplayUses,
    Pictures,
    Swatches(Vec<Choice>),
    Text,
    List(List),
    Page(PaneId),
}

impl Kind {
    pub fn choices(&self) -> &[Choice] {
        match self {
            Self::Choices(choices)
            | Self::Menu(choices)
            | Self::Swatches(choices)
            | Self::Stops { stops: choices, .. } => choices,
            Self::Switch
            | Self::Level
            | Self::Range { .. }
            | Self::RefreshRates
            | Self::Resolutions
            | Self::DisplayUses
            | Self::Pictures
            | Self::Text
            | Self::List(_)
            | Self::Page(_) => &[],
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
    pub description: Option<String>,
    pub kind: Kind,
}

impl Setting {
    fn described(mut self, description: &str) -> Self {
        self.description = Some(L10N.tr(description));
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: Option<String>,
    pub settings: Vec<Setting>,
    pub note: Option<String>,
}

impl Section {
    fn noted(mut self, note: &str) -> Self {
        self.note = Some(L10N.tr(note));
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pane {
    pub id: PaneId,
    pub name: String,
    pub summary: String,
    pub keywords: Vec<String>,
    pub master: Option<SettingId>,
    pub sections: Vec<Section>,
}

impl Pane {
    pub fn settings(&self) -> impl Iterator<Item = &Setting> {
        self.sections
            .iter()
            .flat_map(|section| section.settings.iter())
    }

    fn with_master(mut self, master: SettingId) -> Self {
        self.master = Some(master);
        self
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
        description: None,
        kind,
    }
}

fn section(title: Option<String>, settings: Vec<Setting>) -> Section {
    Section {
        title,
        settings,
        note: None,
    }
}

fn pane(id: PaneId, sections: Vec<Section>) -> Pane {
    let name = id.keyword();
    Pane {
        id,
        name: L10N.tr(name),
        summary: L10N.tr(&format!("{name}-summary")),
        keywords: words(&L10N.tr(&format!("{name}-keywords"))),
        master: None,
        sections,
    }
}

pub const MAX_SEGMENTS: usize = 4;

fn choice(choices: Vec<Choice>) -> Kind {
    if choices.len() > MAX_SEGMENTS {
        Kind::Menu(choices)
    } else {
        Kind::Choices(choices)
    }
}

fn page(id: SettingId, pane: PaneId) -> Setting {
    let name = pane.keyword();
    setting(
        id,
        L10N.tr(name),
        L10N.tr(&format!("{name}-keywords")),
        Kind::Page(pane),
    )
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
pub const CURSOR_SIZES: [u32; 5] = [24, 32, 48, 64, 96];
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

fn rotation_choices() -> Vec<Choice> {
    std::iter::once(Choice {
        label: L10N.tr("rotation-standard"),
        value: Value::Number(0),
    })
    .chain([90, 180, 270].into_iter().map(|degrees: u32| Choice {
        label: L10N.tr_args("degrees", [("value", degrees.into())]),
        value: Value::Number(degrees),
    }))
    .collect()
}

pub const AUTOMATIC: &str = "automatic";

const BAR_SIZES: (u32, u32) = (32, 56);
const NIGHT_WARMTH: (u32, u32) = (0, 100);
pub const TEXT_SIZES: [u32; 5] = [100, 125, 150, 175, 200];

const SCALES: [u32; 5] = [100, 125, 150, 175, 200];

fn list(id: SettingId, name: &str, list: List) -> Setting {
    setting(
        id,
        L10N.tr(name),
        L10N.tr(&format!("{name}-keywords")),
        Kind::List(list),
    )
}

fn switch(id: SettingId, name: &str) -> Setting {
    setting(
        id,
        L10N.tr(name),
        L10N.tr(&format!("{name}-keywords")),
        Kind::Switch,
    )
}

fn level(id: SettingId, name: &str) -> Setting {
    setting(
        id,
        L10N.tr(name),
        L10N.tr(&format!("{name}-keywords")),
        Kind::Level,
    )
}

fn connection_panes() -> Vec<Pane> {
    vec![
        pane(
            PaneId::Wifi,
            vec![
                section(None, vec![switch(SettingId::WifiPower, "wifi-power")]),
                section(
                    Some(L10N.tr("known-networks")),
                    vec![list(
                        SettingId::KnownNetworks,
                        "known-networks",
                        List::KnownNetworks,
                    )],
                ),
                section(
                    Some(L10N.tr("networks")),
                    vec![list(SettingId::Networks, "networks", List::Networks)],
                ),
            ],
        )
        .with_master(SettingId::WifiPower),
        pane(
            PaneId::Bluetooth,
            vec![
                section(
                    None,
                    vec![switch(SettingId::BluetoothPower, "bluetooth-power")],
                ),
                section(
                    Some(L10N.tr("my-devices")),
                    vec![list(
                        SettingId::PairedDevices,
                        "my-devices",
                        List::PairedDevices,
                    )],
                ),
                section(
                    Some(L10N.tr("nearby-devices")),
                    vec![list(
                        SettingId::NearbyDevices,
                        "nearby-devices",
                        List::NearbyDevices,
                    )],
                ),
            ],
        )
        .with_master(SettingId::BluetoothPower),
        pane(
            PaneId::Network,
            vec![
                section(
                    Some(L10N.tr("ethernet")),
                    vec![list(SettingId::Ethernet, "ethernet", List::Ethernet)],
                ),
                section(
                    Some(L10N.tr("vpn")),
                    vec![list(SettingId::Vpns, "vpn", List::Vpns)],
                ),
            ],
        ),
    ]
}

fn desktop_panes() -> Vec<Pane> {
    vec![
        pane(
            PaneId::Appearance,
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
                    None,
                    vec![setting(
                        SettingId::Accent,
                        L10N.tr("accent"),
                        L10N.tr("accent-keywords"),
                        Kind::Swatches(keyword_choices(vec![
                            (L10N.tr("accent-ultramarine"), "ultramarine"),
                            (L10N.tr("accent-teal"), "teal"),
                            (L10N.tr("accent-moss"), "moss"),
                            (L10N.tr("accent-amber"), "amber"),
                            (L10N.tr("accent-ember"), "ember"),
                            (L10N.tr("accent-rose"), "rose"),
                            (L10N.tr("accent-plum"), "plum"),
                            (L10N.tr("accent-graphite"), "graphite"),
                        ])),
                    )],
                ),
                section(
                    None,
                    vec![setting(
                        SettingId::TextSize,
                        L10N.tr("text-size"),
                        L10N.tr("text-size-keywords"),
                        Kind::Stops {
                            stops: numbers(&TEXT_SIZES),
                            low: L10N.tr("text-size-default"),
                            high: L10N.tr("text-size-largest"),
                        },
                    )],
                ),
                section(
                    Some(L10N.tr("wallpaper")),
                    vec![
                        setting(
                            SettingId::Wallpaper,
                            L10N.tr("wallpaper"),
                            L10N.tr("wallpaper-picture-keywords"),
                            Kind::Pictures,
                        ),
                        setting(
                            SettingId::WallpaperFit,
                            L10N.tr("wallpaper-fit"),
                            L10N.tr("wallpaper-fit-keywords"),
                            Kind::Choices(keyword_choices(vec![
                                (L10N.tr("wallpaper-fill"), "fill"),
                                (L10N.tr("wallpaper-fit-whole"), "fit"),
                            ])),
                        ),
                        switch(SettingId::TintedFrame, "tinted-frame"),
                    ],
                ),
            ],
        ),
        pane(
            PaneId::Bar,
            vec![section(
                None,
                vec![
                    setting(
                        SettingId::BarLayout,
                        L10N.tr("bar-layout"),
                        L10N.tr("bar-layout-keywords"),
                        Kind::Choices(keyword_choices(vec![
                            (L10N.tr("bar-layout-combined"), "combined"),
                            (L10N.tr("bar-layout-separate"), "separate"),
                        ])),
                    ),
                    setting(
                        SettingId::BarEdge,
                        L10N.tr("bar-position"),
                        L10N.tr("bar-position-keywords"),
                        Kind::Choices(keyword_choices(vec![
                            (L10N.tr("edge-left"), "left"),
                            (L10N.tr("edge-bottom"), "bottom"),
                            (L10N.tr("edge-right"), "right"),
                            (L10N.tr("edge-top"), "top"),
                        ])),
                    ),
                    setting(
                        SettingId::BarIconSize,
                        L10N.tr("bar-size"),
                        L10N.tr("bar-size-keywords"),
                        Kind::Range {
                            range: BAR_SIZES,
                            step: 4,
                            low: L10N.tr("small"),
                            high: L10N.tr("large"),
                        },
                    ),
                    switch(SettingId::BarAutoHide, "bar-auto-hide"),
                ],
            )],
        ),
        pane(
            PaneId::Notifications,
            vec![
                section(
                    None,
                    vec![
                        switch(SettingId::DoNotDisturb, "do-not-disturb")
                            .described("do-not-disturb-description"),
                        switch(SettingId::FocusScheduled, "focus-scheduled")
                            .described("focus-scheduled-description"),
                        setting(
                            SettingId::FocusFrom,
                            L10N.tr("night-from"),
                            L10N.tr("focus-times-keywords"),
                            Kind::Text,
                        ),
                        setting(
                            SettingId::FocusTo,
                            L10N.tr("night-to"),
                            L10N.tr("focus-times-keywords"),
                            Kind::Text,
                        ),
                    ],
                ),
                section(
                    Some(L10N.tr("app-notifications")),
                    vec![list(
                        SettingId::AppNotifications,
                        "app-notifications",
                        List::NotificationApps,
                    )],
                ),
            ],
        ),
        search_pane(),
        accessibility_pane(),
    ]
}

fn search_pane() -> Pane {
    pane(
        PaneId::Search,
        vec![
            section(
                Some(L10N.tr("search-results")),
                vec![
                    switch(SettingId::SearchApplications, "search-applications"),
                    switch(SettingId::SearchRecentApps, "search-recent-apps"),
                    switch(SettingId::SearchRecentDocuments, "search-recent-documents"),
                    switch(SettingId::SearchDocuments, "search-documents"),
                    switch(SettingId::SearchFolders, "search-folders"),
                    switch(SettingId::SearchSettings, "search-system-settings"),
                    switch(SettingId::SearchSystem, "search-system-actions"),
                    switch(SettingId::SearchCalculator, "search-calculator"),
                    switch(SettingId::SearchConversion, "search-conversion"),
                ],
            )
            .noted("search-results-note"),
            section(
                Some(L10N.tr("search-privacy")),
                vec![list(
                    SettingId::SearchExcluded,
                    "search-privacy",
                    List::SearchExcluded,
                )],
            ),
        ],
    )
}

fn accessibility_pane() -> Pane {
    pane(
        PaneId::Accessibility,
        vec![
            section(
                Some(L10N.tr("vision")),
                vec![
                    switch(SettingId::ScreenReader, "screen-reader"),
                    switch(SettingId::Zoom, "zoom"),
                    switch(SettingId::IncreaseContrast, "increase-contrast"),
                    switch(SettingId::ReduceTransparency, "reduce-transparency"),
                    switch(SettingId::ReduceMotion, "reduce-motion"),
                    setting(
                        SettingId::TextSizeLink,
                        L10N.tr("text-size"),
                        L10N.tr("text-size-keywords"),
                        Kind::Page(PaneId::Appearance),
                    ),
                    setting(
                        SettingId::CursorSize,
                        L10N.tr("cursor-size"),
                        L10N.tr("cursor-size-keywords"),
                        Kind::Stops {
                            stops: numbers(&CURSOR_SIZES),
                            low: L10N.tr("small"),
                            high: L10N.tr("large"),
                        },
                    ),
                ],
            ),
            section(
                Some(L10N.tr("hearing")),
                vec![switch(SettingId::FlashScreen, "flash-screen")],
            ),
            section(
                Some(L10N.tr("motor")),
                vec![
                    switch(SettingId::StickyKeys, "sticky-keys")
                        .described("sticky-keys-description"),
                    switch(SettingId::SlowKeys, "slow-keys").described("slow-keys-description"),
                    switch(SettingId::BounceKeys, "bounce-keys")
                        .described("bounce-keys-description"),
                ],
            ),
        ],
    )
}

fn device_panes() -> Vec<Pane> {
    vec![
        displays_pane(),
        night_light_pane(),
        sound_pane(),
        keyboard_pane(),
        pane(
            PaneId::KeyboardShortcuts,
            vec![section(
                None,
                vec![list(SettingId::Shortcuts, "shortcuts", List::Shortcuts)],
            )],
        ),
        mouse_pane(),
        trackpad_pane(),
        pane(
            PaneId::Printers,
            vec![
                section(
                    Some(L10N.tr("printers-list")),
                    vec![list(SettingId::Printers, "printers-list", List::Printers)],
                ),
                section(
                    Some(L10N.tr("add-printer")),
                    vec![list(
                        SettingId::NearbyPrinters,
                        "add-printer",
                        List::NearbyPrinters,
                    )],
                ),
            ],
        ),
    ]
}

fn displays_pane() -> Pane {
    pane(
        PaneId::Displays,
        vec![
            section(
                None,
                vec![
                    level(SettingId::Brightness, "brightness"),
                    page(SettingId::NightLightPage, PaneId::NightLight),
                ],
            ),
            section(
                None,
                vec![
                    setting(
                        SettingId::DisplayUse,
                        L10N.tr("display-use"),
                        L10N.tr("display-use-keywords"),
                        Kind::DisplayUses,
                    ),
                    setting(
                        SettingId::DisplayResolution,
                        L10N.tr("display-resolution"),
                        L10N.tr("display-resolution-keywords"),
                        Kind::Resolutions,
                    ),
                    setting(
                        SettingId::DisplayScale,
                        L10N.tr("display-scale"),
                        L10N.tr("display-scale-keywords"),
                        choice(scale_choices()),
                    ),
                    setting(
                        SettingId::RefreshRate,
                        L10N.tr("refresh-rate"),
                        L10N.tr("refresh-rate-keywords"),
                        Kind::RefreshRates,
                    ),
                    setting(
                        SettingId::DisplayRotation,
                        L10N.tr("display-rotation"),
                        L10N.tr("display-rotation-keywords"),
                        choice(rotation_choices()),
                    ),
                    switch(SettingId::VariableRefreshRate, "variable-refresh-rate"),
                ],
            ),
        ],
    )
}

fn night_light_pane() -> Pane {
    pane(
        PaneId::NightLight,
        vec![
            section(
                None,
                vec![switch(SettingId::NightLight, "night-light-power")],
            ),
            section(
                None,
                vec![
                    setting(
                        SettingId::NightSchedule,
                        L10N.tr("night-schedule"),
                        L10N.tr("night-schedule-keywords"),
                        Kind::Choices(keyword_choices(vec![
                            (L10N.tr("night-sunset"), "sunset"),
                            (L10N.tr("night-custom"), "custom"),
                        ])),
                    ),
                    setting(
                        SettingId::NightFrom,
                        L10N.tr("night-from"),
                        L10N.tr("night-times-keywords"),
                        Kind::Text,
                    ),
                    setting(
                        SettingId::NightTo,
                        L10N.tr("night-to"),
                        L10N.tr("night-times-keywords"),
                        Kind::Text,
                    ),
                ],
            ),
            section(
                None,
                vec![setting(
                    SettingId::NightWarmth,
                    L10N.tr("night-warmth"),
                    L10N.tr("night-warmth-keywords"),
                    Kind::Range {
                        range: NIGHT_WARMTH,
                        step: 5,
                        low: L10N.tr("less-warm"),
                        high: L10N.tr("more-warm"),
                    },
                )],
            ),
        ],
    )
    .with_master(SettingId::NightLight)
}

fn sound_pane() -> Pane {
    pane(
        PaneId::Sound,
        vec![
            section(
                Some(L10N.tr("output")),
                vec![
                    list(
                        SettingId::OutputDevice,
                        "output-device",
                        List::OutputDevices,
                    ),
                    level(SettingId::OutputVolume, "output-volume"),
                    switch(SettingId::OutputMuted, "mute"),
                ],
            ),
            section(
                Some(L10N.tr("input")),
                vec![
                    list(SettingId::InputDevice, "input-device", List::InputDevices),
                    level(SettingId::InputVolume, "input-volume"),
                    switch(SettingId::InputMuted, "mute-input"),
                    list(SettingId::InputLevel, "input-level", List::InputLevel),
                ],
            ),
            section(
                Some(L10N.tr("sound-effects")),
                vec![
                    setting(
                        SettingId::AlertSound,
                        L10N.tr("alert-sound"),
                        L10N.tr("alert-sound-keywords"),
                        Kind::Menu(keyword_choices(vec![
                            (L10N.tr("alert-mallet"), "mallet"),
                            (L10N.tr("alert-knock"), "knock"),
                            (L10N.tr("alert-low"), "low"),
                            (L10N.tr("alert-tick"), "tick"),
                        ])),
                    ),
                    level(SettingId::AlertVolume, "alert-volume"),
                    switch(SettingId::SoundEffects, "interface-sounds"),
                    switch(SettingId::VolumeFeedback, "volume-feedback"),
                ],
            ),
            section(
                Some(L10N.tr("app-volumes")),
                vec![list(SettingId::AppVolumes, "app-volumes", List::AppVolumes)],
            ),
        ],
    )
}

fn keyboard_pane() -> Pane {
    pane(
        PaneId::Keyboard,
        vec![
            section(
                Some(L10N.tr("input-sources")),
                vec![list(
                    SettingId::InputSources,
                    "input-sources",
                    List::InputSources,
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
                None,
                vec![page(SettingId::ShortcutsPage, PaneId::KeyboardShortcuts)],
            ),
        ],
    )
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

fn acceleration(id: SettingId) -> Setting {
    switch(id, "pointer-acceleration").described("pointer-acceleration-description")
}

fn mouse_pane() -> Pane {
    pane(
        PaneId::Mouse,
        vec![section(
            None,
            vec![
                setting(
                    SettingId::MousePrimaryButton,
                    L10N.tr("primary-button"),
                    L10N.tr("primary-button-keywords"),
                    Kind::Choices(keyword_choices(vec![
                        (L10N.tr("primary-button-left"), "left"),
                        (L10N.tr("primary-button-right"), "right"),
                    ])),
                ),
                setting(
                    SettingId::MouseNaturalScrolling,
                    L10N.tr("natural-scrolling"),
                    L10N.tr("mouse-natural-scrolling-keywords"),
                    Kind::Switch,
                ),
                speed(SettingId::MouseSpeed, L10N.tr("mouse-speed-keywords")),
                acceleration(SettingId::MouseAcceleration),
            ],
        )],
    )
}

fn trackpad_pane() -> Pane {
    pane(
        PaneId::Trackpad,
        vec![
            section(
                Some(L10N.tr("trackpad-click")),
                vec![
                    switch(SettingId::TrackpadTapToClick, "tap-to-click"),
                    setting(
                        SettingId::TrackpadDragging,
                        L10N.tr("dragging"),
                        L10N.tr("dragging-keywords"),
                        Kind::Menu(keyword_choices(vec![
                            (L10N.tr("dragging-off"), "off"),
                            (L10N.tr("dragging-tap"), "tap"),
                            (L10N.tr("dragging-tap-with-lock"), "tap-with-lock"),
                        ])),
                    )
                    .described("dragging-description"),
                    setting(
                        SettingId::TrackpadSecondaryClick,
                        L10N.tr("secondary-click"),
                        L10N.tr("secondary-click-keywords"),
                        Kind::Choices(keyword_choices(vec![
                            (L10N.tr("secondary-click-two-fingers"), "two-fingers"),
                            (L10N.tr("secondary-click-corner"), "corner"),
                        ])),
                    ),
                ],
            ),
            section(
                Some(L10N.tr("trackpad-scroll")),
                vec![
                    switch(SettingId::TrackpadNaturalScrolling, "natural-scrolling"),
                    setting(
                        SettingId::TrackpadScrollMethod,
                        L10N.tr("scroll-method"),
                        L10N.tr("scroll-method-keywords"),
                        Kind::Choices(keyword_choices(vec![
                            (L10N.tr("scroll-method-two-fingers"), "two-fingers"),
                            (L10N.tr("scroll-method-edge"), "edge"),
                        ])),
                    ),
                ],
            ),
            section(
                Some(L10N.tr("trackpad-pointer")),
                vec![
                    speed(SettingId::TrackpadSpeed, L10N.tr("trackpad-speed-keywords")),
                    acceleration(SettingId::TrackpadAcceleration),
                    switch(
                        SettingId::TrackpadDisableWhileTyping,
                        "disable-while-typing",
                    ),
                ],
            ),
            section(
                Some(L10N.tr("trackpad-gestures")),
                vec![
                    gesture(SettingId::TrackpadWorkspaceSwipe, "workspace-swipe"),
                    gesture(SettingId::TrackpadOverviewSwipe, "overview-swipe"),
                    gesture(SettingId::TrackpadDesktopSpread, "desktop-spread"),
                    gesture(SettingId::TrackpadLauncherPinch, "launcher-pinch"),
                ],
            ),
        ],
    )
}

fn gesture(id: SettingId, name: &str) -> Setting {
    switch(id, name).described(&format!("{name}-description"))
}

fn system_panes() -> Vec<Pane> {
    vec![
        power_pane(),
        pane(
            PaneId::PrivacySecurity,
            vec![
                section(
                    Some(L10N.tr("app-permissions")),
                    vec![
                        page(SettingId::CameraPage, PaneId::Camera),
                        page(SettingId::MicrophonePage, PaneId::Microphone),
                        page(SettingId::ScreenshotsPage, PaneId::Screenshots),
                        page(SettingId::ScreenSharingPage, PaneId::ScreenSharing),
                        page(SettingId::LocationPage, PaneId::Location),
                    ],
                ),
                section(
                    Some(L10N.tr("screen-lock")),
                    vec![setting(
                        SettingId::ScreenLock,
                        L10N.tr("lock-when-off"),
                        L10N.tr("screen-lock-keywords"),
                        Kind::Page(PaneId::Power),
                    )],
                ),
                section(
                    Some(L10N.tr("clipboard")),
                    vec![list(
                        SettingId::ClipboardHistory,
                        "clipboard-history",
                        List::Clipboard,
                    )],
                ),
            ],
        ),
        pane(
            PaneId::Apps,
            vec![
                section(
                    Some(L10N.tr("default-apps")),
                    vec![list(
                        SettingId::DefaultApps,
                        "default-apps",
                        List::DefaultApps,
                    )],
                ),
                section(
                    Some(L10N.tr("open-at-login")),
                    vec![list(
                        SettingId::LoginItems,
                        "open-at-login",
                        List::LoginItems,
                    )],
                ),
                section(
                    Some(L10N.tr("reopen")),
                    vec![
                        switch(SettingId::ReopenApps, "reopen-at-login")
                            .described("reopen-at-login-description"),
                        switch(SettingId::ReopenAfterCrash, "reopen-after-crash")
                            .described("reopen-after-crash-description"),
                    ],
                ),
            ],
        ),
        permission_pane(Permission::Camera),
        permission_pane(Permission::Microphone),
        permission_pane(Permission::Screenshots),
        permission_pane(Permission::ScreenSharing),
        permission_pane(Permission::Location),
        language_time_pane(),
        about_pane(),
        pane(
            PaneId::Storage,
            vec![
                section(
                    None,
                    vec![list(
                        SettingId::StorageUsage,
                        "storage-usage",
                        List::Storage,
                    )],
                ),
                section(
                    Some(L10N.tr("large-files")),
                    vec![list(SettingId::LargeFiles, "large-files", List::LargeFiles)],
                ),
            ],
        ),
        pane(
            PaneId::SoftwareUpdate,
            vec![section(
                None,
                vec![list(
                    SettingId::SoftwareUpdates,
                    "software-updates",
                    List::SoftwareUpdates,
                )],
            )],
        ),
    ]
}

fn permission_pane(permission: Permission) -> Pane {
    let id = permission.pane();
    pane(
        id,
        vec![section(
            None,
            vec![setting(
                SettingId::AppPermissions,
                L10N.tr(&format!("{}-apps", id.keyword())),
                L10N.tr(&format!("{}-keywords", id.keyword())),
                Kind::List(List::Permissions(permission)),
            )],
        )],
    )
}

fn power_pane() -> Pane {
    pane(
        PaneId::Power,
        vec![
            section(
                Some(L10N.tr("battery")),
                vec![list(
                    SettingId::BatteryLevel,
                    "battery-level",
                    List::BatteryHistory,
                )],
            ),
            section(
                Some(L10N.tr("energy")),
                vec![
                    setting(
                        SettingId::PowerMode,
                        L10N.tr("power-mode"),
                        L10N.tr("power-mode-keywords"),
                        Kind::Choices(keyword_choices(vec![
                            (L10N.tr("power-saver"), "power-saver"),
                            (L10N.tr("balanced"), "balanced"),
                            (L10N.tr("performance"), "performance"),
                        ])),
                    ),
                    switch(SettingId::LowPowerOnBattery, "low-power-on-battery"),
                ],
            ),
            section(
                Some(L10N.tr("when-inactive")),
                vec![
                    setting(
                        SettingId::DimAfter,
                        L10N.tr("dim-after"),
                        L10N.tr("dim-after-keywords"),
                        choice(durations(&DIM_CHOICES)),
                    ),
                    setting(
                        SettingId::LockAfter,
                        L10N.tr("lock-after"),
                        L10N.tr("lock-after-keywords"),
                        choice(durations(&LOCK_CHOICES)),
                    ),
                    setting(
                        SettingId::SleepAfter,
                        L10N.tr("sleep-after"),
                        L10N.tr("sleep-after-keywords"),
                        choice(sleep_choices()),
                    ),
                ],
            ),
            section(
                Some(L10N.tr("on-power-adapter")),
                vec![setting(
                    SettingId::LidOnPower,
                    L10N.tr("lid-on-power"),
                    L10N.tr("lid-on-power-keywords"),
                    Kind::Menu(keyword_choices(vec![
                        (L10N.tr("lid-sleep"), "sleep"),
                        (L10N.tr("lid-keep-running"), "keep-running"),
                    ])),
                )],
            ),
        ],
    )
}

fn language_time_pane() -> Pane {
    pane(
        PaneId::LanguageTime,
        vec![
            section(
                Some(L10N.tr("language")),
                vec![list(SettingId::Language, "language", List::Languages)],
            ),
            section(
                Some(L10N.tr("formats")),
                vec![
                    list(SettingId::Region, "region", List::Regions),
                    switch(SettingId::Clock24Hour, "clock-24-hour"),
                ],
            ),
            section(
                Some(L10N.tr("time")),
                vec![
                    switch(SettingId::AutomaticTime, "automatic-time"),
                    list(SettingId::DateAndTime, "date-and-time", List::Clock),
                ],
            ),
            section(
                Some(L10N.tr("time-zone")),
                vec![list(SettingId::TimeZone, "time-zone", List::TimeZones)],
            ),
        ],
    )
}

fn about_pane() -> Pane {
    pane(
        PaneId::About,
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
                vec![list(
                    SettingId::AboutThisComputer,
                    "about-this-computer",
                    List::About,
                )],
            ),
            section(
                None,
                vec![
                    page(SettingId::StoragePage, PaneId::Storage),
                    page(SettingId::SoftwareUpdatePage, PaneId::SoftwareUpdate),
                ],
            ),
        ],
    )
}

pub fn panes() -> Vec<Pane> {
    let mut panes = connection_panes();
    panes.extend(desktop_panes());
    panes.extend(device_panes());
    panes.extend(system_panes());
    panes
}
