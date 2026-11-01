use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;

use accesskit::{Action, ActionData, ActionRequest, TreeId, TreeUpdate};
use bande_audio::{Direction, Snapshot, meter_level};
use bande_config::{DisplaySettings, MAX_INPUT_SOURCES, Placement, Rotation};
use bande_datetime::daylight::Place;
use bande_toolkit::ui::{
    Context, Event, Key, Modifiers, PointerButton, ScrollState, SearchField, Size, Ui, Widget,
    WidgetId,
};
use bande_toolkit::{Appearance, Image, Palette, Rect, SHEET_INSET, Scene, TextSystem};
use jiff::Zoned;
use std::time::{Duration, Instant};

use crate::L10N;
use crate::about::{About, AboutCommand};
use crate::battery::{BatteryCommand, BatteryState};
use crate::defaults::{DefaultApp, DefaultAppCommand};
use crate::follow::{Command, Update};
use crate::forget::Forget;
use crate::idle::Idle;
use crate::locale::{LocaleCommand, LocaleState};
use crate::login::{LoginCommand, LoginItems, login_matches};
use crate::night::clock_text;
use crate::notifications::{AppNotifications, NotificationCommand};
use crate::permissions::Grants;
use crate::pictures::{Pair, PictureCommand};
use crate::pointing::PointingDevices;
use crate::privacy::focus_after_asking;
use crate::privilege::Privilege;
use crate::services::{AudioCommand, BluetoothCommand, NetworkCommand};
use crate::software::{SoftwareCommand, SoftwareState};
use crate::state::{Answer, BluetoothState, Display, Joined, NetworkState, Question, WifiState};
use crate::storage::{Storage, StorageCommand};
use crate::time::{TimeCommand, TimeState, parse_manual, zone_matches};
use crate::widgets::{Column, Crossfade, Root};
use bande_keyboard::Layouts;
use bande_settings_panes::{AUTOMATIC, Pane, PaneId, Permission, SettingId, Value, panes};
use bande_settings_panes::{PaneHit, search};

