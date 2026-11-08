use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionData, ActionRequest, TreeId, TreeUpdate};
use bande_audio::{Direction, Snapshot, meter_level};
use bande_bus::Reading;
use bande_config::{DisplaySettings, MAX_INPUT_SOURCES, Placement, Rotation};
use bande_keyboard::Layouts;
use bande_l10n::tr;
use bande_settings_panes::{
    AUTOMATIC, Kind, Pane, PaneId, Permission, Setting, SettingId, Value, panes,
};
use bande_settings_panes::{PaneHit, search};
use bande_status::brightness::{DisplayLevel, LevelRequest};
use bande_toolkit::ui::{
    Context, Event, Key, Modifiers, PointerButton, ScrollState, SearchField, Size, Ui, Widget,
    WidgetId,
};
use bande_toolkit::{Appearance, Image, Palette, Rect, SHEET_INSET, Scene, SessionEnd, TextSystem};
use jiff::Zoned;

use crate::about::{About, AboutCommand};
use crate::backup::{BackupIntent, BackupView};
use crate::battery::{BatteryCommand, PowerState};
use crate::defaults::{DefaultApp, DefaultAppCommand};
use crate::diagnostics::{Diagnostics, DiagnosticsCommand};
use crate::follow::{Command, Update};
use crate::forget::Forget;
use crate::language::{SAME_AS_LANGUAGE, locale_matches};
use crate::locale::{DEFAULT_LOCALE, LocaleCommand, LocaleState, same_locale};
use crate::login::{App, LoginCommand, LoginItems, app_matches, login_matches};
use crate::night::clock_text;
use crate::notifications::{AppNotifications, NotificationCommand};
use crate::passwords::{Keychain, PasswordCommand};
use crate::permissions::Grants;
use crate::pictures::{Pair, PictureCommand};
use crate::pointing::PointingDevices;
use crate::printers::{PrinterCommand, PrintersState};
use crate::privacy::focus_after_asking;
use crate::privilege::Privilege;
use crate::services::{AudioCommand, BluetoothCommand, NetworkCommand};
use crate::sharing::{SharingCommand, SharingState};
use crate::software::{Need, SoftwareCommand, SoftwareState};
use crate::state::{Answer, BluetoothState, Display, Joined, NetworkState, Question, WifiState};
use crate::storage::{Storage, StorageCommand};
use crate::time::{TimeCommand, TimeState, parse_manual, zone_matches};
use crate::users::{Account, NewAccountForm, PasswordForm, PictureEdit, UsersCommand};
use crate::widgets::{Column, Crossfade, Root};
use crate::wifi::{HIDDEN_SECURITY, Other};

