use bande_l10n::{text, tr, trn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Connections,
    Desktop,
    Devices,
    System,
}

impl Group {
    pub fn label(self) -> String {
        match self {
            Self::Connections => tr!("Connections"),
            Self::Desktop => tr!("Desktop"),
            Self::Devices => tr!("Devices"),
            Self::System => tr!("System"),
        }
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
    Sharing,
    Power,
    LockScreen,
    Users,
    Passwords,
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
    Diagnostics,
    SendReport,
    Backup,
}

impl PaneId {
    const KEYWORDS: [(Self, &'static str); 35] = [
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
        (Self::Sharing, "sharing"),
        (Self::Power, "power"),
        (Self::LockScreen, "lock-screen"),
        (Self::Users, "users"),
        (Self::Passwords, "passwords"),
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
        (Self::Diagnostics, "diagnostics"),
        (Self::SendReport, "send-report"),
        (Self::Backup, "backup"),
    ];

    pub fn group(self) -> Option<Group> {
        match self {
            Self::Wifi | Self::Bluetooth | Self::Network | Self::Sharing => {
                Some(Group::Connections)
            }
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
            Self::Power
            | Self::LockScreen
            | Self::Users
            | Self::Passwords
            | Self::PrivacySecurity
            | Self::Apps
            | Self::LanguageTime
            | Self::Backup
            | Self::About => Some(Group::System),
            Self::KeyboardShortcuts
            | Self::NightLight
            | Self::Camera
            | Self::Microphone
            | Self::Screenshots
            | Self::ScreenSharing
            | Self::Location
            | Self::Storage
            | Self::SoftwareUpdate
            | Self::Diagnostics
            | Self::SendReport => None,
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
            Self::Sharing => "folder-publicshare-symbolic",
            Self::Power => "system-shutdown-symbolic",
            Self::LockScreen => "system-lock-screen-symbolic",
            Self::Users => "system-users-symbolic",
            Self::Passwords => "dialog-password-symbolic",
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
            Self::Backup => "document-open-recent-symbolic",
            Self::SoftwareUpdate => "view-refresh-symbolic",
            Self::Diagnostics => "dialog-warning-symbolic",
            Self::SendReport => "dialog-warning-symbolic",
        }
    }

    pub fn title(self) -> String {
        match self {
            Self::Wifi => tr!("Wi-Fi"),
            Self::Bluetooth => tr!("Bluetooth"),
            Self::Network => tr!("Network"),
            Self::Appearance => tr!("Appearance"),
            Self::Bar => tr!("Taskbar"),
            Self::Notifications => tr!("Notifications"),
            Self::Search => tr!("Search"),
            Self::Accessibility => tr!("Accessibility"),
            Self::Displays => tr!("Displays"),
            Self::NightLight => tr!("Night Light"),
            Self::Sound => tr!("Sound"),
            Self::Keyboard => tr!("Keyboard"),
            Self::KeyboardShortcuts => tr!("Keyboard Shortcuts"),
            Self::Mouse => tr!("Mouse"),
            Self::Trackpad => tr!("Trackpad"),
            Self::Printers => tr!("Printers & Scanners"),
            Self::Sharing => tr!("Sharing"),
            Self::Power => tr!("Power"),
            Self::LockScreen => tr!("Lock Screen"),
            Self::Users => tr!("Users & Groups"),
            Self::Passwords => tr!("Passwords"),
            Self::PrivacySecurity => tr!("Privacy & Security"),
            Self::Camera => tr!("Camera"),
            Self::Microphone => tr!("Microphone"),
            Self::Screenshots => tr!("Screenshots"),
            Self::ScreenSharing => tr!("Screen Sharing"),
            Self::Location => tr!("Location"),
            Self::Apps => tr!("Apps"),
            Self::LanguageTime => tr!("Language & Time"),
            Self::About => tr!("About"),
            Self::Storage => tr!("Storage"),
            Self::SoftwareUpdate => tr!("Software Update"),
            Self::Diagnostics => tr!("Diagnostics"),
            Self::SendReport => tr!("Send Report…"),
            Self::Backup => tr!("Backup"),
        }
    }

    pub fn summary(self) -> String {
        match self {
            Self::Wifi => tr!("Join networks and see how you are connected"),
            Self::Bluetooth => tr!("Connect headphones, keyboards and other devices"),
            Self::Network => tr!("Wired connections and VPNs"),
            Self::Appearance => {
                tr!("Light, dark or automatic, accent color, wallpaper and text size")
            }
            Self::Bar => tr!("Where the taskbar sits and how it behaves"),
            Self::Notifications => tr!("Banners and Notification Center"),
            Self::Search => tr!("What the launcher finds and which folders it leaves out"),
            Self::Accessibility => {
                tr!("Make the screen, sound, keyboard and pointer easier to use")
            }
            Self::Displays => tr!("Arrangement, resolution and size of each display"),
            Self::NightLight => tr!("Warmer colors after dark"),
            Self::Sound => tr!("Output, input and sound effects"),
            Self::Keyboard => tr!("Input sources, key repeat and modifier keys"),
            Self::KeyboardShortcuts => tr!("The keys that work everywhere on the desktop"),
            Self::Mouse => tr!("Buttons, scrolling and tracking speed"),
            Self::Trackpad => tr!("Tapping, clicking, scrolling, tracking speed and gestures"),
            Self::Printers => tr!("Add printers, pick the default and see what is printing"),
            Self::Sharing => tr!("Share folders with other computers and allow remote login"),
            Self::Power => tr!("Battery, power mode and what the lid does"),
            Self::LockScreen => tr!("Screen saver, display off, sleep and the password"),
            Self::Users => tr!("Your account and the others on this computer"),
            Self::Passwords => tr!("The passwords apps and networks keep in your keychain"),
            Self::PrivacySecurity => {
                tr!("Which apps may use the camera, take screenshots or share the screen")
            }
            Self::Camera => tr!("Apps that asked to use the camera"),
            Self::Microphone => tr!("Apps that asked to use the microphone"),
            Self::Screenshots => tr!("Apps that asked to take screenshots"),
            Self::ScreenSharing => tr!("Apps that remember what they may share"),
            Self::Location => tr!("Apps that asked for your location"),
            Self::Apps => tr!("Which apps open your files and links, and which open at login"),
            Self::LanguageTime => tr!("Language, formats, time and time zone"),
            Self::About => tr!("This computer"),
            Self::Storage => tr!("What fills the disk and how to free up space"),
            Self::SoftwareUpdate => tr!("Updates for Bande and Debian apps"),
            Self::Diagnostics => tr!(
                "What failed on this computer. Read each report, and choose whether it is sent."
            ),
            Self::SendReport => tr!("Tell what went wrong, with what this computer recorded."),
            Self::Backup => tr!("Hourly encrypted versions on an external disk"),
        }
    }

    pub fn keywords(self) -> String {
        match self {
            Self::Wifi => tr!("wireless, wlan, internet, network"),
            Self::Bluetooth => tr!("wireless, headphones, mouse, keyboard, speaker"),
            Self::Network => tr!("internet, connection, lan"),
            Self::Appearance => tr!(
                "theme, look, colour, color, wallpaper, background, desktop picture, picture, photo, image"
            ),
            Self::Bar => {
                tr!("bar, dock, menu bar, launcher, taskbar, panel, favourites, favorites")
            }
            Self::Notifications => tr!("alerts, banners"),
            Self::Search => tr!("spotlight, launcher, super, results, recents, privacy"),
            Self::Accessibility => tr!("a11y, disability, assistive, vision, hearing, motor"),
            Self::Displays => tr!("monitor, screen, resolution, brightness, night light"),
            Self::NightLight => tr!(
                "night shift, blue light, warm, redshift, colour temperature, color temperature, eye strain"
            ),
            Self::Sound => tr!("audio, speakers, headphones, microphone"),
            Self::Keyboard => tr!("keys, typing"),
            Self::KeyboardShortcuts => {
                tr!("hotkeys, shortcuts, keys, super, command, combinations")
            }
            Self::Mouse => tr!("pointer, cursor, click, scroll, wheel, buttons"),
            Self::Trackpad => tr!("touchpad, pointer, cursor, tap, click, scroll"),
            Self::Printers => tr!("print, printer, cups, queue, scanner, ipp, airprint"),
            Self::Sharing => {
                tr!("share, file sharing, samba, smb, remote login, ssh, network folder")
            }
            Self::Power => tr!("energy, battery, low power mode, power saver, lid"),
            Self::LockScreen => tr!(
                "auto lock, automatic lock, lock when idle, security, inactive, screen saver, screen timeout, password"
            ),
            Self::Users => tr!("account, user, password, login, administrator, admin"),
            Self::Passwords => tr!(
                "keychain, keyring, saved passwords, secrets, wi-fi passwords, website passwords, accounts"
            ),
            Self::PrivacySecurity => {
                tr!("permissions, screen recording, camera access, privacy, security, clipboard")
            }
            Self::Camera => tr!("webcam, video calls, camera access, permission"),
            Self::Microphone => tr!("mic, audio recording, microphone access, permission"),
            Self::Screenshots => tr!("screen capture, screenshot permission, take screenshots"),
            Self::ScreenSharing => tr!(
                "screen recording, screencast, share screen, remote desktop, remembered windows"
            ),
            Self::Location => tr!("location services, gps, geolocation, where am I"),
            Self::Apps => tr!(
                "applications, login items, startup apps, autostart, open at login, default apps, open with"
            ),
            Self::LanguageTime => tr!(
                "locale, region, formats, international, date, time, timezone, clock, ntp, translation"
            ),
            Self::About => tr!(
                "about this computer, system information, specifications, specs, general, hostname, computer name, version"
            ),
            Self::Storage => tr!("disk, drive, space, ssd"),
            Self::SoftwareUpdate => tr!("updates, upgrade, packages, apt, debian"),
            Self::Diagnostics => tr!(
                "crash, crashes, freeze, hang, quit unexpectedly, kernel panic, core dump, log, diagnostics, report, send, bug, privacy"
            ),
            Self::SendReport => {
                tr!("bug, report, problem, freeze, crash, feedback, send, diagnose")
            }
            Self::Backup => {
                tr!("time machine, restore, versions, external disk, history, snapshot, recover")
            }
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
            Self::Storage | Self::SoftwareUpdate | Self::SendReport => Some(Self::About),
            Self::Diagnostics => Some(Self::PrivacySecurity),
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
    OutputBalance,
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
    Style,
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
    BarFloating,
    BarGap,
    DisplayUse,
    DisplayResolution,
    DisplayScale,
    RefreshRate,
    ColourProfile,
    ShowAllResolutions,
    DisplayRotation,
    ScreenSaverAfter,
    DisplayOffAfter,
    RequirePasswordAfter,
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
    AutomaticUpdates,
    UpdateCheckEvery,
    BatteryLevel,
    PowerMode,
    LowPowerOnBattery,
    PowerStatus,
    LidOnPower,
    AppNotifications,
    SoftwareUpdates,
    BackupStatus,
    Ethernet,
    EthernetAddress,
    EthernetRouter,
    EthernetDns,
    Vpns,
    Language,
    AppLanguages,
    Region,
    StoragePage,
    SoftwareUpdatePage,
    DiagnosticsPage,
    DiagnosticsList,
    SendReportPage,
    SendReportForm,
    ShortcutsPage,
    Spelling,
    TextReplacements,
    ClipboardHistory,
    KeyboardAccess,
    SecurityStatus,
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
    SearchMail,
    SearchEvents,
    SearchContacts,
    SearchFolders,
    SearchSettings,
    SearchSystem,
    SearchCalculator,
    SearchConversion,
    SearchExcluded,
    Printers,
    NearbyPrinters,
    Sharing,
    Accounts,
    FullName,
    LockMessage,
    ShowPreviews,
    SavedPasswords,
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
    Accounts,
    Printers,
    NearbyPrinters,
    Sharing,
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
    Diagnostics,
    SendReport,
    LargeFiles,
    Backup,
    Clock,
    TimeZones,
    LoginItems,
    BatteryHistory,
    PowerStatus,
    NotificationApps,
    SoftwareUpdates,
    Ethernet,
    Vpns,
    Languages,
    AppLanguages,
    Regions,
    DefaultApps,
    Clipboard,
    Spelling,
    TextReplacements,
    KeyboardAccess,
    SecurityStatus,
    SearchExcluded,
    Permissions(Permission),
    Passwords,
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
    ColourProfiles,
    TextSizes,
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
            | Self::ColourProfiles
            | Self::TextSizes
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
    fn described(mut self, description: String) -> Self {
        self.description = Some(description);
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
    fn noted(mut self, note: String) -> Self {
        self.note = Some(note);
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
    Pane {
        id,
        name: id.title(),
        summary: id.summary(),
        keywords: words(&id.keywords()),
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
    setting(id, pane.title(), pane.keywords(), Kind::Page(pane))
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

const DAY: u64 = 24 * 60 * 60;

fn update_check_choices() -> Vec<Choice> {
    [
        (text!("When available"), 0),
        (text!("Every day"), DAY),
        (text!("Every 7 days"), 7 * DAY),
        (text!("Every 31 days"), 31 * DAY),
    ]
    .into_iter()
    .map(|(label, seconds)| Choice {
        label: label.tr(),
        value: Value::Seconds(seconds),
    })
    .collect()
}

fn password_choices() -> Vec<Choice> {
    PASSWORD_CHOICES
        .iter()
        .map(|&seconds| Choice {
            label: match seconds {
                0 => tr!("Immediately"),
                seconds => duration_label(seconds),
            },
            value: Value::Seconds(seconds),
        })
        .collect()
}

fn sleep_choices() -> Vec<Choice> {
    let mut choices = keyword_choices(vec![(tr!("With display"), WITH_DISPLAY)]);
    choices.extend(durations(&SLEEP_CHOICES));
    choices
}

pub fn duration_label(seconds: u64) -> String {
    match seconds {
        0 => tr!("Never"),
        1..60 => tr!("{count} s", count = seconds),
        _ if seconds.is_multiple_of(3600) => {
            trn!("{count} hour", "{count} hours", count = (seconds / 3600))
        }
        _ => tr!("{count} min", count = (seconds / 60)),
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

const SCREEN_SAVER_CHOICES: [u64; 8] = [60, 120, 300, 600, 1200, 1800, 3600, 0];
const DISPLAY_OFF_CHOICES: [u64; 9] = [60, 120, 300, 600, 1200, 1800, 3600, 10800, 0];
const PASSWORD_CHOICES: [u64; 7] = [0, 5, 60, 300, 900, 3600, 28800];
const SLEEP_CHOICES: [u64; 4] = [1800, 3600, 10800, 0];
pub const CURSOR_SIZES: [u32; 5] = [24, 32, 48, 64, 96];
const REPEAT_RATES: [u32; 9] = [2, 5, 10, 15, 20, 25, 30, 45, 60];
const REPEAT_DELAYS: [u32; 8] = [2000, 1500, 1000, 800, 600, 450, 300, 200];
pub const POINTER_SPEEDS: u32 = 10;

fn rotation_choices() -> Vec<Choice> {
    std::iter::once(Choice {
        label: tr!("Standard"),
        value: Value::Number(0),
    })
    .chain([90, 180, 270].into_iter().map(|degrees: u32| Choice {
        label: tr!("{value}°", value = degrees),
        value: Value::Number(degrees),
    }))
    .collect()
}

pub const AUTOMATIC: &str = "automatic";

const BAR_SIZES: (u32, u32) = (22, 56);
const BAR_GAPS: (u32, u32) = (0, 24);
const NIGHT_WARMTH: (u32, u32) = (0, 100);
pub const TEXT_SIZES: [u32; 5] = [100, 125, 150, 175, 200];

pub const SCALES: [u32; 5] = [200, 175, 150, 125, 100];

fn list(id: SettingId, name: String, keywords: String, list: List) -> Setting {
    setting(id, name, keywords, Kind::List(list))
}

fn switch(id: SettingId, name: String, keywords: String) -> Setting {
    setting(id, name, keywords, Kind::Switch)
}

fn level(id: SettingId, name: String, keywords: String) -> Setting {
    setting(id, name, keywords, Kind::Level)
}

fn connection_panes() -> Vec<Pane> {
    vec![
        pane(
            PaneId::Wifi,
            vec![
                section(None, vec![switch(SettingId::WifiPower, tr!("Wi-Fi"), tr!("on, off, airplane"))]),
                section(
                    Some(tr!("Known Networks")),
                    vec![list(SettingId::KnownNetworks, tr!("Known Networks"), tr!("saved, remembered, auto-join, forget, details, ip address, router, dns, mac address, private address, low data, metered"),
                        List::KnownNetworks,
                    )],
                ),
                section(
                    Some(tr!("Other Networks")),
                    vec![list(SettingId::Networks, tr!("Other Networks"), tr!("join, connect, ssid, hidden, other, password"), List::Networks)],
                ),
            ],
        )
        .with_master(SettingId::WifiPower),
        pane(
            PaneId::Bluetooth,
            vec![
                section(
                    None,
                    vec![switch(SettingId::BluetoothPower, tr!("Bluetooth"), tr!("on, off"))],
                ),
                section(
                    Some(tr!("My devices")),
                    vec![list(SettingId::PairedDevices, tr!("My devices"), tr!("paired, connect, disconnect, headphones"),
                        List::PairedDevices,
                    )],
                ),
                section(
                    Some(tr!("Nearby devices")),
                    vec![list(SettingId::NearbyDevices, tr!("Nearby devices"), tr!("pair, add, discover, new device"),
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
                    Some(tr!("Ethernet")),
                    vec![list(SettingId::Ethernet, tr!("Ethernet"), tr!("wired, cable, lan, ip address, dns, router, gateway, dhcp, static ip, manual address, subnet"), List::Ethernet)],
                ),
                section(
                    Some(tr!("VPN")),
                    vec![list(SettingId::Vpns, tr!("VPN"), tr!("wireguard, openvpn, tunnel, private network, import, work network"), List::Vpns)],
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
                        tr!("Appearance"),
                        tr!("dark mode, light mode, night, theme"),
                        Kind::Choices(keyword_choices(vec![
                            (tr!(context = "appearance", "Light"), "light"),
                            (tr!(context = "appearance", "Dark"), "dark"),
                            (tr!("Auto"), "automatic"),
                        ])),
                    )],
                ),
                section(
                    None,
                    vec![setting(
                        SettingId::Style,
                        tr!("Style"),
                        tr!(
                            "glass, blur, translucent, solid, transparent, material, look, taskbar"
                        ),
                        Kind::Choices(keyword_choices(vec![
                            (tr!("Refined Solid"), "refined"),
                            (tr!("Wallpaper-Lit"), "wallpaper-lit"),
                            (tr!("Glass"), "glass"),
                        ])),
                    )],
                ),
                section(
                    None,
                    vec![setting(
                        SettingId::Accent,
                        tr!("Accent color"),
                        tr!("accent, highlight, selection, color, colour, folders, tint"),
                        Kind::Swatches(keyword_choices(vec![
                            (tr!("Ultramarine"), "ultramarine"),
                            (tr!("Teal"), "teal"),
                            (tr!("Moss"), "moss"),
                            (tr!("Amber"), "amber"),
                            (tr!("Ember"), "ember"),
                            (tr!("Rose"), "rose"),
                            (tr!("Plum"), "plum"),
                            (tr!("Graphite"), "graphite"),
                        ])),
                    )],
                ),
                section(
                    None,
                    vec![setting(
                        SettingId::TextSize,
                        tr!("Text size"),
                        tr!(
                            "font size, larger text, bigger text, smaller text, readability, text scaling"
                        ),
                        Kind::Stops {
                            stops: numbers(&TEXT_SIZES),
                            low: tr!("Default"),
                            high: tr!("Largest"),
                        },
                    )],
                ),
                section(
                    Some(tr!("Wallpaper")),
                    vec![
                        setting(
                            SettingId::Wallpaper,
                            tr!("Wallpaper"),
                            tr!("background, picture, add picture"),
                            Kind::Pictures,
                        ),
                        setting(
                            SettingId::WallpaperFit,
                            tr!("Picture size"),
                            tr!("fill, fit, scale, stretch, crop"),
                            Kind::Choices(keyword_choices(vec![
                                (tr!("Fill screen"), "fill"),
                                (tr!("Fit to screen"), "fit"),
                            ])),
                        ),
                        switch(
                            SettingId::TintedFrame,
                            tr!("Tint window frames with the wallpaper"),
                            tr!("tinted frame, wallpaper colour, chassis, window colour"),
                        ),
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
                        SettingId::BarEdge,
                        tr!("Position on screen"),
                        tr!("panel, taskbar, dock, edge, left, right, bottom, top"),
                        Kind::Choices(keyword_choices(vec![
                            (tr!("Left"), "left"),
                            (tr!("Bottom"), "bottom"),
                            (tr!("Right"), "right"),
                            (tr!("Top"), "top"),
                        ])),
                    ),
                    setting(
                        SettingId::BarIconSize,
                        tr!("Size"),
                        tr!("icon size, bigger, smaller, large, small, width"),
                        Kind::Range {
                            range: BAR_SIZES,
                            step: 4,
                            low: tr!("Small"),
                            high: tr!("Large"),
                        },
                    ),
                    switch(
                        SettingId::BarFloating,
                        tr!("Float away from the screen edge"),
                        tr!("floating, attached, dock, margin, edge, rounded"),
                    ),
                    setting(
                        SettingId::BarGap,
                        tr!("Spacing"),
                        tr!("gap, margin, space, distance, edge, floating, maximized"),
                        Kind::Range {
                            range: BAR_GAPS,
                            step: 2,
                            low: tr!(context = "gap", "None"),
                            high: tr!("Wide"),
                        },
                    ),
                    switch(
                        SettingId::BarAutoHide,
                        tr!("Automatically hide and show the taskbar"),
                        tr!("autohide, hide, show, reveal, dock"),
                    ),
                ],
            )],
        ),
        pane(
            PaneId::Notifications,
            vec![
                section(
                    None,
                    vec![
                        switch(
                            SettingId::DoNotDisturb,
                            tr!("Do Not Disturb"),
                            tr!("focus, quiet, silence, mute notifications, banners"),
                        )
                        .described(tr!("Banners wait; urgent alerts still show")),
                        switch(
                            SettingId::ShowPreviews,
                            tr!("Show previews"),
                            tr!("preview, body, text, content, privacy, hide"),
                        )
                        .described(tr!(
                            "Off shows each notification's title without its text or picture"
                        )),
                        switch(
                            SettingId::FocusScheduled,
                            tr!("Turn on by schedule"),
                            tr!("focus, schedule, night, sleep, bedtime, quiet hours, automatic"),
                        )
                        .described(tr!("Do Not Disturb turns on and off at set times each day")),
                        setting(
                            SettingId::FocusFrom,
                            tr!("From"),
                            tr!("start, end, evening, morning, schedule times, quiet hours"),
                            Kind::Text,
                        ),
                        setting(
                            SettingId::FocusTo,
                            tr!("To"),
                            tr!("start, end, evening, morning, schedule times, quiet hours"),
                            Kind::Text,
                        ),
                    ],
                ),
                section(
                    Some(tr!("Application notifications")),
                    vec![list(
                        SettingId::AppNotifications,
                        tr!("Application notifications"),
                        tr!(
                            "per app, apps, banners, alerts, allow notifications, persistent, notification center, block, silence an app"
                        ),
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
                Some(tr!("Search results")),
                vec![
                    switch(
                        SettingId::SearchApplications,
                        tr!("Applications"),
                        tr!("apps, programs"),
                    ),
                    switch(
                        SettingId::SearchRecentApps,
                        tr!("Recent apps"),
                        tr!("recently used apps, recents"),
                    ),
                    switch(
                        SettingId::SearchRecentDocuments,
                        tr!("Recent documents"),
                        tr!("recent files, recently opened, recents, history"),
                    ),
                    switch(
                        SettingId::SearchDocuments,
                        tr!("Documents"),
                        tr!("files, file names, contents, text"),
                    ),
                    switch(SettingId::SearchMail, tr!("Mail"), tr!("email, messages")),
                    switch(
                        SettingId::SearchEvents,
                        tr!("Events"),
                        tr!("calendar, appointments"),
                    ),
                    switch(
                        SettingId::SearchContacts,
                        tr!("Contacts"),
                        tr!("people, addresses"),
                    ),
                    switch(SettingId::SearchFolders, tr!("Folders"), tr!("directories")),
                    switch(
                        SettingId::SearchSettings,
                        tr!("System Settings"),
                        tr!("settings, preferences, panes"),
                    ),
                    switch(
                        SettingId::SearchSystem,
                        tr!("System actions"),
                        tr!("lock screen, sleep, log out, restart, shut down"),
                    ),
                    switch(
                        SettingId::SearchCalculator,
                        tr!("Calculator"),
                        tr!("maths, math, arithmetic, percentages"),
                    ),
                    switch(
                        SettingId::SearchConversion,
                        tr!("Conversion"),
                        tr!("units, convert, temperature, distance"),
                    ),
                ],
            )
            .noted(tr!("Choose which results appear when you search")),
            section(
                Some(tr!("Search privacy")),
                vec![list(
                    SettingId::SearchExcluded,
                    tr!("Search privacy"),
                    tr!("exclude, excluded folders, hide folders, private, index"),
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
                Some(tr!("Vision")),
                vec![
                    switch(
                        SettingId::ScreenReader,
                        tr!("Screen reader"),
                        tr!("orca, voice, speech, spoken, read aloud, blind, voiceover, narrator"),
                    ),
                    switch(
                        SettingId::Zoom,
                        tr!("Zoom"),
                        tr!("magnifier, magnify, enlarge, low vision, Alt+Super+8"),
                    ),
                    switch(
                        SettingId::IncreaseContrast,
                        tr!("Increase contrast"),
                        tr!("high contrast, borders, outlines, legibility"),
                    ),
                    switch(
                        SettingId::ReduceTransparency,
                        tr!("Reduce transparency"),
                        tr!("blur, translucent, opaque, see-through"),
                    ),
                    switch(
                        SettingId::ReduceMotion,
                        tr!("Reduce motion"),
                        tr!("animation, animations, movement, motion sickness"),
                    ),
                    setting(
                        SettingId::TextSizeLink,
                        tr!("Text size"),
                        tr!(
                            "font size, larger text, bigger text, smaller text, readability, text scaling"
                        ),
                        Kind::Page(PaneId::Appearance),
                    ),
                    setting(
                        SettingId::CursorSize,
                        tr!("Pointer size"),
                        tr!("cursor size, mouse pointer, bigger pointer, arrow"),
                        Kind::Stops {
                            stops: numbers(&CURSOR_SIZES),
                            low: tr!("Small"),
                            high: tr!("Large"),
                        },
                    ),
                ],
            ),
            section(
                Some(tr!("Hearing")),
                vec![switch(
                    SettingId::FlashScreen,
                    tr!("Flash the screen when an alert sound plays"),
                    tr!("visual bell, alert, beep, deaf, hard of hearing"),
                )],
            ),
            section(
                Some(tr!("Motor")),
                vec![
                    switch(
                        SettingId::StickyKeys,
                        tr!("Sticky keys"),
                        tr!("modifier, one hand, shift five times, latch"),
                    )
                    .described(tr!(
                        "Press Shift, Control, Alt or Super one at a time for a shortcut"
                    )),
                    switch(
                        SettingId::SlowKeys,
                        tr!("Slow keys"),
                        tr!("delay, accidental keys, hold, acceptance"),
                    )
                    .described(tr!("Accept a key only after it is held down briefly")),
                    switch(
                        SettingId::BounceKeys,
                        tr!("Bounce keys"),
                        tr!("double press, tremor, debounce, stutter"),
                    )
                    .described(tr!("Ignore quick repeated presses of the same key")),
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
                vec![list(
                    SettingId::Shortcuts,
                    tr!("Keyboard shortcuts"),
                    tr!("hotkeys, keys, super, command, combinations"),
                    List::Shortcuts,
                )],
            )],
        ),
        mouse_pane(),
        trackpad_pane(),
        pane(
            PaneId::Sharing,
            vec![section(
                None,
                vec![list(
                    SettingId::Sharing,
                    tr!("Sharing"),
                    tr!("file sharing, samba, smb, remote login, ssh, shared folders"),
                    List::Sharing,
                )],
            )],
        ),
        pane(
            PaneId::Printers,
            vec![
                section(
                    Some(tr!("Printers")),
                    vec![list(
                        SettingId::Printers,
                        tr!("Printers"),
                        tr!("default printer, queue, jobs, pause, resume, remove, cancel"),
                        List::Printers,
                    )],
                ),
                section(
                    Some(tr!("Add a Printer")),
                    vec![list(
                        SettingId::NearbyPrinters,
                        tr!("Add a Printer"),
                        tr!("add, nearby, network, airprint, ipp everywhere, address, ip"),
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
                vec![page(SettingId::NightLightPage, PaneId::NightLight)],
            ),
            section(
                None,
                vec![
                    level(
                        SettingId::Brightness,
                        tr!("Brightness"),
                        tr!("backlight, dim, bright, screen brightness"),
                    ),
                    setting(
                        SettingId::DisplayUse,
                        tr!("Use as"),
                        tr!("main display, mirror, extend, primary, menu bar, bar"),
                        Kind::DisplayUses,
                    ),
                    setting(
                        SettingId::DisplayScale,
                        tr!("Size of text and controls"),
                        tr!("scale, zoom, resolution, hidpi, retina, bigger, smaller"),
                        Kind::TextSizes,
                    ),
                    switch(
                        SettingId::ShowAllResolutions,
                        tr!("Show all resolutions"),
                        tr!("resolution, modes, pixels, native"),
                    ),
                    setting(
                        SettingId::DisplayResolution,
                        tr!("Resolution"),
                        tr!("pixels, size, mode, sharp"),
                        Kind::Resolutions,
                    ),
                    setting(
                        SettingId::RefreshRate,
                        tr!("Refresh rate"),
                        tr!("hertz, hz, smooth, frame rate"),
                        Kind::RefreshRates,
                    ),
                    setting(
                        SettingId::DisplayRotation,
                        tr!("Rotation"),
                        tr!("rotate, portrait, landscape, orientation, turn"),
                        choice(rotation_choices()),
                    ),
                    setting(
                        SettingId::ColourProfile,
                        tr!("Color profile"),
                        tr!("colour, color, profile, icc, srgb, p3, wide gamut, calibration"),
                        Kind::ColourProfiles,
                    ),
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
                vec![switch(
                    SettingId::NightLight,
                    tr!("Night Light"),
                    tr!("on, off, night shift, blue light"),
                )],
            ),
            section(
                None,
                vec![
                    setting(
                        SettingId::NightSchedule,
                        tr!("Schedule"),
                        tr!("sunset, sunrise, automatic, custom, times"),
                        Kind::Choices(keyword_choices(vec![
                            (tr!("Sunset to sunrise"), "sunset"),
                            (tr!("Custom"), "custom"),
                        ])),
                    ),
                    setting(
                        SettingId::NightFrom,
                        tr!("From"),
                        tr!("start, end, evening, morning, schedule times"),
                        Kind::Text,
                    ),
                    setting(
                        SettingId::NightTo,
                        tr!("To"),
                        tr!("start, end, evening, morning, schedule times"),
                        Kind::Text,
                    ),
                ],
            ),
            section(
                None,
                vec![setting(
                    SettingId::NightWarmth,
                    tr!("Warmth"),
                    tr!("colour temperature, color temperature, warm, kelvin, intensity, strength"),
                    Kind::Range {
                        range: NIGHT_WARMTH,
                        step: 5,
                        low: tr!("Less warm"),
                        high: tr!("More warm"),
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
                Some(tr!("Output")),
                vec![
                    list(
                        SettingId::OutputDevice,
                        tr!("Play sound through"),
                        tr!("speakers, headphones, output device"),
                        List::OutputDevices,
                    ),
                    setting(
                        SettingId::OutputBalance,
                        tr!("Balance"),
                        tr!("left, right, stereo, channels, speakers"),
                        Kind::Range {
                            range: (0, 100),
                            step: 5,
                            low: tr!("Left"),
                            high: tr!("Right"),
                        },
                    ),
                    level(
                        SettingId::OutputVolume,
                        tr!("Output volume"),
                        tr!("loudness, speakers, level"),
                    ),
                    switch(
                        SettingId::OutputMuted,
                        tr!("Mute"),
                        tr!("silence, sound off"),
                    ),
                ],
            ),
            section(
                Some(tr!("Input")),
                vec![
                    list(
                        SettingId::InputDevice,
                        tr!("Record sound from"),
                        tr!("microphone, input device, recording"),
                        List::InputDevices,
                    ),
                    level(
                        SettingId::InputVolume,
                        tr!("Input volume"),
                        tr!("microphone, gain, recording, level"),
                    ),
                    switch(
                        SettingId::InputMuted,
                        tr!("Mute microphone"),
                        tr!("silence, microphone off"),
                    ),
                    list(
                        SettingId::InputLevel,
                        tr!("Input level"),
                        tr!("microphone, meter, loudness, test"),
                        List::InputLevel,
                    ),
                ],
            ),
            section(
                Some(tr!("Sound effects")),
                vec![
                    setting(
                        SettingId::AlertSound,
                        tr!("Alert sound"),
                        tr!("beep, bell, error, warning"),
                        Kind::Menu(keyword_choices(vec![
                            (tr!("Mallet"), "mallet"),
                            (tr!("Knock"), "knock"),
                            (tr!("Low Mallet"), "low"),
                            (tr!("Tick"), "tick"),
                        ])),
                    ),
                    level(
                        SettingId::AlertVolume,
                        tr!("Alert volume"),
                        tr!("beep, bell, sound effects, loudness"),
                    ),
                    switch(
                        SettingId::SoundEffects,
                        tr!("Play user interface sound effects"),
                        tr!("screenshot, trash, charger, clicks, sounds"),
                    ),
                    switch(
                        SettingId::VolumeFeedback,
                        tr!("Play feedback when volume is changed"),
                        tr!("volume keys, tick, click"),
                    ),
                ],
            ),
            section(
                Some(tr!("Apps playing sound")),
                vec![list(
                    SettingId::AppVolumes,
                    tr!("Apps playing sound"),
                    tr!("per-app volume, application, mixer, stream"),
                    List::AppVolumes,
                )],
            ),
        ],
    )
}

fn keyboard_pane() -> Pane {
    pane(
        PaneId::Keyboard,
        vec![
            section(
                Some(tr!("Input sources")),
                vec![list(
                    SettingId::InputSources,
                    tr!("Input sources"),
                    tr!("keyboard layout, language, input method, switch, dvorak, qwerty, azerty"),
                    List::InputSources,
                )],
            ),
            section(
                Some(tr!("Key repeat")),
                vec![
                    setting(
                        SettingId::KeyRepeatRate,
                        tr!("Repeat rate"),
                        tr!("key repeat, speed, typing, hold"),
                        Kind::Stops {
                            stops: numbers(&REPEAT_RATES),
                            low: tr!("Slow"),
                            high: tr!("Fast"),
                        },
                    ),
                    setting(
                        SettingId::KeyRepeatDelay,
                        tr!("Delay until repeat"),
                        tr!("key repeat, hold, wait, typing"),
                        Kind::Stops {
                            stops: numbers(&REPEAT_DELAYS),
                            low: tr!("Long"),
                            high: tr!("Short"),
                        },
                    ),
                ],
            ),
            section(
                Some(tr!("Spelling")),
                vec![list(
                    SettingId::Spelling,
                    tr!("Spelling"),
                    String::new(),
                    List::Spelling,
                )],
            ),
            section(
                Some(tr!("Text Replacements")),
                vec![list(SettingId::TextReplacements, tr!("Text Replacements"), tr!("text replacement, replace, abbreviation, shortcut, expand, snippet, autotext"),
                    List::TextReplacements,
                )],
            )
            .noted(tr!("As you type, the text on the left is replaced with the text on the right.")),
            section(
                Some(tr!("Modifier keys")),
                vec![setting(
                    SettingId::CapsLock,
                    tr!("Caps Lock key"),
                    tr!("modifier, escape, control, compose, uppercase"),
                    Kind::Menu(keyword_choices(vec![
                        (tr!("Caps Lock"), "caps-lock"),
                        (tr!("Escape"), "escape"),
                        (tr!("Control"), "control"),
                        (tr!("Compose"), "compose"),
                        (tr!("No action"), "off"),
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
        tr!("Tracking speed"),
        keywords,
        Kind::Stops {
            stops: pointer_speed_choices(),
            low: tr!("Slow"),
            high: tr!("Fast"),
        },
    )
}

fn acceleration(id: SettingId) -> Setting {
    switch(
        id,
        tr!("Pointer acceleration"),
        tr!("acceleration, precision, flat, gaming"),
    )
    .described(tr!("Faster movements move the pointer farther"))
}

fn mouse_pane() -> Pane {
    pane(
        PaneId::Mouse,
        vec![section(
            None,
            vec![
                setting(
                    SettingId::MousePrimaryButton,
                    tr!("Primary button"),
                    tr!("left-handed, right-handed, swap buttons, click"),
                    Kind::Choices(keyword_choices(vec![
                        (tr!("Left"), "left"),
                        (tr!("Right"), "right"),
                    ])),
                ),
                setting(
                    SettingId::MouseNaturalScrolling,
                    tr!("Natural scrolling"),
                    tr!("scroll direction, reverse, wheel"),
                    Kind::Switch,
                ),
                speed(SettingId::MouseSpeed, tr!("pointer, cursor, acceleration")),
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
                Some(tr!("Click")),
                vec![
                    switch(
                        SettingId::TrackpadTapToClick,
                        tr!("Tap to click"),
                        tr!("touchpad, tap, click"),
                    ),
                    setting(
                        SettingId::TrackpadDragging,
                        tr!("Dragging"),
                        tr!("drag, tap and drag, drag lock, move"),
                        Kind::Menu(keyword_choices(vec![
                            (tr!("Off"), "off"),
                            (tr!("Tap and drag"), "tap"),
                            (tr!("Tap and drag with lock"), "tap-with-lock"),
                        ])),
                    )
                    .described(tr!("Tap twice and keep the finger down to drag")),
                    setting(
                        SettingId::TrackpadSecondaryClick,
                        tr!("Secondary click"),
                        tr!("right click, context menu, two fingers, corner"),
                        Kind::Choices(keyword_choices(vec![
                            (tr!("Two fingers"), "two-fingers"),
                            (tr!("Bottom right corner"), "corner"),
                        ])),
                    ),
                ],
            ),
            section(
                Some(tr!("Scroll")),
                vec![
                    switch(
                        SettingId::TrackpadNaturalScrolling,
                        tr!("Natural scrolling"),
                        tr!("scroll direction, reverse, touchpad"),
                    ),
                    setting(
                        SettingId::TrackpadScrollMethod,
                        tr!("Scroll with"),
                        tr!("scroll, two fingers, edge scrolling"),
                        Kind::Choices(keyword_choices(vec![
                            (tr!("Two fingers"), "two-fingers"),
                            (tr!("One finger along the edge"), "edge"),
                        ])),
                    ),
                ],
            ),
            section(
                Some(tr!("Pointer")),
                vec![
                    speed(
                        SettingId::TrackpadSpeed,
                        tr!("pointer, cursor, acceleration, touchpad"),
                    ),
                    acceleration(SettingId::TrackpadAcceleration),
                    switch(
                        SettingId::TrackpadDisableWhileTyping,
                        tr!("Ignore the trackpad while typing"),
                        tr!("palm, touchpad, accidental"),
                    ),
                ],
            ),
            section(
                Some(tr!("Gestures")),
                vec![
                    gesture(
                        SettingId::TrackpadWorkspaceSwipe,
                        tr!("Swipe between workspaces"),
                        tr!("workspaces, swipe, three fingers, gesture"),
                        tr!("Swipe left or right with three fingers"),
                    ),
                    gesture(
                        SettingId::TrackpadOverviewSwipe,
                        tr!("Overview"),
                        tr!("overview, all windows, swipe, three fingers, gesture"),
                        tr!("Swipe up or down with three fingers"),
                    ),
                    gesture(
                        SettingId::TrackpadDesktopSpread,
                        tr!("Show desktop"),
                        tr!("desktop, spread, four fingers, gesture"),
                        tr!("Spread four fingers apart"),
                    ),
                    gesture(
                        SettingId::TrackpadLauncherPinch,
                        tr!("Launcher"),
                        tr!("launcher, apps, pinch, four fingers, gesture"),
                        tr!("Pinch four fingers together"),
                    ),
                ],
            ),
        ],
    )
}

fn gesture(id: SettingId, name: String, keywords: String, description: String) -> Setting {
    switch(id, name, keywords).described(description)
}

fn system_panes() -> Vec<Pane> {
    vec![
        power_pane(),
        lock_screen_pane(),
        pane(
            PaneId::Users,
            vec![section(
                None,
                vec![list(SettingId::Accounts, tr!("Accounts"), tr!("change password, password, administrator, standard, login name, picture, photo, full name, add user, new user, delete user, remove user"), List::Accounts)],
            )],
        ),
        pane(
            PaneId::Passwords,
            vec![section(
                None,
                vec![list(SettingId::SavedPasswords, tr!("Saved passwords"), tr!("show password, copy password, change password, delete password, keychain"),
                    List::Passwords,
                )],
            )],
        ),
        pane(
            PaneId::PrivacySecurity,
            vec![
                section(
                    Some(tr!("This computer")),
                    vec![list(SettingId::SecurityStatus, tr!("This computer"), tr!("disk, encryption, encrypted, filevault, luks, remote login, ssh, security"),
                        List::SecurityStatus,
                    )],
                ),
                section(
                    Some(tr!("App permissions")),
                    vec![
                        page(SettingId::CameraPage, PaneId::Camera),
                        page(SettingId::MicrophonePage, PaneId::Microphone),
                        page(SettingId::ScreenshotsPage, PaneId::Screenshots),
                        page(SettingId::ScreenSharingPage, PaneId::ScreenSharing),
                        page(SettingId::LocationPage, PaneId::Location),
                    ],
                ),
                section(
                    Some(tr!("Diagnostics")),
                    vec![page(SettingId::DiagnosticsPage, PaneId::Diagnostics)],
                ),
                section(
                    Some(tr!("Keyboard access")),
                    vec![list(SettingId::KeyboardAccess, tr!("Keyboard access"), tr!("input method, keylogger, typing, fcitx, ibus, virtual keyboard, input monitoring"),
                        List::KeyboardAccess,
                    )],
                ),
                section(
                    Some(tr!("Clipboard")),
                    vec![list(SettingId::ClipboardHistory, tr!("Clipboard history"), tr!("copy, paste, clear clipboard, forget copied, history"),
                        List::Clipboard,
                    )],
                ),
            ],
        ),
        pane(
            PaneId::Apps,
            vec![
                section(
                    Some(tr!("Default apps")),
                    vec![list(SettingId::DefaultApps, tr!("Default apps"), tr!("default browser, web browser, mail, email, music, video, pictures, photos, pdf, text editor, open with, file associations"),
                        List::DefaultApps,
                    )],
                ),
                section(
                    Some(tr!("Open at login")),
                    vec![list(SettingId::LoginItems, tr!("Open at login"), tr!("startup apps, autostart, launch at login, start automatically, when I log in"),
                        List::LoginItems,
                    )],
                ),
                section(
                    Some(tr!("Reopen")),
                    vec![
                        switch(SettingId::ReopenApps, tr!("Reopen windows when logging back in"), tr!("restore, resume, windows, session, log out, restart"))
                            .described(tr!("Apps that were open when you logged out or restarted open again")),
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
        pane(
            PaneId::Backup,
            vec![section(
                None,
                vec![list(SettingId::BackupStatus, tr!("Backup"), tr!("back up, restore, disk, password, pause, oldest, versions"), List::Backup)],
            )],
        ),
        about_pane(),
        pane(
            PaneId::SendReport,
            vec![section(
                None,
                vec![list(SettingId::SendReportForm, tr!("Send Report"), tr!("bug, report, problem, freeze, crash, feedback, send, diagnose"),
                    List::SendReport,
                )],
            )],
        ),
        pane(
            PaneId::Diagnostics,
            vec![section(
                None,
                vec![list(SettingId::DiagnosticsList, tr!("Reports"), tr!("crash, crashes, freeze, hang, quit unexpectedly, kernel panic, core dump, log, diagnostics, report, send, bug"),
                    List::Diagnostics,
                )],
            )],
        ),
        pane(
            PaneId::Storage,
            vec![
                section(
                    None,
                    vec![list(SettingId::StorageUsage, tr!("Storage"), tr!("disk space, free, used, available, full, capacity, trash, empty trash, apps, documents"),
                        List::Storage,
                    )],
                ),
                section(
                    Some(tr!("Largest files")),
                    vec![list(SettingId::LargeFiles, tr!("Largest files"), tr!("large files, big files, biggest, clean up, free up space, reclaim"), List::LargeFiles)],
                ),
            ],
        ),
        pane(
            PaneId::SoftwareUpdate,
            vec![section(
                None,
                vec![
                    list(SettingId::SoftwareUpdates, tr!("Software Update"), tr!("install updates, check for updates, security updates, upgrade, packages, restart"),
                        List::SoftwareUpdates,
                    ),
                    setting(
                        SettingId::UpdateCheckEvery,
                        tr!("Check for updates"),
                        tr!("update check, how often, schedule, interval, security updates"),
                        choice(update_check_choices()),
                    )
                    .described(tr!("Between checks nothing is fetched or installed, even at startup. A longer wait also delays security fixes. Check Now always works.")),
                    switch(SettingId::AutomaticUpdates, tr!("Automatically keep this computer up to date"), tr!("automatic updates, install automatically, background updates, unattended"))
                        .described(tr!("Downloads updates in the background and installs them while the computer is idle. Updates that need your password or a restart wait for you.")),
                ],
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
                match permission {
                    Permission::Camera => tr!("Apps that asked to use the camera"),
                    Permission::Microphone => tr!("Apps that asked to use the microphone"),
                    Permission::Screenshots => tr!("Apps that asked to take screenshots"),
                    Permission::ScreenSharing => tr!("Apps that remember what they may share"),
                    Permission::Location => tr!("Apps that asked for your location"),
                },
                id.keywords(),
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
                Some(tr!("Battery")),
                vec![list(
                    SettingId::BatteryLevel,
                    tr!("Battery level"),
                    tr!("charge, percentage, history, usage, remaining, time left, charging"),
                    List::BatteryHistory,
                )],
            ),
            section(
                Some(tr!("Energy")),
                vec![
                    setting(
                        SettingId::PowerMode,
                        tr!("Power mode"),
                        tr!(
                            "power profile, low power mode, power saver, performance, balanced, fan, speed, energy saving"
                        ),
                        Kind::Choices(keyword_choices(vec![
                            (tr!("Power Saver"), "power-saver"),
                            (tr!("Balanced"), "balanced"),
                            (tr!("Performance"), "performance"),
                        ])),
                    ),
                    switch(
                        SettingId::LowPowerOnBattery,
                        tr!("Use Power Saver on battery"),
                        tr!("low power mode, automatic, unplugged, longer battery life"),
                    ),
                    list(
                        SettingId::PowerStatus,
                        tr!("Power source"),
                        tr!("power adapter, mains, plugged in, energy, sleep, power profiles"),
                        List::PowerStatus,
                    ),
                ],
            ),
            section(
                Some(tr!("On power adapter")),
                vec![setting(
                    SettingId::LidOnPower,
                    tr!("When the lid is closed"),
                    tr!("lid, clamshell, close, external display, keep awake, sleep"),
                    Kind::Menu(keyword_choices(vec![
                        (tr!("Sleep"), "sleep"),
                        (tr!("Keep running"), "keep-running"),
                    ])),
                )],
            ),
        ],
    )
}

pub fn without_sleep(panes: &mut [Pane]) {
    for pane in panes {
        for section in &mut pane.sections {
            section
                .settings
                .retain(|setting| setting.id != SettingId::SleepAfter);
        }
        if pane.id == PaneId::LockScreen {
            pane.summary = tr!("Screen saver, display off and the password");
        }
    }
}

fn lock_screen_pane() -> Pane {
    pane(
        PaneId::LockScreen,
        vec![
            section(
                Some(tr!("When inactive")),
                vec![
                    setting(
                        SettingId::ScreenSaverAfter,
                        tr!("Start Screen Saver when inactive"),
                        tr!("idle, screen saver, screensaver, lock screen, clock"),
                        choice(durations(&SCREEN_SAVER_CHOICES)),
                    ),
                    setting(
                        SettingId::DisplayOffAfter,
                        tr!("Turn display off when inactive"),
                        tr!("idle, display sleep, blank, dim, screen timeout, energy"),
                        choice(durations(&DISPLAY_OFF_CHOICES)),
                    ),
                    setting(
                        SettingId::SleepAfter,
                        tr!("Sleep after"),
                        tr!("idle, suspend, energy, battery, power, computer sleep"),
                        choice(sleep_choices()),
                    ),
                ],
            ),
            section(
                Some(tr!("When locked")),
                vec![
                    setting(
                        SettingId::RequirePasswordAfter,
                        tr!("Require password after screen saver begins or display is turned off"),
                        tr!("idle, password, security, lock screen, auto lock, grace"),
                        choice(password_choices()),
                    ),
                    setting(
                        SettingId::LockMessage,
                        tr!("Message on the lock screen"),
                        tr!("lock screen, message, contact, if found, note, text"),
                        Kind::Text,
                    )
                    .described(tr!("Shown above the buttons while the screen is locked, such as how to return a lost computer")),
                ],
            ),
        ],
    )
}

fn language_time_pane() -> Pane {
    pane(
        PaneId::LanguageTime,
        vec![
            section(
                Some(tr!("Language")),
                vec![list(
                    SettingId::Language,
                    tr!("Language"),
                    tr!(
                        "interface language, translation, localisation, localization, english, swedish, german, french, spanish"
                    ),
                    List::Languages,
                )],
            ),
            section(
                Some(tr!("Apps")),
                vec![list(
                    SettingId::AppLanguages,
                    tr!("Apps"),
                    tr!(
                        "app language, per-app language, language for one app, application language"
                    ),
                    List::AppLanguages,
                )],
            ),
            section(
                Some(tr!("Formats")),
                vec![
                    list(
                        SettingId::Region,
                        tr!("Region"),
                        tr!(
                            "date format, number format, currency, measurement units, metric, imperial, paper size, first day of week, calendar, country"
                        ),
                        List::Regions,
                    ),
                    switch(
                        SettingId::Clock24Hour,
                        tr!("24-hour time"),
                        tr!("12-hour, am, pm, hour format, menu bar clock, lock screen clock"),
                    ),
                ],
            ),
            section(
                Some(tr!("Time")),
                vec![
                    switch(
                        SettingId::AutomaticTime,
                        tr!("Set time automatically"),
                        tr!("ntp, network time, internet time, synchronise, synchronize, sync"),
                    ),
                    list(
                        SettingId::DateAndTime,
                        tr!("Date and time"),
                        tr!("set the date, set the time, manual, wrong time"),
                        List::Clock,
                    ),
                ],
            ),
            section(
                Some(tr!("Time zone")),
                vec![list(
                    SettingId::TimeZone,
                    tr!("Time zone"),
                    tr!(
                        "timezone, zone, city, region, location, utc, gmt, daylight saving, travel"
                    ),
                    List::TimeZones,
                )],
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
                    tr!("Name"),
                    tr!("computer name, hostname, host name, device name, rename"),
                    Kind::Text,
                )],
            ),
            section(
                None,
                vec![list(
                    SettingId::AboutThisComputer,
                    tr!("About this computer"),
                    tr!(
                        "model, chip, processor, cpu, memory, ram, version, debian, bande, serial number"
                    ),
                    List::About,
                )],
            ),
            section(
                None,
                vec![
                    page(SettingId::StoragePage, PaneId::Storage),
                    page(SettingId::SoftwareUpdatePage, PaneId::SoftwareUpdate),
                    page(SettingId::SendReportPage, PaneId::SendReport),
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