const PICK_RESULTS: usize = 8;
const SECOND: Duration = Duration::from_secs(1);
pub(crate) const USE_MAIN: &str = "main";
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
    Reveal(PathBuf),
    Pick(Option<Picker>),
    PickQuery(String),
    Picked(String),
    RemoveLoginItem(String),
    OpenApp(String),
    SetAppRule(AppNotifications),
    Software(SoftwareCommand),
    Network(NetworkCommand),
    ManualIpv4(bool),
    ImportVpn,
    Locale(LocaleCommand),
    DefaultApp(&'static str, String),
    Grant(Permission, String, bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Picker {
    Zone,
    LoginApp,
    InputSource,
}

impl Picker {
    fn setting(self) -> SettingId {
        match self {
            Self::Zone => SettingId::TimeZone,
            Self::LoginApp => SettingId::LoginItems,
            Self::InputSource => SettingId::InputSources,
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

pub struct SettingsView {
    pub(crate) panes: Vec<Pane>,
    pub(crate) values: HashMap<SettingId, Value>,
    pub(crate) display_settings: BTreeMap<String, DisplaySettings>,
    pub(crate) arrangements: Vec<Vec<Placement>>,
    pub(crate) displays: Vec<Display>,
    pub(crate) keep: Option<Keep>,
    pub(crate) audio: Option<Snapshot>,
    pub(crate) input_level: Rc<Cell<f32>>,
    pub(crate) wifi: Option<WifiState>,
    pub(crate) join: Join,
    pub(crate) bluetooth: Option<BluetoothState>,
    pub(crate) busy: Vec<String>,
    pub(crate) pairing: Option<Question>,
    pub(crate) pictures: Vec<(String, Pair)>,
    pub(crate) thumbnails: HashMap<Pair, Option<Image>>,
    pub(crate) query: String,
    pub(crate) hits: Vec<PaneHit>,
    pub(crate) pane: PaneId,
    pub(crate) layouts: Option<Layouts>,
    pub(crate) pointing: PointingDevices,
    pub(crate) about: Option<About>,
    pub(crate) storage: Option<Storage>,
    pub(crate) drafts: HashMap<SettingId, String>,
    pub(crate) confirming_empty_trash: bool,
    pub(crate) confirming_clear_clipboard: bool,
    pub(crate) forgetting: Option<Forget>,
    pub(crate) time: Option<TimeState>,
    pub(crate) picking: Option<(Picker, String)>,
    pub(crate) login: Option<LoginItems>,
    pub(crate) battery: Option<BatteryState>,
    pub(crate) notification_apps: Option<Vec<AppNotifications>>,
    pub(crate) app_page: Option<String>,
    pub(crate) software: SoftwareState,
    pub(crate) network: Option<NetworkState>,
    pub(crate) manual_ipv4: bool,
    pub(crate) locale: Option<LocaleState>,
    ui: Ui,
    root: WidgetId,
    search: WidgetId,
    pub(crate) anchors: Vec<(Anchor, WidgetId)>,
    pub(crate) intents: Intents,
    pub(crate) commands: Vec<Command>,
    pub(crate) message: Option<String>,
    pub(crate) errors: HashMap<SettingId, String>,
    pub(crate) refused: Vec<Privilege>,
    pub(crate) default_apps: Option<Vec<DefaultApp>>,
    pub(crate) permissions: Grants,
    pressed: bool,
    stale: bool,
    pub(crate) focus_next: Option<Anchor>,
    reveal_pending: bool,
    pub(crate) scroll: Rc<RefCell<ScrollState>>,
    pub(crate) page_view: Option<WidgetId>,
    pub(crate) crossfade: Rc<RefCell<Crossfade>>,
    pub(crate) shown: Option<(PaneId, bool)>,
    pub(crate) sidebar_scroll: Rc<RefCell<ScrollState>>,
    pub(crate) icon_dirs: Vec<PathBuf>,
    pub(crate) symbols: HashMap<Symbol, Option<Image>>,
    pub(crate) place: OnceCell<Option<Place>>,
    pub(crate) clock: Box<dyn Fn() -> Zoned>,
}

pub(crate) type Symbol = (&'static str, u32, [u8; 4]);

impl SettingsView {
    pub fn new(palette: Palette, text: TextSystem) -> Self {
        let intents: Intents = Rc::default();
        let queries = Rc::clone(&intents);
        let field = SearchField::new(&L10N.tr("search"), "", move |query: &str| {
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
            arrangements: Vec::new(),
            displays: Vec::new(),
            keep: None,
            audio: None,
            input_level: Rc::default(),
            wifi: None,
            join: Join::default(),
            bluetooth: None,
            busy: Vec::new(),
            pairing: None,
            pictures: Vec::new(),
            thumbnails: HashMap::new(),
            query: String::new(),
            hits: Vec::new(),
            pane: PaneId::Appearance,
            layouts: None,
            pointing: PointingDevices::default(),
            about: None,
            storage: None,
            drafts: HashMap::new(),
            confirming_empty_trash: false,
            confirming_clear_clipboard: false,
            forgetting: None,
            time: None,
            picking: None,
            login: None,
            battery: None,
            notification_apps: None,
            app_page: None,
            software: SoftwareState::Unknown,
            network: None,
            manual_ipv4: false,
            locale: None,
            ui: Ui::new(Box::new(root), cx),
            root: root_id,
            search,
            anchors: Vec::new(),
            intents,
            commands: Vec::new(),
            message: None,
            errors: HashMap::new(),
            refused: Vec::new(),
            default_apps: None,
            permissions: Vec::new(),
            pressed: false,
            stale: true,
            focus_next: None,
            reveal_pending: false,
            scroll: Rc::default(),
            page_view: None,
            crossfade: Rc::default(),
            shown: None,
            sidebar_scroll: Rc::default(),
            icon_dirs: bande_xdg::data_dirs(),
            symbols: HashMap::new(),
            place: OnceCell::new(),
            clock: Box::new(bande_datetime::now),
        };
        view.rebuild();
        view
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

    pub fn set_clock(&mut self, clock: impl Fn() -> Zoned + 'static, place: Option<Place>) {
        self.clock = Box::new(clock);
        self.place = OnceCell::from(place);
        self.invalidate();
    }

    pub fn set_icon_dirs(&mut self, dirs: Vec<PathBuf>) {
        self.icon_dirs = dirs;
        self.symbols.clear();
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

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn set_layouts(&mut self, layouts: Layouts) {
        self.layouts = Some(layouts);
        self.invalidate();
    }

    pub fn set_pointing_devices(&mut self, devices: PointingDevices) {
        if self.pointing != devices {
            self.pointing = devices;
            self.availability_changed();
        }
    }

    pub fn hide_screen_reader(&mut self) {
        for section in self.panes.iter_mut().flat_map(|pane| &mut pane.sections) {
            section
                .settings
                .retain(|setting| setting.id != SettingId::ScreenReader);
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

    pub(crate) fn layouts(&mut self) -> &Layouts {
        self.layouts.get_or_insert_with(Layouts::load)
    }

    fn edit_sources(&mut self, edit: impl FnOnce(&mut Vec<String>)) {
        let mut sources = self.input_sources();
        edit(&mut sources);
        if !sources.is_empty() && sources.len() <= MAX_INPUT_SOURCES {
            self.change(SettingId::InputSources, Value::Codes(sources));
        }
    }

    pub fn set_displays(&mut self, displays: Vec<Display>) {
        if self.displays != displays {
            self.displays = displays;
            self.availability_changed();
        }
    }

    pub fn apply(&mut self, update: Update) {
        match update {
            Update::InputLevel(peak) => {
                self.input_level
                    .set(meter_level(peak, self.input_level.get()));
                return;
            }
            Update::Values(values) => {
                for (id, value) in values {
                    self.values.insert(id, value);
                }
            }
            Update::Displays(displays) => self.display_settings = displays,
            Update::Arrangements(arrangements) => self.arrangements = arrangements,
            Update::Audio(snapshot) => self.apply_audio(snapshot),
            Update::Wifi(wifi) => {
                if let Some(wifi) = &wifi {
                    self.values
                        .insert(SettingId::WifiPower, Value::Flag(wifi.enabled));
                }
                self.wifi = wifi;
            }
            Update::Joined(ssid, outcome) => self.joined(ssid, outcome),
            Update::Bluetooth(bluetooth) => {
                if let Some(bluetooth) = &bluetooth {
                    self.values
                        .insert(SettingId::BluetoothPower, Value::Flag(bluetooth.powered));
                    let connection = |state: Option<&BluetoothState>, address: &str| {
                        state?
                            .paired
                            .iter()
                            .find(|device| device.address == address)
                            .map(|device| device.connected)
                    };
                    let before = self.bluetooth.as_ref();
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
            Update::Locale(locale) => self.locale = locale,
            Update::Pointing(devices) => self.set_pointing_devices(devices),
            Update::Network(network) => {
                if let Some(ethernet) = network.ethernet.as_ref() {
                    self.manual_ipv4 |= ethernet.manual;
                    for (id, value) in [
                        (SettingId::EthernetAddress, ethernet.address.clone()),
                        (SettingId::EthernetRouter, ethernet.router.clone()),
                        (SettingId::EthernetDns, Some(ethernet.dns.join(", "))),
                    ] {
                        if !self.manual_ipv4 || !self.values.contains_key(&id) {
                            self.values
                                .insert(id, Value::Text(value.unwrap_or_default()));
                        }
                    }
                }
                self.network = Some(network);
            }
            Update::Battery(battery) => {
                match battery.as_ref().and_then(|battery| battery.profile.clone()) {
                    Some(profile) => {
                        self.values
                            .insert(SettingId::PowerMode, Value::Keyword(profile));
                    }
                    None => {
                        self.values.remove(&SettingId::PowerMode);
                    }
                }
                self.battery = battery;
            }
            Update::Storage(storage) => {
                if storage.trash_items == 0 {
                    self.confirming_empty_trash = false;
                }
                self.storage = Some(storage);
            }
            Update::Failed(setting, error) => {
                self.busy.clear();
                match setting {
                    Some(setting) => {
                        self.errors.insert(setting, error);
                    }
                    None => self.message = Some(error),
                }
            }
            Update::Refused(refused) => self.refused = refused,
            Update::DefaultApps(defaults) => self.default_apps = Some(defaults),
            Update::Permissions(permissions) => self.permissions = permissions,
        }
        self.availability_changed();
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
        }
        self.audio = snapshot.filter(|snapshot| snapshot.connected);
    }

    fn joined(&mut self, ssid: Vec<u8>, outcome: Joined) {
        if self.join.joining.as_ref() != Some(&ssid) {
            return;
        }
        self.join.joining = None;
        match outcome {
            Joined::Joined => self.join.error = None,
            Joined::WrongPassword => {
                self.join.error = Some(L10N.tr("wrong-password"));
                self.join.asking = Some(ssid);
                self.focus_next = Some(Anchor::Password);
            }
            Joined::Failed(reason) => {
                self.join.error = Some(L10N.tr_args("join-failed", [("reason", reason.into())]));
            }
        }
    }

    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn selected_pane(&self) -> PaneId {
        self.pane
    }

    pub(crate) fn available(&self, pane: PaneId) -> bool {
        match pane {
            PaneId::Wifi => self.wifi.is_some(),
            PaneId::Bluetooth => self.bluetooth.is_some(),
            PaneId::Network => self.network.is_some(),
            PaneId::Notifications => self.values.contains_key(&SettingId::DoNotDisturb),
            PaneId::Sound => self.audio.is_some(),
            PaneId::Displays | PaneId::NightLight => !self.displays.is_empty(),
            PaneId::About => self.about.is_some(),
            PaneId::Appearance
            | PaneId::Accessibility
            | PaneId::Bar
            | PaneId::Keyboard
            | PaneId::KeyboardShortcuts
            | PaneId::Power
            | PaneId::PrivacySecurity
            | PaneId::Camera
            | PaneId::Screenshots
            | PaneId::ScreenSharing
            | PaneId::Apps
            | PaneId::Storage
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

    pub fn highlighted_settings(&self) -> Vec<SettingId> {
        self.hit(self.pane)
            .map(|hit| hit.settings.iter().map(|setting| setting.setting).collect())
            .unwrap_or_default()
    }

    pub fn focused_setting(&self) -> Option<SettingId> {
        let focus = self.ui.focus()?;
        self.anchors.iter().find_map(|(anchor, id)| match anchor {
            Anchor::Setting(setting, _) if *id == focus => Some(*setting),
            _ => None,
        })
    }

    pub fn focused_item(&self) -> Option<String> {
        let focus = self.ui.focus()?;
        self.anchors.iter().find_map(|(anchor, id)| match anchor {
            Anchor::Item(item) if *id == focus => Some(item.clone()),
            _ => None,
        })
    }

    fn anchor_focused(&self, wanted: &Anchor) -> bool {
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
            (Some((Picker::Zone, query)), Some(time)) => {
                zone_matches(&time.zones, query, PICK_RESULTS)
            }
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
        if !self.query.is_empty() {
            self.hits = self.search_hits(&self.query.clone());
            let visible = self.visible_panes();
            if !self.shown_in(&visible)
                && let Some(&first) = visible.first()
            {
                self.select_pane(first);
            }
        }
        self.invalidate();
    }

    fn invalidate(&mut self) {
        self.stale = true;
        if !self.pressed {
            self.rebuild();
        }
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        match event {
            Event::PointerDown(_) => self.pressed = true,
            Event::PointerUp(_) => self.pressed = false,
            _ => {}
        }
        let editing = self
            .focused_setting()
            .filter(|setting| self.drafts.contains_key(setting));
        let handled = match event {
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
        self.settle();
        handled
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

    pub fn settle(&mut self) {
        self.process_intents();
        let focused = self.focused_setting();
        if self.drafts.keys().any(|setting| Some(*setting) != focused) {
            self.commit_drafts();
        }
        if self.stale && !self.pressed {
            self.rebuild();
        }
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
        let intents = std::mem::take(&mut *self.intents.borrow_mut());
        for intent in intents {
            if let Intent::Draft(id, text) = intent {
                self.drafts.insert(id, text);
                continue;
            }
            self.stale = true;
            match intent {
                Intent::Draft(..) => {}
                Intent::Back => {
                    if let Some(group) = self.app_page.take() {
                        self.focus_next = Some(Anchor::Item(format!("app:{group}")));
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
                Intent::Reveal(path) => self.commands.push(Command::Reveal(path)),
                Intent::OpenApp(group) => {
                    self.focus_next = Some(Anchor::Back);
                    self.app_page = Some(group);
                    *self.scroll.borrow_mut() = ScrollState::default();
                }
                Intent::Software(command) => self.commands.push(Command::Software(command)),
                Intent::Grant(permission, app, on) => self.grant(permission, app, on),
                Intent::Network(command) => self.commands.push(Command::Network(command)),
                Intent::ImportVpn => self.commands.push(Command::ImportVpn),
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
                Intent::Locale(command) => {
                    self.errors.remove(&SettingId::Language);
                    self.errors.remove(&SettingId::Region);
                    if let Some(locale) = &mut self.locale {
                        match &command {
                            LocaleCommand::Language(language) => {
                                language.clone_into(&mut locale.language)
                            }
                            LocaleCommand::Region(region) => locale.region.clone_from(region),
                        }
                    }
                    self.commands.push(Command::Locale(command));
                }
                Intent::ManualIpv4(manual) => {
                    self.manual_ipv4 = manual;
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
                    self.message = None;
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

    fn picked(&mut self, picker: Picker, key: String) {
        match picker {
            Picker::Zone => {
                if let Some(time) = &mut self.time {
                    time.zone = key.clone();
                }
                self.commands.push(Command::Time(TimeCommand::Zone(key)));
            }
            Picker::LoginApp => self.commands.push(Command::Login(LoginCommand::Add(key))),
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
        let device = self.bluetooth.as_ref().and_then(|bluetooth| {
            bluetooth
                .paired
                .iter()
                .chain(&bluetooth.nearby)
                .find(|device| device.address == address)
                .cloned()
        });
        if let Some(device) = device {
            self.busy.push(address);
            self.message = None;
            self.commands.push(Command::Bluetooth(command(device)));
        }
    }

    fn start_join(&mut self, ssid: Vec<u8>, password: Option<String>) {
        let Some(network) = self
            .wifi
            .as_ref()
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

    pub fn select_pane(&mut self, pane: PaneId) {
        if pane == self.pane {
            return;
        }
        if self.pane == PaneId::Bluetooth {
            self.commands
                .push(Command::Bluetooth(BluetoothCommand::Discover(false)));
        }
        if self.pane == PaneId::Sound {
            self.commands
                .push(Command::Audio(AudioCommand::Listen(false)));
        }
        match pane {
            PaneId::Wifi => self.commands.push(Command::Network(NetworkCommand::Scan)),
            PaneId::Bluetooth => self
                .commands
                .push(Command::Bluetooth(BluetoothCommand::Discover(true))),
            PaneId::Storage => self
                .commands
                .push(Command::Storage(StorageCommand::Measure)),
            PaneId::Apps => {
                self.commands.push(Command::Login(LoginCommand::Refresh));
                self.commands
                    .push(Command::DefaultApp(DefaultAppCommand::Refresh));
            }
            PaneId::Notifications => self
                .commands
                .push(Command::Notifications(NotificationCommand::Refresh)),
            PaneId::SoftwareUpdate if self.software == SoftwareState::Unknown => self
                .commands
                .push(Command::Software(SoftwareCommand::Check)),
            PaneId::Sound => self
                .commands
                .push(Command::Audio(AudioCommand::Listen(true))),
            _ => {}
        }
        self.pane = pane;
        self.confirming_empty_trash = false;
        self.confirming_clear_clipboard = false;
        self.picking = None;
        self.app_page = None;
        self.errors.clear();
        *self.scroll.borrow_mut() = ScrollState::default();
        self.invalidate();
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
            Value::Number(percent) => {
                Some(L10N.tr_args("text-size-percent", [("percent", (*percent).into())]))
            }
            _ => None,
        }
    }

    fn idle(&self) -> Option<Idle> {
        let seconds = |id| match self.values.get(&id) {
            Some(Value::Seconds(seconds)) => Some(*seconds),
            _ => None,
        };
        Some(Idle {
            dim: seconds(SettingId::DimAfter)?,
            lock: seconds(SettingId::LockAfter)?,
            sleep: seconds(SettingId::SleepAfter),
        })
    }

    fn command(id: SettingId, value: Value) -> Command {
        match (id, value) {
            (SettingId::OutputVolume, Value::Fraction(volume)) => {
                Command::Audio(AudioCommand::Volume(Direction::Output, volume))
            }
            (SettingId::InputVolume, Value::Fraction(volume)) => {
                Command::Audio(AudioCommand::Volume(Direction::Input, volume))
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
            (SettingId::Brightness, Value::Fraction(level)) => Command::Brightness(level),
            (id, value) => Command::Set(id, value),
        }
    }

    fn commit_drafts(&mut self) {
        for (id, text) in std::mem::take(&mut self.drafts) {
            let text = text.trim().to_owned();
            let clearable = matches!(id, SettingId::EthernetRouter | SettingId::EthernetDns);
            if clearable || !text.is_empty() {
                self.change(id, Value::Text(text));
            }
        }
        self.invalidate();
    }

    fn change(&mut self, id: SettingId, value: Value) {
        if matches!(
            id,
            SettingId::EthernetAddress | SettingId::EthernetRouter | SettingId::EthernetDns
        ) {
            self.values.insert(id, value);
            let text = |id| match self.values.get(&id) {
                Some(Value::Text(text)) => text.clone(),
                _ => String::new(),
            };
            let manual = bande_network::parse_manual(
                &text(SettingId::EthernetAddress),
                &text(SettingId::EthernetRouter),
                &text(SettingId::EthernetDns),
            );
            self.message = match manual {
                Some(manual) => {
                    self.commands
                        .push(Command::Network(NetworkCommand::ConfigureEthernet(Some(
                            manual,
                        ))));
                    None
                }
                None => Some(L10N.tr("manual-address-invalid")),
            };
            return;
        }
        if let (SettingId::NightFrom | SettingId::NightTo, Value::Text(text)) = (id, &value) {
            match clock_text(text) {
                Some(time) => {
                    self.errors.remove(&id);
                    if self.values.get(&id) != Some(&Value::Text(time.clone())) {
                        self.values.insert(id, Value::Text(time.clone()));
                        self.commands.push(Command::Set(id, Value::Text(time)));
                    }
                }
                None => {
                    self.errors.insert(id, L10N.tr("night-time-invalid"));
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
                    self.errors.insert(id, L10N.tr("manual-time-invalid"));
                }
            }
            return;
        }
        if self.values.get(&id) == Some(&value) {
            return;
        }
        let writes = match (id, self.idle()) {
            (SettingId::DimAfter | SettingId::LockAfter | SettingId::SleepAfter, Some(idle)) => {
                idle.writes(id, &value)
            }
            _ => vec![(id, value)],
        };
        for (id, value) in writes {
            self.values.insert(id, value.clone());
            self.commands.push(Self::command(id, value));
        }
        let unmuted = match id {
            SettingId::OutputVolume => Some(SettingId::OutputMuted),
            SettingId::InputVolume => Some(SettingId::InputMuted),
            _ => None,
        };
        if let Some(muted) = unmuted {
            self.values.insert(muted, Value::Flag(false));
        }
        self.errors.remove(&id);
        self.message = None;
    }

    fn can_go_back(&self) -> bool {
        self.app_page.is_some() || self.pane.parent().is_some()
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
                .insert(pending.identity.clone(), pending.previous);
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
        self.message = None;
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

        self.message = None;
        self.commands.push(Command::Arrangement(placements));
    }

    fn change_display(&mut self, identity: String, id: SettingId, value: Value) {
        let mut display = self
            .display_settings
            .get(&identity)
            .copied()
            .unwrap_or_default();
        let previous = display;
        let mut confirm = false;
        match (id, value) {
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
            (SettingId::VariableRefreshRate, Value::Flag(on)) => {
                display.variable_refresh_rate = on.then_some(true);
            }
            _ => return,
        }
        if confirm && display != previous {
            let previous = match &self.keep {
                Some(keep) if keep.identity == identity => keep.previous,
                _ => previous,
            };
            self.keep_displays(true);
            self.keep = Some(Keep {
                identity: identity.clone(),
                previous,
                until: Instant::now() + KEEP_DISPLAY_SETTINGS,
            });
        }
        self.display_settings.insert(identity.clone(), display);
        self.message = None;
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

    pub fn accessibility(&self) -> TreeUpdate {
        self.ui.accessibility()
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        let handled = self.ui.perform(request);
        self.settle();
        handled
    }
}