pub(crate) const PICK_RESULTS: usize = 8;
const SECOND: Duration = Duration::from_secs(1);
pub(crate) const USE_MAIN: &str = "main";
pub(crate) const ADD_COLOUR_PROFILE: &str = "add";
pub(crate) const USE_EXTENDED: &str = "extended";
pub(crate) const MIRROR_PREFIX: &str = "mirror:";

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Intent {
    SelectPane(PaneId),
    Open(PaneId, SettingId),
    StepPane(isize),
    Query(String),
    Set(SettingId, Value),
    Display(String, SettingId, Value),
    Arrange(String, (i32, i32)),
    KeepDisplays(bool),
    Default(Direction, String),
    AppVolume(String, f32),
    VolumeFeedback,
    Join(Vec<u8>),
    Password(String),
    CancelJoin,
    Connect(String),
    Disconnect(String),
    Pair(String),
    AskForget(Option<Forget>),
    Forget,
    Answer(Answer),
    ChoosePicture,
    RemoveSource(usize),
    MoveSource(usize, bool),
    Back,
    Draft(SettingId, String),
    AskEmptyTrash(bool),
    AskClearClipboard(bool),
    ClearClipboard,
    EmptyTrash,
    Backup(BackupIntent),
    Reveal(PathBuf),
    Pick(Option<Picker>),
    PickQuery(String),
    Picked(String),
    ChangeAppLanguage(String),
    RemoveAppLanguage(String),
    RemoveLoginItem(String),
    OpenApp(String),
    SetAppRule(AppNotifications),
    Software(SoftwareCommand),
    Later(Need),
    EndSession(SessionEnd),
    Network(NetworkCommand),
    ManualIpv4(bool),
    OpenNetwork(Vec<u8>),
    AskOther(bool),
    OtherName(String),
    OtherSecurity(usize),
    JoinOther(Option<String>),
    WifiManual(bool),
    OpenPrinter(String),
    OpenReport(String),
    CopyReport(String),
    DeleteReport(String),
    SendReport(String),
    SetSendingReports(bool),
    DraftReportDescription(String),
    CreateReport(()),
    SaveReport((String, String)),
    DefaultPrinter(String),
    Sharing(SharingCommand),
    PrinterAddress(String),
    AddPrinterAddress,
    InstallPrinting,
    CancelJob(u32),
    CancelQueuedJob(i32),
    PausePrinter((String, bool)),
    AskPassword(bool),
    PasswordEntry(usize, String),
    OpenAccount(String),
    ChooseAccountPicture,
    KeepPicture(bool),
    PictureMoved,
    Administrator(String, bool),
    AskNewAccount(bool),
    NewAccountName(String),
    NewAccountLogin(String),
    NewAccountAdmin(bool),
    NewAccountPassword(usize, String),
    AskDeleteAccount(bool),
    DeleteAccount(u32, bool),
    ImportVpn,
    ExcludeFolder,
    IncludeFolder(String),
    Locale(LocaleCommand),
    DefaultApp(&'static str, String),
    Grant(Permission, String, bool),
    KeyboardAccess(String, bool),
    Spelling(crate::spelling::SpellingCommand),
    Battery(BatteryCommand),
    EditReplacement(Option<String>),
    ReplacementText(crate::spelling::ReplacementField, String),
    Passwords(PasswordCommand),
    PasswordQuery(String),
    ChangePassword(Option<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Picker {
    Zone,
    LoginApp,
    InputSource,
    Language,
    Region,
    LanguageApp,
    AppLanguage,
}

impl Picker {
    fn setting(self) -> SettingId {
        match self {
            Self::Zone => SettingId::TimeZone,
            Self::LoginApp => SettingId::LoginItems,
            Self::InputSource => SettingId::InputSources,
            Self::Language => SettingId::Language,
            Self::Region => SettingId::Region,
            Self::LanguageApp | Self::AppLanguage => SettingId::AppLanguages,
        }
    }
}

pub(crate) type Intents = Rc<RefCell<Vec<Intent>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Anchor {
    Pane(PaneId),
    Setting(SettingId, Option<String>),
    Item(String),
    Password,
    Picker,
    Back,
}

pub(crate) fn intent<T>(
    intents: &Intents,
    make: impl Fn(T) -> Intent + 'static,
) -> impl FnMut(T) + 'static {
    let intents = Rc::clone(intents);
    move |argument| intents.borrow_mut().push(make(argument))
}

fn request(action: Action, target: WidgetId, data: Option<ActionData>) -> ActionRequest {
    ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target.into(),
        data,
    }
}

fn plain(modifiers: &Modifiers) -> bool {
    !modifiers.logo && !modifiers.control && !modifiers.alt
}

pub const KEEP_DISPLAY_SETTINGS: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Keep {
    pub(crate) identity: String,
    pub(crate) previous: DisplaySettings,
    pub(crate) until: Instant,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Join {
    pub asking: Option<Vec<u8>>,
    pub joining: Option<Vec<u8>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Detail {
    App(String),
    Network(Vec<u8>),
    Printer(String),
    Account(String),
    Report(String),
}

pub struct SettingsView {
    pub(crate) panes: Vec<Pane>,
    pub(crate) values: HashMap<SettingId, Value>,
    pub(crate) display_settings: BTreeMap<String, DisplaySettings>,
    pub(crate) display_levels: Vec<DisplayLevel>,
    pub(crate) arrangements: Vec<Vec<Placement>>,
    pub(crate) displays: Vec<Display>,
    pub(crate) keep: Option<Keep>,
    pub(crate) audio: Option<Snapshot>,
    pub(crate) input_level: Rc<Cell<f32>>,
    pub(crate) wifi: Reading<WifiState>,
    pub(crate) join: Join,
    pub(crate) bluetooth: Reading<BluetoothState>,
    pub(crate) busy: Vec<String>,
    pub(crate) pairing: Option<Question>,
    pub(crate) pictures: Vec<(String, Pair)>,
    pub(crate) colour_profiles: Vec<(String, String)>,
    pub(crate) show_all_resolutions: bool,
    pub(crate) thumbnails: HashMap<Pair, Option<Image>>,
    pub(crate) query: String,
    pub(crate) hits: Vec<PaneHit>,
    pub(crate) pane: PaneId,
    pub(crate) layouts: Option<Layouts>,
    pub(crate) pointing: PointingDevices,
    pub(crate) about: Option<About>,
    pub(crate) diagnostics: Diagnostics,
    report_wanted: Option<String>,
    pub(crate) storage: Option<Storage>,
    pub(crate) files_installed: bool,
    pub(crate) backup: BackupView,
    pub(crate) drafts: HashMap<SettingId, String>,
    pub(crate) confirming_empty_trash: bool,
    pub(crate) confirming_clear_clipboard: bool,
    pub(crate) forgetting: Option<Forget>,
    pub(crate) time: Option<TimeState>,
    pub(crate) picking: Option<(Picker, String)>,
    pub(crate) login: Option<LoginItems>,
    pub(crate) language_app: Option<String>,
    pub(crate) power: Option<PowerState>,
    pub(crate) sleeps: bool,
    pub(crate) notification_apps: Option<Vec<AppNotifications>>,
    pub(crate) detail: Option<Detail>,
    pub(crate) other: Option<Other>,
    pub(crate) wifi_manual: bool,
    pub(crate) printers: Option<PrintersState>,
    pub(crate) sharing: Option<SharingState>,
    pub(crate) installing_sharing: bool,
    pub(crate) installing_vpn_plugin: bool,
    pub(crate) adding_printer: bool,
    pub(crate) installing_printing: bool,
    pub(crate) printer_address: String,
    pub(crate) accounts: Option<Vec<Account>>,
    pub(crate) accounts_managed: bool,
    pub(crate) picture_edit: Option<PictureEdit>,
    pub(crate) new_account: Option<NewAccountForm>,
    pub(crate) deleting_account: bool,
    pub(crate) password: Option<PasswordForm>,
    pub(crate) password_message: Option<String>,
    pub(crate) software: SoftwareState,
    pub(crate) software_checked: Option<i64>,
    pub(crate) software_hint: Option<String>,
    pub(crate) network: Option<NetworkState>,
    pub(crate) manual_ipv4: bool,
    pub(crate) locale: Option<LocaleState>,
    ui: Ui,
    root: WidgetId,
    search: WidgetId,
    pub(crate) anchors: Vec<(Anchor, WidgetId)>,
    pub(crate) intents: Intents,
    pub(crate) commands: Vec<Command>,
    pub(crate) failures: HashMap<PaneId, String>,
    pub(crate) errors: HashMap<SettingId, String>,
    pub(crate) refused: Vec<Privilege>,
    pub(crate) default_apps: Option<Vec<DefaultApp>>,
    pub(crate) permissions: Grants,
    pub(crate) keyboard_access: Vec<crate::consent::KeyboardAccess>,
    pub(crate) spelling: crate::spelling::SpellingState,
    pub(crate) replacing: Option<crate::spelling::Replacing>,
    pub(crate) security: crate::security_status::SecurityStatus,
    pub(crate) keychain: Keychain,
    pub(crate) stale: bool,
    rebuilt: bool,
    pub(crate) focus_next: Option<Anchor>,
    reveal_pending: bool,
    pub(crate) scroll: Rc<RefCell<ScrollState>>,
    pub(crate) page_view: Option<WidgetId>,
    pub(crate) crossfade: Rc<RefCell<Crossfade>>,
    pub(crate) shown: Option<(PaneId, bool)>,
    pub(crate) sidebar_scroll: Rc<RefCell<ScrollState>>,
    pub(crate) icon_dirs: Vec<PathBuf>,
    pub(crate) symbols: HashMap<Symbol, Option<Image>>,
    pub(crate) clock: Box<dyn Fn() -> Zoned>,
    on_screen: bool,
}

pub(crate) type Symbol = (&'static str, u32, [u8; 4]);

impl SettingsView {
    pub fn new(palette: Palette, text: TextSystem) -> Self {
        let intents: Intents = Rc::default();
        let queries = Rc::clone(&intents);
        let field = SearchField::new(&tr!("Search"), "", move |query: &str| {
            queries.borrow_mut().push(Intent::Query(query.to_owned()));
        });
        let search = field.id();
        let placeholder = || Box::new(Column::new(0.0, 0.0, Vec::new())) as Box<dyn Widget>;
        let root = Root::new(Box::new(field), placeholder(), placeholder());
        let root_id = root.id();
        let mut cx = Context::new(text, palette);
        cx.keyboard_navigation = true;
        let mut view = Self {
            panes: panes(),
            values: HashMap::new(),
            display_settings: BTreeMap::new(),
            display_levels: Vec::new(),
            arrangements: Vec::new(),
            displays: Vec::new(),
            keep: None,
            audio: None,
            input_level: Rc::default(),
            wifi: Reading::Absent,
            join: Join::default(),
            bluetooth: Reading::Absent,
            busy: Vec::new(),
            pairing: None,
            pictures: Vec::new(),
            colour_profiles: Vec::new(),
            show_all_resolutions: false,
            thumbnails: HashMap::new(),
            query: String::new(),
            hits: Vec::new(),
            pane: PaneId::Appearance,
            layouts: None,
            pointing: PointingDevices::default(),
            about: None,
            diagnostics: Diagnostics::default(),
            report_wanted: None,
            storage: None,
            files_installed: false,
            backup: BackupView::default(),
            drafts: HashMap::new(),
            confirming_empty_trash: false,
            confirming_clear_clipboard: false,
            forgetting: None,
            time: None,
            picking: None,
            login: None,
            language_app: None,
            power: None,
            sleeps: true,
            notification_apps: None,
            detail: None,
            other: None,
            wifi_manual: false,
            printers: None,
            sharing: None,
            installing_sharing: false,
            installing_vpn_plugin: false,
            adding_printer: false,
            installing_printing: false,
            printer_address: String::new(),
            accounts: None,
            accounts_managed: false,
            picture_edit: None,
            new_account: None,
            deleting_account: false,
            password: None,
            password_message: None,
            software: SoftwareState::Unknown,
            software_checked: None,
            software_hint: None,
            network: None,
            manual_ipv4: false,
            locale: None,
            ui: Ui::new(Box::new(root), cx),
            root: root_id,
            search,
            anchors: Vec::new(),
            intents,
            commands: Vec::new(),
            failures: HashMap::new(),
            errors: HashMap::new(),
            refused: Vec::new(),
            default_apps: None,
            permissions: Vec::new(),
            keyboard_access: Vec::new(),
            spelling: Default::default(),
            replacing: None,
            security: crate::security_status::SecurityStatus::default(),
            keychain: Keychain::default(),
            stale: true,
            rebuilt: false,
            focus_next: None,
            reveal_pending: false,
            scroll: Rc::default(),
            page_view: None,
            crossfade: Rc::default(),
            shown: None,
            sidebar_scroll: Rc::default(),
            icon_dirs: bande_xdg::data_dirs(),
            symbols: HashMap::new(),
            clock: Box::new(bande_datetime::now),
            on_screen: true,
        };
        view.rebuild();
        view
    }

    pub fn set_window_active(&mut self, active: bool) {
        if self.ui.set_window_active(active) {
            self.invalidate();
        }
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        let cx = self.ui.context();
        let changed = cx.palette != appearance.palette()
            || cx.reduce_motion != appearance.reduce_motion
            || cx.text.size() != appearance.text_size;
        if changed {
            self.ui.set_appearance(appearance);
            self.invalidate();
        }
    }

    pub fn set_clock(&mut self, clock: impl Fn() -> Zoned + 'static) {
        self.clock = Box::new(clock);
        self.invalidate();
    }

    pub fn set_value(&mut self, id: SettingId, value: Value) {
        if self.values.get(&id) != Some(&value) {
            self.values.insert(id, value);
            self.invalidate();
        }
    }

    pub fn value(&self, id: SettingId) -> Option<&Value> {
        self.values.get(&id)
    }

    pub fn failure(&self) -> Option<&str> {
        self.failures.get(&self.pane).map(String::as_str)
    }

    pub(crate) fn clear_failure(&mut self) {
        self.failures.remove(&self.pane);
    }

    pub(crate) fn fail_here(&mut self, failure: String) {
        self.failures.insert(self.pane, failure);
    }

    pub fn set_pointing_devices(&mut self, devices: PointingDevices) {
        if self.pointing != devices {
            self.pointing = devices;
            self.availability_changed();
        }
    }

    pub fn hide_screen_reader(&mut self) {
        self.remove_setting(SettingId::ScreenReader);
    }

    fn remove_setting(&mut self, id: SettingId) {
        for section in self.panes.iter_mut().flat_map(|pane| &mut pane.sections) {
            section.settings.retain(|setting| setting.id != id);
        }
        self.availability_changed();
    }

    pub(crate) fn offers_screen_reader(&self) -> bool {
        self.panes
            .iter()
            .flat_map(Pane::settings)
            .any(|setting| setting.id == SettingId::ScreenReader)
    }

    pub fn picking(&self) -> Option<&str> {
        self.picking.as_ref().map(|(_, query)| query.as_str())
    }

    pub(crate) fn input_sources(&self) -> Vec<String> {
        match self.values.get(&SettingId::InputSources) {
            Some(Value::Codes(codes)) => codes.clone(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn excluded_folders(&self) -> Vec<String> {
        match self.values.get(&SettingId::SearchExcluded) {
            Some(Value::Codes(folders)) => folders.clone(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn layouts(&self) -> &Layouts {
        static UNLOADED: LazyLock<Layouts> = LazyLock::new(Layouts::default);
        self.layouts.as_ref().unwrap_or(&UNLOADED)
    }

    fn edit_sources(&mut self, edit: impl FnOnce(&mut Vec<String>)) {
        let mut sources = self.input_sources();
        edit(&mut sources);
        if !sources.is_empty() && sources.len() <= MAX_INPUT_SOURCES {
            self.change(SettingId::InputSources, Value::Codes(sources));
        }
    }

    pub fn set_files_installed(&mut self, installed: bool) {
        if self.files_installed != installed {
            self.files_installed = installed;
            self.invalidate();
        }
    }

    pub fn set_displays(&mut self, displays: Vec<Display>) {
        if self.displays != displays {
            self.displays = displays;
            self.availability_changed();
        }
    }

    pub fn apply(&mut self, updates: Vec<Update>) {
        let mut changes = Vec::new();
        for update in updates {
            match update {
                Update::InputLevel(peak) => {
                    self.input_level
                        .set(meter_level(peak, self.input_level.get()));
                }
                update => changes.push(update),
            }
        }
        if changes.is_empty() {
            return;
        }
        let before = self.outline();
        let mut shown = false;
        let mut answered = false;
        for update in changes {
            shown |= self.shows(&update);
            answered |= matches!(
                update,
                Update::Joined(..) | Update::Password(_) | Update::AccountCreated(_)
            );
            self.take(update);
        }
        self.refresh_hits();
        if answered || ((shown || self.outline() != before) && !self.typing_secret()) {
            self.invalidate();
        }
    }

    fn typing_secret(&self) -> bool {
        let Some(focus) = self.ui.focus() else {
            return false;
        };
        self.anchors.iter().any(|(anchor, id)| {
            *id == focus
                && match anchor {
                    Anchor::Password => true,
                    Anchor::Item(item) => item.starts_with("password-"),
                    _ => false,
                }
        })
    }

    fn outline(&self) -> (PaneId, Vec<PaneId>, Vec<bool>, Option<String>) {
        let available = self
            .panes
            .iter()
            .map(|pane| self.available(pane.id))
            .collect();
        (self.pane, self.visible_panes(), available, self.attention())
    }

    fn shows(&self, update: &Update) -> bool {
        let (owner, settings): (Option<PaneId>, &[SettingId]) = match update {
            Update::Wifi(_) => (
                Some(PaneId::Wifi),
                &[
                    SettingId::WifiPower,
                    SettingId::KnownNetworks,
                    SettingId::Networks,
                ],
            ),
            Update::Bluetooth(_) => (
                Some(PaneId::Bluetooth),
                &[
                    SettingId::BluetoothPower,
                    SettingId::PairedDevices,
                    SettingId::NearbyDevices,
                ],
            ),
            Update::Network(_) => (
                Some(PaneId::Network),
                &[
                    SettingId::Ethernet,
                    SettingId::Vpns,
                    SettingId::EthernetAddress,
                    SettingId::EthernetRouter,
                    SettingId::EthernetDns,
                ],
            ),
            Update::Audio(_) => (
                Some(PaneId::Sound),
                &[
                    SettingId::OutputDevice,
                    SettingId::OutputBalance,
                    SettingId::OutputVolume,
                    SettingId::OutputMuted,
                    SettingId::InputDevice,
                    SettingId::InputVolume,
                    SettingId::InputMuted,
                    SettingId::InputLevel,
                    SettingId::AppVolumes,
                ],
            ),
            Update::Power(_) => (
                Some(PaneId::Power),
                &[
                    SettingId::PowerStatus,
                    SettingId::BatteryLevel,
                    SettingId::PowerMode,
                    SettingId::LowPowerOnBattery,
                    SettingId::LidOnPower,
                ],
            ),
            Update::Storage(_) => (
                Some(PaneId::Storage),
                &[SettingId::StorageUsage, SettingId::LargeFiles],
            ),
            Update::Backup(_) => (Some(PaneId::Backup), &[SettingId::BackupStatus]),
            Update::Software(_) | Update::SoftwareChecked(_) | Update::SoftwareHint(_) => {
                (Some(PaneId::SoftwareUpdate), &[SettingId::SoftwareUpdates])
            }
            Update::Pictures(_) | Update::Thumbnail(..) => (None, &[SettingId::Wallpaper]),
            Update::ColourProfiles(_) | Update::ColourProfileAdded(..) => {
                (None, &[SettingId::ColourProfile])
            }
            Update::Sharing(_) => (Some(PaneId::Sharing), &[SettingId::Sharing]),
            Update::Printers(_) | Update::PrinterAdded(..) | Update::PrintingInstalled => (
                Some(PaneId::Printers),
                &[SettingId::Printers, SettingId::NearbyPrinters],
            ),
            Update::Accounts(..)
            | Update::Password(_)
            | Update::PictureChosen(..)
            | Update::AccountCreated(_) => (Some(PaneId::Users), &[SettingId::Accounts]),
            Update::DisplayLevels(_) => (Some(PaneId::Displays), &[SettingId::Brightness]),
            Update::Passwords(..) | Update::RevealedPassword(..) => {
                (Some(PaneId::Passwords), &[SettingId::SavedPasswords])
            }
            _ => return true,
        };
        let feeds = |setting: &Setting| {
            settings.contains(&setting.id)
                || matches!(setting.kind, Kind::Page(child) if Some(child) == owner)
        };
        owner == Some(self.pane)
            || self
                .panes
                .iter()
                .find(|pane| pane.id == self.pane)
                .is_some_and(|pane| pane.settings().any(feeds))
    }

    fn take(&mut self, update: Update) {
        match update {
            Update::InputLevel(_) => {}
            Update::Layouts(layouts) => self.layouts = Some(layouts),
            Update::Values(values) => {
                for (id, value) in values {
                    self.values.insert(id, value);
                }
            }
            Update::Displays(displays) => self.display_settings = displays,
            Update::DisplayLevels(levels) => self.display_levels = levels,
            Update::Arrangements(arrangements) => self.arrangements = arrangements,
            Update::Audio(snapshot) => self.apply_audio(snapshot),
            Update::Wifi(wifi) => {
                match wifi.present() {
                    Some(wifi) => self
                        .values
                        .insert(SettingId::WifiPower, Value::Flag(wifi.enabled)),
                    None => self.values.remove(&SettingId::WifiPower),
                };
                if let Some(Detail::Network(ssid)) = &self.detail
                    && !wifi.present().is_some_and(|wifi| {
                        wifi.enabled && wifi.known.iter().any(|known| known.ssid == *ssid)
                    })
                {
                    self.detail = None;
                }
                self.wifi = wifi;
            }
            Update::Joined(ssid, outcome) => self.joined(ssid, outcome),
            Update::Bluetooth(bluetooth) => {
                if bluetooth.present().is_none() {
                    self.values.remove(&SettingId::BluetoothPower);
                }
                if let Some(bluetooth) = bluetooth.present() {
                    self.values
                        .insert(SettingId::BluetoothPower, Value::Flag(bluetooth.powered));
                    let connection = |state: Option<&BluetoothState>, address: &str| {
                        state?
                            .paired
                            .iter()
                            .find(|device| device.address == address)
                            .map(|device| device.connected)
                    };
                    let before = self.bluetooth.present();
                    self.busy.retain(|address| {
                        connection(before, address) == connection(Some(bluetooth), address)
                    });
                }
                self.bluetooth = bluetooth;
            }
            Update::Pairing(question) => {
                if question.is_some() {
                    self.focus_next = Some(Anchor::Password);
                }
                self.pairing = question;
            }
            Update::Pictures(pictures) => self.pictures = pictures,
            Update::ColourProfiles(profiles) => self.colour_profiles = profiles,
            Update::ColourProfileAdded(identity, profile) => {
                self.change_display(identity, SettingId::ColourProfile, Value::Keyword(profile));
            }
            Update::Thumbnail(pair, picture) => {
                let image = Image::from_rgba(picture.width, picture.height, picture.rgba);
                self.thumbnails.insert(pair, image);
            }
            Update::About(about) => {
                if !self.drafts.contains_key(&SettingId::ComputerName) {
                    self.values
                        .insert(SettingId::ComputerName, Value::Text(about.name.clone()));
                }
                self.about = Some(about);
            }
            Update::Time(time) => {
                if let Some(time) = &time {
                    self.values
                        .insert(SettingId::AutomaticTime, Value::Flag(time.automatic));
                }
                self.time = time;
            }
            Update::Login(login) => self.login = Some(login),
            Update::NotificationApps(apps) => self.notification_apps = Some(apps),
            Update::Software(state) => self.software = state,
            Update::SoftwareChecked(at) => self.software_checked = at,
            Update::SoftwareHint(hint) => self.software_hint = hint,
            Update::Locale(locale) => self.locale = locale,
            Update::Pointing(devices) => self.set_pointing_devices(devices),
            Update::Network(network) => {
                if let Some(ethernet) = network.ethernet.as_ref() {
                    self.manual_ipv4 |= ethernet.manual;
                    for (id, value) in [
                        (
                            SettingId::EthernetAddress,
                            ethernet.addressing.address.clone(),
                        ),
                        (
                            SettingId::EthernetRouter,
                            ethernet.addressing.router.clone(),
                        ),
                        (
                            SettingId::EthernetDns,
                            Some(if self.manual_ipv4 {
                                ethernet.addressing.dns.join(", ")
                            } else {
                                ethernet
                                    .dns
                                    .iter()
                                    .map(ToString::to_string)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            }),
                        ),
                    ] {
                        if !self.manual_ipv4 || !self.values.contains_key(&id) {
                            self.values
                                .insert(id, Value::Text(value.unwrap_or_default()));
                        }
                    }
                }
                self.network = Some(network);
            }
            Update::Power(power) => {
                match power.as_ref().and_then(|power| power.profile.clone()) {
                    Some(profile) => {
                        self.values
                            .insert(SettingId::PowerMode, Value::Keyword(profile));
                    }
                    None => {
                        self.values.remove(&SettingId::PowerMode);
                    }
                }
                self.power = power;
            }
            Update::CannotSleep => {
                self.sleeps = false;
                bande_settings_panes::without_sleep(&mut self.panes);
                self.availability_changed();
            }
            Update::Diagnostics(diagnostics) => {
                let open = self.diagnostics.open.take();
                self.diagnostics = diagnostics;
                self.diagnostics.open = open;
                if let Some(wanted) = self.report_wanted.take()
                    && let Some(id) = self
                        .diagnostics
                        .reports
                        .iter()
                        .find(|report| report.id == wanted || report.program == wanted)
                        .map(|report| report.id.clone())
                {
                    self.open_report(id);
                }
            }
            Update::DiagnosticsReport(id, text) => self.diagnostics.open = Some((id, text)),
            Update::ReportCreated(id) => {
                self.select_pane(PaneId::Diagnostics);
                self.detail = Some(Detail::Report(id));
                self.focus_next = Some(Anchor::Back);
            }
            Update::ReportSaved(path) => self.diagnostics.saved = Some(path),
            Update::Storage(storage) => {
                if storage.trash_items == 0 {
                    self.confirming_empty_trash = false;
                }
                self.storage = Some(storage);
            }
            Update::Backup(update) => self.backup_updated(update),
            Update::Failed(setting, error) => {
                self.busy.clear();
                self.errors.insert(setting, error);
            }
            Update::PaneFailed(pane, error) => {
                self.busy.clear();
                if pane == self.pane {
                    *self.scroll.borrow_mut() = ScrollState::default();
                }
                self.failures.insert(pane, error);
            }
            Update::Refused(refused) => self.refused = refused,
            Update::DefaultApps(defaults) => self.default_apps = Some(defaults),
            Update::Permissions(permissions) => self.permissions = permissions,
            Update::InstallingVpnPlugin(installing) => self.installing_vpn_plugin = installing,
            Update::Sharing(state) => {
                self.installing_sharing = false;
                self.sharing = Some(state);
            }
            Update::KeyboardAccess(access) => self.keyboard_access = access,
            Update::Spelling(state) => self.spelling = state,
            Update::Security(status) => self.security = status,
            Update::Printers(printers) => {
                if let Some(Detail::Printer(id)) = &self.detail
                    && !printers.printers.iter().any(|printer| printer.id == *id)
                {
                    self.detail = None;
                }
                self.printers = Some(printers);
            }
            Update::PrintingInstalled => self.installing_printing = false,
            Update::Accounts(accounts, managed) => self.take_accounts(accounts, managed),
            Update::PictureChosen(login, image) => self.picture_chosen(login, image),
            Update::AccountCreated(error) => self.account_created(error),
            Update::Passwords(unlocked, items) => self.take_passwords(unlocked, items),
            Update::RevealedPassword(path, secret) => self.keychain.revealed = Some((path, secret)),
            Update::CopiedPassword(secret) => self.commands.push(Command::CopySecret(secret)),
            Update::Password(outcome) => self.password_changed(outcome),
            Update::PrinterAdded(added) => {
                self.adding_printer = false;
                if added {
                    self.printer_address.clear();
                }
            }
        }
    }

    fn apply_audio(&mut self, snapshot: Option<Snapshot>) {
        if let Some(snapshot) = &snapshot {
            for (device, volume, muted) in [
                (
                    snapshot.default_output(),
                    SettingId::OutputVolume,
                    SettingId::OutputMuted,
                ),
                (
                    snapshot.default_input(),
                    SettingId::InputVolume,
                    SettingId::InputMuted,
                ),
            ] {
                match device {
                    Some(device) => {
                        self.values
                            .insert(volume, Value::Fraction(device.level.volume));
                        self.values.insert(muted, Value::Flag(device.level.muted));
                    }
                    None => {
                        self.values.remove(&volume);
                        self.values.remove(&muted);
                    }
                }
            }
            match snapshot.default_output().and_then(|output| output.balance) {
                Some(balance) => {
                    self.values.insert(
                        SettingId::OutputBalance,
                        Value::Number(((balance + 1.0) * 50.0).round() as u32),
                    );
                }
                None => {
                    self.values.remove(&SettingId::OutputBalance);
                }
            }
        }
        self.audio = snapshot.filter(|snapshot| snapshot.connected);
    }

    fn joined(&mut self, ssid: Vec<u8>, outcome: Joined) {
        if self.join.joining.as_ref() != Some(&ssid) {
            return;
        }
        self.join.joining = None;
        match outcome {
            Joined::Joined => {
                self.join.error = None;
                self.other = None;
            }
            Joined::WrongPassword => {
                self.join.error = Some(tr!("The password is not correct. Try again."));
                if self.other.is_none() {
                    self.join.asking = Some(ssid);
                }
                self.focus_next = Some(Anchor::Password);
            }
            Joined::Failed(reason) => {
                self.join.error = Some(tr!(
                    "Could not join the network: {reason}.",
                    reason = reason
                ));
            }
        }
    }

    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn available(&self, pane: PaneId) -> bool {
        match pane {
            PaneId::Wifi => !self.wifi.is_absent(),
            PaneId::Bluetooth => !self.bluetooth.is_absent(),
            PaneId::Network => self.network.is_some(),
            PaneId::Notifications => self.values.contains_key(&SettingId::DoNotDisturb),
            PaneId::Sound => self.audio.is_some(),
            PaneId::Displays | PaneId::NightLight => !self.displays.is_empty(),
            PaneId::About => self.about.is_some(),
            PaneId::Appearance
            | PaneId::Accessibility
            | PaneId::Bar
            | PaneId::Search
            | PaneId::Keyboard
            | PaneId::KeyboardShortcuts
            | PaneId::Power
            | PaneId::LockScreen
            | PaneId::PrivacySecurity
            | PaneId::Camera
            | PaneId::Screenshots
            | PaneId::ScreenSharing
            | PaneId::Apps
            | PaneId::Printers
            | PaneId::Sharing
            | PaneId::Users
            | PaneId::Passwords
            | PaneId::Storage
            | PaneId::Diagnostics
            | PaneId::SendReport
            | PaneId::Backup
            | PaneId::SoftwareUpdate => true,
            PaneId::LanguageTime => self.locale.is_some() || self.time.is_some(),
            PaneId::Mouse => self.pointing.mouse,
            PaneId::Trackpad => self.pointing.trackpad,
            PaneId::Microphone | PaneId::Location => self.has_grants(pane),
        }
    }

    pub fn visible_panes(&self) -> Vec<PaneId> {
        if self.query.trim().is_empty() {
            self.panes
                .iter()
                .map(|pane| pane.id)
                .filter(|pane| pane.parent().is_none() && self.available(*pane))
                .collect()
        } else {
            self.hits.iter().map(|hit| hit.pane).collect()
        }
    }

    pub fn focused_setting(&self) -> Option<SettingId> {
        let focus = self.ui.focus()?;
        self.anchors.iter().find_map(|(anchor, id)| match anchor {
            Anchor::Setting(setting, _) if *id == focus => Some(*setting),
            _ => None,
        })
    }

    pub(crate) fn anchor_focused(&self, wanted: &Anchor) -> bool {
        let focus = self.ui.focus();
        self.anchors
            .iter()
            .any(|(anchor, id)| anchor == wanted && Some(*id) == focus)
    }

    pub(crate) fn pick_results(&mut self) -> Vec<String> {
        if let Some((Picker::InputSource, query)) = self.picking.clone() {
            return self.source_matches(&query);
        }
        match (&self.picking, &self.time) {
            (Some((Picker::Language, query)), _) => self
                .locale
                .as_ref()
                .map(|locale| {
                    locale_matches(&locale.available, &locale.translations, query, PICK_RESULTS)
                })
                .unwrap_or_default(),
            (Some((Picker::Region, query)), _) if query.trim().is_empty() => {
                vec![SAME_AS_LANGUAGE.to_owned()]
            }
            (Some((Picker::Region, query)), _) => self
                .locale
                .as_ref()
                .map(|locale| locale_matches(&locale.available, &[], query, PICK_RESULTS))
                .unwrap_or_default(),
            (Some((Picker::Zone, query)), Some(time)) => {
                zone_matches(&time.zones, query, PICK_RESULTS)
            }
            (Some((Picker::AppLanguage, query)), _) => self
                .locale
                .as_ref()
                .map(|locale| {
                    let mut matches = locale_matches(
                        &locale.available,
                        &locale.translations,
                        query,
                        PICK_RESULTS + 1,
                    );
                    matches.retain(|code| !same_locale(code, DEFAULT_LOCALE));
                    matches.truncate(PICK_RESULTS);
                    matches
                })
                .unwrap_or_default(),
            (Some((Picker::LanguageApp, query)), _) => self
                .login
                .as_ref()
                .map(|login| {
                    let chosen =
                        |app: &App| login.languages.iter().any(|(known, _)| known.id == app.id);
                    app_matches(&login.apps, chosen, query, PICK_RESULTS)
                })
                .unwrap_or_default(),
            (Some((Picker::LoginApp, query)), _) => self
                .login
                .as_ref()
                .map(|login| login_matches(login, query, PICK_RESULTS))
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    pub fn search_focused(&self) -> bool {
        self.ui.focus() == Some(self.search)
    }

    fn shown_in(&self, visible: &[PaneId]) -> bool {
        visible.contains(&self.pane)
            || (self.available(self.pane)
                && self
                    .pane
                    .parent()
                    .is_some_and(|parent| visible.contains(&parent)))
    }

    pub(crate) fn hit(&self, pane: PaneId) -> Option<&PaneHit> {
        self.hits.iter().find(|hit| hit.pane == pane)
    }

    fn availability_changed(&mut self) {
        self.refresh_hits();
        self.invalidate();
    }

    fn refresh_hits(&mut self) {
        if !self.query.is_empty() {
            self.hits = self.search_hits(&self.query.clone());
            let visible = self.visible_panes();
            if !self.shown_in(&visible)
                && let Some(&first) = visible.first()
            {
                self.select_pane(first);
            }
        }
    }

    fn invalidate(&mut self) {
        self.stale = true;
        if !self.ui.pointer_captured() {
            self.rebuild();
        }
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        let editing = self
            .focused_setting()
            .filter(|setting| self.drafts.contains_key(setting));
        let replacing = self.replacing.is_some() && self.replacement_focused();
        let handled = match event {
            Event::Key {
                key: Key::Enter, ..
            } if replacing => {
                self.commit_replacement();
                self.invalidate();
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && replacing => {
                self.cancel_replacement();
                self.invalidate();
                true
            }
            Event::Key {
                key: Key::Enter, ..
            } if editing.is_some() => {
                self.commit_drafts();
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && editing.is_some() => {
                self.drafts.clear();
                self.invalidate();
                true
            }
            Event::Key {
                key: Key::Character('f'),
                modifiers,
            } if modifiers.control && !modifiers.logo && !modifiers.alt => {
                self.ui.perform(&request(Action::Focus, self.search, None));
                true
            }
            Event::Key {
                key: Key::Left,
                modifiers,
            } if modifiers.alt && self.can_go_back() => {
                self.intents.borrow_mut().push(Intent::Back);
                true
            }
            Event::PointerDown(pointer)
                if pointer.button == PointerButton::Back && self.can_go_back() =>
            {
                self.intents.borrow_mut().push(Intent::Back);
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && self.confirming_empty_trash => {
                self.intents.borrow_mut().push(Intent::AskEmptyTrash(false));
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && self.forgetting.is_some() => {
                self.intents.borrow_mut().push(Intent::AskForget(None));
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && self.confirming_clear_clipboard => {
                self.intents
                    .borrow_mut()
                    .push(Intent::AskClearClipboard(false));
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && self.join.asking.is_some() => {
                self.intents.borrow_mut().push(Intent::CancelJoin);
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && self.other.is_some() => {
                self.intents.borrow_mut().push(Intent::AskOther(false));
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && self.password.is_some() => {
                self.intents.borrow_mut().push(Intent::AskPassword(false));
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && self.picture_edit.is_some() => {
                self.intents.borrow_mut().push(Intent::KeepPicture(false));
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && self.new_account.is_some() => {
                self.intents.borrow_mut().push(Intent::AskNewAccount(false));
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && self.deleting_account => {
                self.intents
                    .borrow_mut()
                    .push(Intent::AskDeleteAccount(false));
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && self.picking.is_some() => {
                self.intents.borrow_mut().push(Intent::Pick(None));
                true
            }
            Event::Key {
                key: Key::Enter, ..
            } if self.anchor_focused(&Anchor::Picker) => {
                if let Some(first) = self.pick_results().into_iter().next() {
                    self.intents.borrow_mut().push(Intent::Picked(first));
                }
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && self.pairing.is_some() => {
                self.intents
                    .borrow_mut()
                    .push(Intent::Answer(Answer::Decline));
                true
            }
            Event::Key {
                key: Key::Escape,
                modifiers,
            } if plain(modifiers) && !self.query.is_empty() => {
                self.ui.perform(&request(
                    Action::SetValue,
                    self.search,
                    Some(ActionData::Value("".into())),
                ));
                self.intents.borrow_mut().push(Intent::Query(String::new()));
                true
            }
            Event::Key {
                key: key @ (Key::Up | Key::Down),
                ..
            } if self.search_focused() => {
                let step = if *key == Key::Up { -1 } else { 1 };
                self.intents.borrow_mut().push(Intent::StepPane(step));
                true
            }
            Event::Key {
                key: Key::Enter, ..
            } if self.search_focused() => {
                self.focus_best_match();
                true
            }
            Event::Key {
                key: key @ (Key::PageUp | Key::PageDown),
                ..
            } => {
                let action = match key {
                    Key::PageUp => Action::ScrollUp,
                    _ => Action::ScrollDown,
                };
                self.page_view
                    .is_some_and(|page| self.ui.perform(&request(action, page, None)))
            }
            _ => self.ui.handle(event),
        };
        let handled = handled || self.type_into_search(event);
        self.settle() | handled
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    pub fn set_full_screen(&mut self, full_screen: bool) {
        let inset = match full_screen {
            true => 0.0,
            false => SHEET_INSET as f32,
        };
        if let Some(root) = self.ui.widget_as::<Root>(self.root) {
            root.set_inset(inset);
        }
    }

    pub fn settle(&mut self) -> bool {
        self.rebuilt = false;
        if self.replacing.is_some() && !self.replacement_focused() {
            self.commit_replacement();
        }
        self.process_intents();
        let focused = self.focused_setting();
        if self.drafts.keys().any(|setting| Some(*setting) != focused) {
            self.commit_drafts();
        }
        if self.stale && !self.ui.pointer_captured() {
            self.rebuild();
        }
        self.rebuilt
    }

    fn type_into_search(&mut self, event: &Event) -> bool {
        let Event::Key {
            key: Key::Character(_),
            modifiers,
        } = event
        else {
            return false;
        };
        if !plain(modifiers) || self.search_focused() {
            return false;
        }
        self.ui.perform(&request(Action::Focus, self.search, None));
        self.ui.handle(&Event::Key {
            key: Key::End,
            modifiers: Modifiers::default(),
        });
        self.ui.handle(event)
    }

    fn focus_best_match(&mut self) {
        if let Some(best) = self.hit(self.pane).and_then(PaneHit::best) {
            self.focus_next = Some(Anchor::Setting(best.setting, None));
            self.invalidate();
        }
    }

    fn process_intents(&mut self) {
        let (pane, sent) = (self.pane, self.commands.len());
        self.run_intents();
        if self.pane == pane && self.commands.len() > sent {
            self.clear_failure();
        }
    }

    fn run_intents(&mut self) {
        let intents = std::mem::take(&mut *self.intents.borrow_mut());
        for intent in intents {
            if let Intent::Draft(id, text) = intent {
                self.drafts.insert(id, text);
                continue;
            }
            if let Intent::NewAccountName(_)
            | Intent::NewAccountLogin(_)
            | Intent::NewAccountAdmin(_) = intent
            {
                self.new_account_field(intent);
                continue;
            }
            if let Intent::OtherName(name) = intent {
                if let Some(other) = &mut self.other {
                    other.name = name;
                }
                continue;
            }
            if let Intent::PrinterAddress(address) = intent {
                self.printer_address = address;
                continue;
            }
            if let Intent::ReplacementText(field, text) = intent {
                self.type_replacement(field, text);
                continue;
            }
            self.stale = true;
            match intent {
                Intent::Draft(..) | Intent::OtherName(..) | Intent::PrinterAddress(..) => {}
                Intent::Back => {
                    if matches!(self.detail, Some(Detail::Account(_)))
                        && let Some(name) = self.drafts.remove(&SettingId::FullName)
                    {
                        self.rename_account(Value::Text(name));
                    }
                    if let Some(detail) = self.detail.take() {
                        self.forgetting = None;
                        self.clear_failure();
                        self.focus_next = Some(Anchor::Item(match detail {
                            Detail::App(group) => format!("app:{group}"),
                            Detail::Network(ssid) => String::from_utf8_lossy(&ssid).into_owned(),
                            Detail::Printer(name) => format!("printer:{name}"),
                            Detail::Account(login) => format!("account:{login}"),
                            Detail::Report(id) => format!("report:{id}"),
                        }));
                        self.picture_edit = None;
                        self.deleting_account = false;
                        self.password = None;
                    } else if let Some(parent) = self.pane.parent() {
                        self.focus_next = Some(Anchor::Item(self.pane.keyword().to_owned()));
                        self.select_pane(parent);
                    }
                }
                Intent::AskEmptyTrash(asking) => {
                    self.confirming_empty_trash = asking;
                    self.focus_next = Some(Anchor::Item(
                        if asking { "empty-trash" } else { "trash" }.to_owned(),
                    ));
                }
                Intent::AskClearClipboard(asking) => {
                    self.confirming_clear_clipboard = asking;
                    self.focus_next = Some(focus_after_asking(asking));
                }
                Intent::ClearClipboard => {
                    self.confirming_clear_clipboard = false;
                    self.focus_next = Some(focus_after_asking(false));
                    self.commands.push(Command::ClearClipboard);
                }
                Intent::EmptyTrash => {
                    self.confirming_empty_trash = false;
                    self.commands
                        .push(Command::Storage(StorageCommand::EmptyTrash));
                }
                Intent::Backup(intent) => self.backup_intent(intent),
                Intent::Reveal(path) => self.commands.push(Command::Reveal(path)),
                Intent::OpenApp(group) => {
                    self.focus_next = Some(Anchor::Back);
                    self.detail = Some(Detail::App(group));
                    *self.scroll.borrow_mut() = ScrollState::default();
                }
                Intent::Software(command) => self.commands.push(Command::Software(command)),
                Intent::EndSession(ending) => self.commands.push(Command::EndSession(ending)),
                Intent::Later(need) => {
                    if self.software.later(need) {
                        self.commands
                            .push(Command::Software(SoftwareCommand::Check));
                    }
                }
                Intent::Grant(permission, app, on) => self.grant(permission, app, on),
                Intent::KeyboardAccess(line, on) => self.change_keyboard_access(line, on),
                Intent::Spelling(command) => self.change_spelling(command),
                Intent::Battery(command) => self.commands.push(Command::Battery(command)),
                Intent::EditReplacement(original) => self.edit_replacement(original),
                Intent::ReplacementText(..) => {}
                Intent::Passwords(command) => self.password_command(command),
                Intent::PasswordQuery(query) => {
                    self.keychain.query = query;
                    self.focus_next = Some(Anchor::Picker);
                }
                Intent::ChangePassword(path) => {
                    self.focus_next = Some(match &path {
                        Some(_) => Anchor::Password,
                        None => Anchor::Setting(SettingId::SavedPasswords, None),
                    });
                    self.keychain.changing = path;
                }
                Intent::Network(command) => self.commands.push(Command::Network(command)),
                Intent::ImportVpn => self.commands.push(Command::ImportVpn),
                Intent::ExcludeFolder => self.commands.push(Command::ExcludeFolder),
                Intent::IncludeFolder(folder) => {
                    self.focus_next = Some(Anchor::Setting(SettingId::SearchExcluded, None));
                    let mut excluded = self.excluded_folders();
                    excluded.retain(|known| *known != folder);
                    self.change(SettingId::SearchExcluded, Value::Codes(excluded));
                }
                Intent::DefaultApp(role, id) => {
                    if let Some(known) = self
                        .default_apps
                        .iter_mut()
                        .flatten()
                        .find(|known| known.role == role)
                    {
                        known.current = Some(id.clone());
                    }
                    self.focus_next = Some(Anchor::Item(format!("default:{role}")));
                    self.commands
                        .push(Command::DefaultApp(DefaultAppCommand::Choose(role, id)));
                }
                Intent::Locale(command) => self.set_locale(command),
                Intent::OpenNetwork(ssid) => {
                    self.open_network(ssid);
                    *self.scroll.borrow_mut() = ScrollState::default();
                }
                Intent::AskOther(asking) => {
                    self.join = Join::default();
                    self.other = asking.then(Other::default);
                    self.focus_next = Some(if asking {
                        Anchor::Setting(SettingId::OtherNetwork, None)
                    } else {
                        Anchor::Item("other-network".to_owned())
                    });
                }
                Intent::OtherSecurity(index) => {
                    if let (Some(other), Some((security, _))) =
                        (&mut self.other, HIDDEN_SECURITY.get(index))
                    {
                        other.security = *security;
                    }
                }
                Intent::JoinOther(password) => self.join_other(password),
                Intent::WifiManual(manual) => self.set_wifi_manual(manual),
                Intent::OpenReport(id) => self.open_report(id),
                Intent::CopyReport(text) => self.commands.push(Command::CopyText(text)),
                Intent::DeleteReport(id) => {
                    self.detail = None;
                    self.commands
                        .push(Command::Diagnostics(DiagnosticsCommand::Delete(id)));
                }
                Intent::SendReport(id) => self
                    .commands
                    .push(Command::Diagnostics(DiagnosticsCommand::Send(id))),
                Intent::SetSendingReports(on) => self
                    .commands
                    .push(Command::Diagnostics(DiagnosticsCommand::SetSending(on))),
                Intent::DraftReportDescription(text) => self.diagnostics.description = text,
                Intent::CreateReport(()) => {
                    let description = std::mem::take(&mut self.diagnostics.description);
                    self.commands
                        .push(Command::Diagnostics(DiagnosticsCommand::Create(
                            description,
                        )));
                }
                Intent::SaveReport((id, text)) => self
                    .commands
                    .push(Command::Diagnostics(DiagnosticsCommand::Save(id, text))),
                Intent::OpenPrinter(name) => {
                    self.detail = Some(Detail::Printer(name));
                    self.forgetting = None;
                    self.clear_failure();
                    self.focus_next = Some(Anchor::Back);
                    *self.scroll.borrow_mut() = ScrollState::default();
                }
                Intent::DefaultPrinter(name) => {
                    self.commands
                        .push(Command::Printers(PrinterCommand::SetDefault(name)));
                }
                Intent::Sharing(command) => {
                    self.installing_sharing |= matches!(
                        command,
                        SharingCommand::InstallSamba | SharingCommand::InstallResolved
                    );
                    self.commands.push(Command::Sharing(command));
                }
                Intent::InstallPrinting => {
                    self.installing_printing = true;
                    self.commands
                        .push(Command::Printers(PrinterCommand::InstallPrinting));
                }
                Intent::AddPrinterAddress => {
                    let address = self.printer_address.trim().to_owned();
                    if !address.is_empty() {
                        self.adding_printer = true;
                        self.commands
                            .push(Command::Printers(PrinterCommand::AddAddress(address)));
                    }
                }
                Intent::CancelQueuedJob(job) => {
                    self.commands
                        .push(Command::Printers(PrinterCommand::CancelQueued(job)));
                }
                Intent::CancelJob(job) => {
                    self.commands
                        .push(Command::Printers(PrinterCommand::Cancel(job)));
                }
                Intent::AskPassword(asking) => self.ask_password(asking),
                Intent::PasswordEntry(step, text) => self.password_entry(step, text),
                Intent::OpenAccount(login) => self.open_account(login),
                Intent::ChooseAccountPicture => self.choose_account_picture(),
                Intent::KeepPicture(keep) => self.keep_picture(keep),
                Intent::PictureMoved => {}
                Intent::Administrator(login, on) => self.set_administrator(login, on),
                Intent::AskNewAccount(asking) => self.ask_new_account(asking),
                Intent::NewAccountPassword(step, text) => self.new_account_password(step, text),
                Intent::AskDeleteAccount(asking) => self.ask_delete_account(asking),
                Intent::DeleteAccount(uid, remove_files) => self.delete_account(uid, remove_files),
                Intent::NewAccountName(_)
                | Intent::NewAccountLogin(_)
                | Intent::NewAccountAdmin(_) => {}
                Intent::PausePrinter((name, paused)) => {
                    self.focus_next = Some(Anchor::Item("printer-pause".to_owned()));
                    self.commands
                        .push(Command::Printers(PrinterCommand::Pause(name, paused)));
                }
                Intent::ManualIpv4(manual) => {
                    self.manual_ipv4 = manual;
                    let dns = self
                        .network
                        .as_ref()
                        .and_then(|network| network.ethernet.as_ref())
                        .filter(|_| manual)
                        .map(|ethernet| ethernet.addressing.dns.join(", "))
                        .unwrap_or_default();
                    self.values.insert(SettingId::EthernetDns, Value::Text(dns));
                    if !manual {
                        self.commands
                            .push(Command::Network(NetworkCommand::ConfigureEthernet(None)));
                    }
                }
                Intent::SetAppRule(app) => {
                    if let Some(apps) = &mut self.notification_apps
                        && let Some(known) = apps.iter_mut().find(|known| known.group == app.group)
                    {
                        known.clone_from(&app);
                    }
                    self.commands
                        .push(Command::Notifications(NotificationCommand::Set(app)));
                }
                Intent::RemoveLoginItem(id) => {
                    self.focus_next = Some(Anchor::Setting(SettingId::LoginItems, None));
                    self.commands.push(Command::Login(LoginCommand::Remove(id)));
                }
                Intent::ChangeAppLanguage(id) => {
                    self.language_app = Some(id);
                    self.picking = Some((Picker::AppLanguage, String::new()));
                    self.focus_next = Some(Anchor::Picker);
                }
                Intent::RemoveAppLanguage(id) => {
                    self.focus_next = Some(Anchor::Setting(SettingId::AppLanguages, None));
                    self.commands
                        .push(Command::Login(LoginCommand::SetLanguage(id, None)));
                }
                Intent::Pick(picker) => {
                    self.focus_next = Some(match (picker, &self.picking) {
                        (Some(_), _) => Anchor::Picker,
                        (None, Some((open, _))) => Anchor::Setting(open.setting(), None),
                        (None, None) => continue,
                    });
                    self.picking = picker.map(|picker| (picker, String::new()));
                }
                Intent::PickQuery(query) => {
                    if let Some((_, picking)) = &mut self.picking {
                        *picking = query;
                        self.focus_next = Some(Anchor::Picker);
                    }
                }
                Intent::Picked(key) => {
                    let Some((picker, _)) = self.picking.take() else {
                        continue;
                    };
                    self.focus_next = Some(Anchor::Setting(picker.setting(), None));
                    self.errors.remove(&picker.setting());
                    self.clear_failure();
                    self.picked(picker, key);
                }
                Intent::SelectPane(pane) => {
                    if pane.parent() == Some(self.pane) {
                        self.focus_next = Some(Anchor::Back);
                    }
                    self.select_pane(pane);
                }
                Intent::Open(pane, setting) => {
                    self.focus_next = Some(Anchor::Setting(setting, None));
                    self.select_pane(pane);
                }
                Intent::StepPane(step) => self.step_pane(step),
                Intent::Query(query) => self.set_query(query),
                Intent::Set(id, value) => self.change(id, value),
                Intent::Display(identity, id, value) => self.change_display(identity, id, value),
                Intent::Arrange(identity, position) => self.arrange(&identity, position),
                Intent::KeepDisplays(keep) => self.keep_displays(keep),
                Intent::Default(direction, name) => {
                    self.commands
                        .push(Command::Audio(AudioCommand::Default(direction, name)));
                }
                Intent::AppVolume(name, volume) => {
                    self.commands
                        .push(Command::Audio(AudioCommand::App(name, volume)));
                }
                Intent::VolumeFeedback => {
                    self.commands.push(Command::Audio(AudioCommand::Feedback))
                }
                Intent::Join(ssid) => self.start_join(ssid, None),
                Intent::Password(password) => {
                    if let Some(ssid) = self.join.asking.clone() {
                        self.start_join(ssid, Some(password));
                    }
                }
                Intent::CancelJoin => self.join = Join::default(),
                Intent::Connect(address) => self.device_command(address, BluetoothCommand::Connect),
                Intent::Disconnect(address) => {
                    self.device_command(address, BluetoothCommand::Disconnect)
                }
                Intent::Pair(address) => self.device_command(address, BluetoothCommand::Pair),
                Intent::AskForget(target) => self.ask_forget(target),
                Intent::Forget => self.forget(),
                Intent::Answer(answer) => {
                    self.pairing = None;
                    self.commands.push(Command::Answer(answer));
                }
                Intent::ChoosePicture => {
                    self.commands.push(Command::Picture(PictureCommand::Choose));
                }
                Intent::RemoveSource(index) => {
                    self.focus_next = Some(Anchor::Setting(SettingId::InputSources, None));
                    self.edit_sources(|sources| {
                        if index < sources.len() {
                            sources.remove(index);
                        }
                    });
                }
                Intent::MoveSource(index, up) => {
                    let target = if up {
                        index.checked_sub(1)
                    } else {
                        Some(index + 1)
                    };
                    self.edit_sources(|sources| {
                        if let Some(target) = target.filter(|target| *target < sources.len()) {
                            sources.swap(index, target);
                        }
                    });
                    if let Some(code) =
                        target.and_then(|target| self.input_sources().get(target).cloned())
                    {
                        self.focus_next = Some(Anchor::Item(code));
                    }
                }
            }
        }
    }

    fn set_locale(&mut self, command: LocaleCommand) {
        self.errors.remove(&SettingId::Language);
        self.errors.remove(&SettingId::Region);
        if let Some(locale) = &mut self.locale {
            match &command {
                LocaleCommand::Language(language) => {
                    self.commands.push(Command::Spelling(
                        crate::spelling::SpellingCommand::GetDictionary(language.clone()),
                    ));
                    language.clone_into(&mut locale.language);
                    locale.pending = Some(SettingId::Language);
                }
                LocaleCommand::Region(region) => {
                    locale.region.clone_from(region);
                    locale.pending = Some(SettingId::Region);
                }
                LocaleCommand::LoginKeyboard(codes) => locale.login_sources.clone_from(codes),
                LocaleCommand::Read => {}
            }
        }
        self.commands.push(Command::Locale(command));
    }

    fn picked(&mut self, picker: Picker, key: String) {
        match picker {
            Picker::Language => self.set_locale(LocaleCommand::Language(key)),
            Picker::Region => self.set_locale(LocaleCommand::Region(
                (key != SAME_AS_LANGUAGE).then_some(key),
            )),
            Picker::Zone => {
                if let Some(time) = &mut self.time {
                    time.zone = key.clone();
                }
                self.commands.push(Command::Time(TimeCommand::Zone(key)));
            }
            Picker::LoginApp => self.commands.push(Command::Login(LoginCommand::Add(key))),
            Picker::LanguageApp => {
                self.language_app = Some(key);
                self.picking = Some((Picker::AppLanguage, String::new()));
                self.focus_next = Some(Anchor::Picker);
            }
            Picker::AppLanguage => {
                if let Some(app) = self.language_app.take() {
                    self.commands
                        .push(Command::Login(LoginCommand::SetLanguage(app, Some(key))));
                }
            }
            Picker::InputSource => {
                self.focus_next = Some(Anchor::Item(key.clone()));
                self.edit_sources(|sources| {
                    if !sources.contains(&key) {
                        sources.push(key);
                    }
                });
            }
        }
    }

    fn device_command(
        &mut self,
        address: String,
        command: fn(bande_bluetooth::Device) -> BluetoothCommand,
    ) {
        let device = self.bluetooth.present().and_then(|bluetooth| {
            bluetooth
                .paired
                .iter()
                .chain(&bluetooth.nearby)
                .find(|device| device.address == address)
                .cloned()
        });
        if let Some(device) = device {
            self.busy.push(address);
            self.commands.push(Command::Bluetooth(command(device)));
        }
    }

    fn start_join(&mut self, ssid: Vec<u8>, password: Option<String>) {
        let Some(network) = self
            .wifi
            .present()
            .and_then(|wifi| wifi.networks.iter().find(|network| network.ssid == ssid))
            .cloned()
        else {
            return;
        };
        if network.current {
            return;
        }
        if password.is_none() && network.security.needs_password() && !network.known {
            self.join = Join {
                asking: Some(ssid),
                ..Join::default()
            };
            self.focus_next = Some(Anchor::Password);
            return;
        }
        self.join = Join {
            joining: Some(ssid.clone()),
            ..Join::default()
        };
        self.focus_next = Some(Anchor::Item(String::from_utf8_lossy(&ssid).into_owned()));
        self.commands
            .push(Command::Network(NetworkCommand::Join(network, password)));
    }

    fn open_report(&mut self, id: String) {
        self.commands
            .push(Command::Diagnostics(DiagnosticsCommand::Read(id.clone())));
        self.detail = Some(Detail::Report(id));
        self.clear_failure();
        self.focus_next = Some(Anchor::Back);
        *self.scroll.borrow_mut() = ScrollState::default();
    }

    pub fn show_report_of(&mut self, wanted: &str) {
        self.report_wanted = Some(wanted.to_owned());
        self.commands
            .push(Command::Diagnostics(DiagnosticsCommand::Refresh));
    }

    pub fn pane_keyword(&self) -> &'static str {
        self.pane.keyword()
    }

    pub fn select_pane(&mut self, pane: PaneId) {
        if pane == PaneId::Diagnostics {
            self.commands
                .push(Command::Diagnostics(DiagnosticsCommand::Refresh));
        }
        if pane == self.pane {
            return;
        }
        self.follow_pane(false);
        if self.pane == PaneId::Passwords {
            self.password_command(PasswordCommand::Lock);
        }
        match pane {
            PaneId::Displays => self
                .commands
                .push(Command::Brightness(LevelRequest::Refresh)),
            PaneId::Wifi => self.commands.push(Command::Network(NetworkCommand::Scan)),
            PaneId::Storage => self
                .commands
                .push(Command::Storage(StorageCommand::Measure)),
            PaneId::Backup => self.backup_opened(),
            PaneId::Apps => {
                self.commands.push(Command::Login(LoginCommand::Refresh));
                self.commands
                    .push(Command::DefaultApp(DefaultAppCommand::Refresh));
            }
            PaneId::LanguageTime => self.commands.push(Command::Login(LoginCommand::Refresh)),
            PaneId::Notifications => self
                .commands
                .push(Command::Notifications(NotificationCommand::Refresh)),
            PaneId::Users => self.commands.push(Command::Users(UsersCommand::Refresh)),
            PaneId::SoftwareUpdate if self.software == SoftwareState::Unknown => {
                self.commands.push(Command::Software(SoftwareCommand::Open))
            }
            _ => {}
        }
        let left = std::mem::replace(&mut self.pane, pane);
        self.follow_pane(true);
        self.confirming_empty_trash = false;
        self.confirming_clear_clipboard = false;
        self.picking = None;
        self.detail = None;
        self.other = None;
        self.password = None;
        self.password_message = None;
        self.picture_edit = None;
        self.new_account = None;
        self.deleting_account = false;
        self.errors.clear();
        self.failures.remove(&left);
        *self.scroll.borrow_mut() = ScrollState::default();
        self.invalidate();
    }

    pub fn set_on_screen(&mut self, on_screen: bool) -> bool {
        if self.on_screen == on_screen {
            return false;
        }
        if on_screen {
            self.on_screen = true;
            self.follow_pane(true);
        } else {
            self.follow_pane(false);
            self.on_screen = false;
        }
        true
    }

    fn follow_pane(&mut self, on: bool) {
        if !self.on_screen {
            return;
        }
        let command = match self.pane {
            PaneId::Bluetooth => Command::Bluetooth(BluetoothCommand::Discover(on)),
            PaneId::Sound => Command::Audio(AudioCommand::Listen(on)),
            PaneId::Wifi | PaneId::Network => Command::Network(NetworkCommand::Watch(on)),
            PaneId::Printers => Command::Printers(PrinterCommand::Watch(on)),
            PaneId::Sharing if on => Command::Sharing(SharingCommand::Read),
            PaneId::Keyboard if on => {
                self.commands
                    .push(Command::Spelling(crate::spelling::SpellingCommand::Read));
                Command::Locale(LocaleCommand::Read)
            }
            PaneId::LanguageTime if on => Command::Locale(LocaleCommand::Read),
            _ => return,
        };
        self.commands.push(command);
    }

    fn step_pane(&mut self, step: isize) {
        let visible = self.visible_panes();
        let Some(at) = visible
            .iter()
            .position(|pane| *pane == self.pane || Some(*pane) == self.pane.parent())
        else {
            return;
        };
        if let Some(&pane) = at
            .checked_add_signed(step)
            .and_then(|next| visible.get(next))
        {
            if !self.search_focused() {
                self.focus_next = Some(Anchor::Pane(pane));
            }
            self.select_pane(pane);
        }
    }

    fn search_hits(&self, query: &str) -> Vec<PaneHit> {
        search(&self.panes, query)
            .into_iter()
            .filter(|hit| self.available(hit.pane))
            .collect()
    }

    fn set_query(&mut self, query: String) {
        if query == self.query {
            return;
        }
        self.hits = self.search_hits(&query);
        self.query = query;
        let visible = self.visible_panes();
        if let Some(&first) = visible.first()
            && (!self.shown_in(&visible) || !self.hits.is_empty())
        {
            self.select_pane(first);
        }
    }

    pub(crate) fn text_size_label(&self) -> Option<String> {
        match self.values.get(&SettingId::TextSize)? {
            Value::Number(percent) => Some(tr!("{percent}%", percent = (*percent))),
            _ => None,
        }
    }

    fn command(id: SettingId, value: Value) -> Command {
        match (id, value) {
            (SettingId::OutputVolume, Value::Fraction(volume)) => {
                Command::Audio(AudioCommand::Volume(Direction::Output, volume))
            }
            (SettingId::InputVolume, Value::Fraction(volume)) => {
                Command::Audio(AudioCommand::Volume(Direction::Input, volume))
            }
            (SettingId::OutputBalance, Value::Number(position)) => {
                Command::Audio(AudioCommand::Balance(position as f32 / 50.0 - 1.0))
            }
            (SettingId::OutputMuted, Value::Flag(muted)) => {
                Command::Audio(AudioCommand::Mute(Direction::Output, muted))
            }
            (SettingId::InputMuted, Value::Flag(muted)) => {
                Command::Audio(AudioCommand::Mute(Direction::Input, muted))
            }
            (SettingId::WifiPower, Value::Flag(on)) => Command::Network(NetworkCommand::Power(on)),
            (SettingId::BluetoothPower, Value::Flag(on)) => {
                Command::Bluetooth(BluetoothCommand::Power(on))
            }
            (SettingId::PowerMode, Value::Keyword(profile)) => {
                Command::Battery(BatteryCommand::Profile(profile))
            }
            (SettingId::AutomaticTime, Value::Flag(on)) => {
                Command::Time(TimeCommand::Automatic(on))
            }
            (SettingId::ComputerName, Value::Text(name)) => {
                Command::About(AboutCommand::Rename(name))
            }
            (id, value) => Command::Set(id, value),
        }
    }

    fn commit_drafts(&mut self) {
        for (id, text) in std::mem::take(&mut self.drafts) {
            let text = text.trim().to_owned();
            let clearable = matches!(
                id,
                SettingId::EthernetRouter
                    | SettingId::EthernetDns
                    | SettingId::WifiRouter
                    | SettingId::WifiDns
                    | SettingId::LockMessage
            );
            if clearable || !text.is_empty() {
                self.change(id, Value::Text(text));
            }
        }
        self.invalidate();
    }

    fn change(&mut self, id: SettingId, value: Value) {
        if id == SettingId::FullName {
            self.rename_account(value);
            return;
        }
        if matches!(
            id,
            SettingId::WifiAddress | SettingId::WifiRouter | SettingId::WifiDns
        ) {
            self.values.insert(id, value);
            self.change_wifi_addressing(id);
            return;
        }
        if matches!(
            id,
            SettingId::EthernetAddress | SettingId::EthernetRouter | SettingId::EthernetDns
        ) {
            self.values.insert(id, value);
            if !self.manual_ipv4 {
                let path = self
                    .network
                    .as_ref()
                    .and_then(|network| network.ethernet.as_ref())
                    .and_then(|ethernet| ethernet.connection.clone());
                let servers = match self.values.get(&SettingId::EthernetDns) {
                    Some(Value::Text(text)) => bande_network::parse_dns(text),
                    _ => Some(Vec::new()),
                };
                match (path, servers) {
                    (Some(path), Some(servers)) if id == SettingId::EthernetDns => {
                        self.clear_failure();
                        self.commands
                            .push(Command::Network(NetworkCommand::ChangeConnection(
                                PaneId::Network,
                                path,
                                bande_network::ConnectionChange::Dns(servers),
                            )));
                    }
                    (_, None) => self.fail_here(tr!(
                        "Enter DNS servers as IPv4 addresses, such as 1.1.1.1, 9.9.9.9."
                    )),
                    _ => self.clear_failure(),
                }
                return;
            }
            let text = |id| match self.values.get(&id) {
                Some(Value::Text(text)) => text.clone(),
                _ => String::new(),
            };
            let manual = bande_network::parse_manual(
                &text(SettingId::EthernetAddress),
                &text(SettingId::EthernetRouter),
                &text(SettingId::EthernetDns),
            );
            match manual {
                Some(manual) => {
                    self.clear_failure();
                    self.commands
                        .push(Command::Network(NetworkCommand::ConfigureEthernet(Some(
                            manual,
                        ))));
                }
                None => self.fail_here(tr!("Type an address such as 192.168.1.20/24, and the router and DNS servers as addresses such as 192.168.1.1.")),
            }
            return;
        }
        if let (
            SettingId::NightFrom | SettingId::NightTo | SettingId::FocusFrom | SettingId::FocusTo,
            Value::Text(text),
        ) = (id, &value)
        {
            match clock_text(text) {
                Some(time) => {
                    self.errors.remove(&id);
                    if self.values.get(&id) != Some(&Value::Text(time.clone())) {
                        self.values.insert(id, Value::Text(time.clone()));
                        self.commands.push(Command::Set(id, Value::Text(time)));
                    }
                }
                None => {
                    self.errors.insert(id, tr!("Enter a time such as 22:00."));
                }
            }
            return;
        }
        if let (SettingId::DateAndTime, Value::Text(text)) = (id, &value) {
            let zone = self
                .time
                .as_ref()
                .map(|time| time.zone.clone())
                .unwrap_or_default();
            match parse_manual(text, &zone) {
                Some(at) => {
                    self.errors.remove(&id);
                    self.commands.push(Command::Time(TimeCommand::Set(at)));
                }
                None => {
                    self.errors.insert(id, tr!("Type the date and time as year-month-day hours:minutes, such as 2026-10-30 14:05."));
                }
            }
            return;
        }
        if self.values.get(&id) == Some(&value) {
            return;
        }
        self.values.insert(id, value.clone());
        self.commands.push(Self::command(id, value));
        let unmuted = match id {
            SettingId::OutputVolume => Some(SettingId::OutputMuted),
            SettingId::InputVolume => Some(SettingId::InputMuted),
            _ => None,
        };
        if let Some(muted) = unmuted {
            self.values.insert(muted, Value::Flag(false));
        }
        self.errors.remove(&id);
        self.clear_failure();
    }

    fn can_go_back(&self) -> bool {
        self.detail.is_some() || self.pane.parent().is_some()
    }

    pub(crate) fn allowed(&self, id: SettingId) -> bool {
        Privilege::of(id).is_none_or(|privilege| !self.refused.contains(&privilege))
    }

    pub fn deadline(&self) -> Option<Instant> {
        let keep = self.keep.as_ref()?;
        let left = keep.until.saturating_duration_since(Instant::now());
        let tick = Duration::from_nanos(left.subsec_nanos().into());
        Some(Instant::now() + if tick.is_zero() { SECOND } else { tick })
    }

    pub fn tick(&mut self, now: Instant) {
        let Some(keep) = &self.keep else {
            return;
        };
        if now >= keep.until {
            self.keep_displays(false);
        } else {
            self.invalidate();
        }
    }

    pub(crate) fn seconds_left(&self) -> u64 {
        self.keep.as_ref().map_or(0, |keep| {
            keep.until
                .saturating_duration_since(Instant::now())
                .as_secs_f64()
                .ceil() as u64
        })
    }

    fn keep_displays(&mut self, keep: bool) {
        let Some(pending) = self.keep.take() else {
            return;
        };
        if !keep {
            self.display_settings
                .insert(pending.identity.clone(), pending.previous.clone());
            self.commands
                .push(Command::Display(pending.identity, pending.previous));
        }
        self.invalidate();
    }

    pub(crate) fn placements(&self) -> Vec<Placement> {
        let identities: Vec<String> = self
            .displays
            .iter()
            .map(|display| display.identity.clone())
            .collect();
        let known = self
            .arrangements
            .iter()
            .find(|placements| bande_config::same_displays(placements, &identities));
        self.displays
            .iter()
            .map(|display| {
                let mirror = known.and_then(|placements| {
                    placements
                        .iter()
                        .find(|placement| placement.display == display.identity)
                        .and_then(|placement| placement.mirror.clone())
                });
                Placement {
                    display: display.identity.clone(),
                    x: display.position.0,
                    y: display.position.1,
                    main: display.position == (0, 0) && mirror.is_none(),
                    mirror,
                }
            })
            .collect()
    }

    fn arrange(&mut self, identity: &str, (x, y): (i32, i32)) {
        let mut placements = self.placements();
        for placement in &mut placements {
            if placement.display == identity {
                placement.x = x;
                placement.y = y;
            }
        }
        for display in &mut self.displays {
            if display.identity == identity {
                display.position = (x, y);
            }
        }
        self.commands.push(Command::Arrangement(placements));
    }

    fn use_display(&mut self, identity: &str, choice: &str) {
        let mut placements = self.placements();
        let mirrored = choice.strip_prefix(MIRROR_PREFIX);
        for placement in &mut placements {
            if choice == USE_MAIN {
                placement.main = placement.display == identity;
            }
            if placement.display == identity {
                placement.mirror = mirrored.map(str::to_owned);
                if mirrored.is_some() {
                    placement.main = false;
                }
            }
        }
        if mirrored.is_some()
            && !placements.iter().any(|placement| placement.main)
            && let Some(source) = placements
                .iter_mut()
                .find(|placement| Some(placement.display.as_str()) == mirrored)
        {
            source.main = true;
        }

        self.commands.push(Command::Arrangement(placements));
    }

    fn set_display_level(&mut self, identity: &str, level: f32) {
        let Some(connector) = self
            .displays
            .iter()
            .find(|display| display.identity == identity)
            .map(|display| display.connector.clone())
        else {
            return;
        };
        if let Some(shown) = self
            .display_levels
            .iter_mut()
            .find(|shown| shown.connector == connector)
        {
            shown.level = level;
        }
        self.commands
            .push(Command::Brightness(LevelRequest::Set(connector, level)));
    }

    fn change_display(&mut self, identity: String, id: SettingId, value: Value) {
        if let (SettingId::ShowAllResolutions, Value::Flag(shown)) = (id, &value) {
            self.show_all_resolutions = *shown;
            self.invalidate();
            return;
        }
        let mut display = self
            .display_settings
            .get(&identity)
            .cloned()
            .unwrap_or_default();
        let previous = display.clone();
        let mut confirm = false;
        match (id, value) {
            (SettingId::Brightness, Value::Fraction(level)) => {
                self.set_display_level(&identity, level);
                return;
            }
            (SettingId::DisplayUse, Value::Keyword(choice)) => {
                self.use_display(&identity, &choice);
                return;
            }
            (SettingId::DisplayResolution, Value::Keyword(size)) => {
                let Some((width, height)) = size
                    .split_once('x')
                    .and_then(|(width, height)| Some((width.parse().ok()?, height.parse().ok()?)))
                else {
                    return;
                };
                display.resolution = Some((width, height));
                display.refresh_rate = None;
                confirm = true;
            }
            (SettingId::DisplayRotation, Value::Number(degrees)) => {
                display.rotation = Rotation::from_degrees(degrees)
                    .filter(|rotation| *rotation != Rotation::Upright);
                confirm = true;
            }
            (SettingId::DisplayScale, Value::Number(percent)) => {
                display.scale = Some(f64::from(percent) / 100.0);
            }
            (SettingId::DisplayScale, Value::Keyword(keyword)) if keyword == AUTOMATIC => {
                display.scale = None;
            }
            (SettingId::RefreshRate, Value::Number(hertz)) => {
                display.refresh_rate = Some(f64::from(hertz));
            }
            (SettingId::ColourProfile, Value::Keyword(profile)) => {
                if profile == ADD_COLOUR_PROFILE {
                    self.commands.push(Command::AddColourProfile(identity));
                    return;
                }
                display.colour_profile = (profile != bande_colour::DISPLAY).then_some(profile);
            }
            _ => return,
        }
        if confirm && display != previous {
            let previous = match &self.keep {
                Some(keep) if keep.identity == identity => keep.previous.clone(),
                _ => previous,
            };
            self.keep_displays(true);
            self.keep = Some(Keep {
                identity: identity.clone(),
                previous,
                until: Instant::now() + KEEP_DISPLAY_SETTINGS,
            });
        }
        self.display_settings
            .insert(identity.clone(), display.clone());
        self.commands.push(Command::Display(identity, display));
    }

    fn rebuild(&mut self) {
        self.reveal_pending = self.focus_next.is_some();
        let focused = self.focus_next.take().or_else(|| {
            let focus = self.ui.focus()?;
            self.anchors
                .iter()
                .find(|(_, id)| *id == focus)
                .map(|(anchor, _)| anchor.clone())
        });
        self.anchors.clear();
        let sidebar = self.sidebar();
        let content = self.content();
        if let Some(root) = self.ui.widget(self.root) {
            let children = root.children_mut();
            children[Root::PANES] = sidebar;
            children[Root::CONTENT] = content;
        }
        let typed = match &focused {
            Some(Anchor::Picker) => self.picking.as_ref().map(|(_, query)| query.clone()),
            Some(Anchor::Setting(setting, _)) => self.drafts.get(setting).cloned(),
            _ => None,
        };
        let target = focused.and_then(|anchor| {
            self.anchors
                .iter()
                .find(|(candidate, _)| match (candidate, &anchor) {
                    (Anchor::Setting(a, _), Anchor::Setting(b, None)) => a == b,
                    _ => *candidate == anchor,
                })
                .map(|(_, id)| *id)
        });
        if let Some(target) = target {
            self.ui.perform(&request(Action::Focus, target, None));
            if let Some(text) = typed {
                self.ui.perform(&request(
                    Action::SetValue,
                    target,
                    Some(ActionData::Value(text.into())),
                ));
                self.intents
                    .borrow_mut()
                    .retain(|intent| !matches!(intent, Intent::PickQuery(_) | Intent::Draft(..)));
            }
        }
        self.stale = false;
        self.rebuilt = true;
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        if std::mem::take(&mut self.reveal_pending) {
            self.ui.layout(Size::new(width as f32, height as f32));
            self.ui.reveal_focus();
        }
        self.ui.scene(width, height)
    }

    pub fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }

    pub fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    pub fn accessibility(&self) -> TreeUpdate {
        self.ui.accessibility()
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        let handled = self.ui.perform(request);
        self.settle() | handled
    }
}
