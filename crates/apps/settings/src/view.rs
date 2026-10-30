use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use accesskit::{Action, ActionData, ActionRequest, TreeId, TreeUpdate};
use std::time::{Duration, Instant};
use unbound_audio::{Direction, Snapshot, meter_level};
use unbound_config::{DisplaySettings, MAX_INPUT_SOURCES, Placement, Rotation};
use unbound_toolkit::ui::{Context, Event, Key, Modifiers, SearchField, Ui, Widget, WidgetId};
use unbound_toolkit::{Image, Palette, Rect, Scene, TextSystem};

use crate::L10N;
use crate::follow::{Command, Update};
use crate::idle::Idle;
use crate::pictures::{Pair, PictureCommand};
use crate::pointing::PointingDevices;
use crate::services::{AudioCommand, BluetoothCommand, NetworkCommand};
use crate::state::{Answer, BluetoothState, Display, Joined, Question, WifiState};
use crate::widgets::{Column, Root};
use unbound_keyboard::Layouts;
use unbound_settings_panes::{AUTOMATIC, Pane, PaneId, SettingId, Value, panes};
use unbound_settings_panes::{PaneHit, search};

const PAGE_STEP: f32 = 240.0;
const SECOND: Duration = Duration::from_secs(1);
pub(crate) const USE_MAIN: &str = "main";
pub(crate) const USE_EXTENDED: &str = "extended";
pub(crate) const MIRROR_PREFIX: &str = "mirror:";

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Intent {
    SelectPane(PaneId),
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
    Answer(Answer),
    ChoosePicture,
    AddingSource(bool),
    SourceQuery(String),
    AddSource(String),
    RemoveSource(usize),
    MoveSource(usize, bool),
}

pub(crate) type Intents = Rc<RefCell<Vec<Intent>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Anchor {
    Pane(PaneId),
    Setting(SettingId, Option<String>),
    Item(String),
    Password,
    SourceSearch,
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
    pub(crate) adding_source: Option<String>,
    pub(crate) pointing: PointingDevices,
    ui: Ui,
    root: WidgetId,
    search: WidgetId,
    pub(crate) anchors: Vec<(Anchor, WidgetId)>,
    pub(crate) intents: Intents,
    pub(crate) commands: Vec<Command>,
    pub(crate) message: Option<String>,
    pressed: bool,
    stale: bool,
    focus_next: Option<Anchor>,
    pub(crate) scroll: Rc<Cell<f32>>,
}

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
            adding_source: None,
            pointing: PointingDevices::default(),
            ui: Ui::new(Box::new(root), cx),
            root: root_id,
            search,
            anchors: Vec::new(),
            intents,
            commands: Vec::new(),
            message: None,
            pressed: false,
            stale: true,
            focus_next: None,
            scroll: Rc::default(),
        };
        view.rebuild();
        view
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.ui.context().palette = palette;
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

    pub fn adding_source(&self) -> Option<&str> {
        self.adding_source.as_deref()
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
                    self.busy.retain(|address| {
                        !bluetooth
                            .paired
                            .iter()
                            .any(|device| device.address == *address)
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
            Update::Failed(error) => {
                self.busy.clear();
                self.message = Some(L10N.tr_args("not-changed", [("error", error.into())]));
            }
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
            PaneId::Notifications => self.values.contains_key(&SettingId::DoNotDisturb),
            PaneId::Sound => self.audio.is_some(),
            PaneId::Displays => !self.displays.is_empty(),
            PaneId::Appearance
            | PaneId::Wallpaper
            | PaneId::Dock
            | PaneId::LockScreen
            | PaneId::Keyboard => true,
            PaneId::TrackpadMouse => self.pointing.trackpad || self.pointing.mouse,
        }
    }

    pub fn visible_panes(&self) -> Vec<PaneId> {
        if self.query.trim().is_empty() {
            self.panes
                .iter()
                .map(|pane| pane.id)
                .filter(|pane| self.available(*pane))
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

    fn source_search_focused(&self) -> bool {
        let focus = self.ui.focus();
        self.anchors
            .iter()
            .any(|(anchor, id)| *anchor == Anchor::SourceSearch && Some(*id) == focus)
    }

    pub fn search_focused(&self) -> bool {
        self.ui.focus() == Some(self.search)
    }

    pub(crate) fn hit(&self, pane: PaneId) -> Option<&PaneHit> {
        self.hits.iter().find(|hit| hit.pane == pane)
    }

    fn availability_changed(&mut self) {
        if !self.query.is_empty() {
            self.hits = self.search_hits(&self.query.clone());
        }
        let visible = self.visible_panes();
        if !visible.contains(&self.pane)
            && let Some(&first) = visible.first()
        {
            self.select_pane(first);
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
        let handled = match event {
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
            } if plain(modifiers) && self.adding_source.is_some() => {
                self.intents.borrow_mut().push(Intent::AddingSource(false));
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
            } if self.source_search_focused() => {
                let query = self.adding_source.clone().unwrap_or_default();
                let added = self.input_sources();
                if let Some((code, _)) = self.source_matches(&query, &added).into_iter().next() {
                    self.intents.borrow_mut().push(Intent::AddSource(code));
                }
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
                let step = if *key == Key::PageUp {
                    -PAGE_STEP
                } else {
                    PAGE_STEP
                };
                self.scroll.set(self.scroll.get() + step);
                true
            }
            _ => self.ui.handle(event),
        };
        let handled = handled || self.type_into_search(event);
        self.settle();
        handled
    }

    fn settle(&mut self) {
        self.process_intents();
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
            self.stale = true;
            match intent {
                Intent::SelectPane(pane) => self.select_pane(pane),
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
                Intent::Answer(answer) => {
                    self.pairing = None;
                    self.commands.push(Command::Answer(answer));
                }
                Intent::ChoosePicture => {
                    self.commands.push(Command::Picture(PictureCommand::Choose));
                }
                Intent::AddingSource(adding) => {
                    self.adding_source = adding.then(String::new);
                    self.focus_next = Some(if adding {
                        Anchor::SourceSearch
                    } else {
                        Anchor::Setting(SettingId::InputSources, None)
                    });
                }
                Intent::SourceQuery(query) => {
                    if self.adding_source.is_some() {
                        self.adding_source = Some(query);
                        self.focus_next = Some(Anchor::SourceSearch);
                    }
                }
                Intent::AddSource(code) => {
                    self.adding_source = None;
                    self.focus_next = Some(Anchor::Item(code.clone()));
                    self.edit_sources(|sources| {
                        if !sources.contains(&code) {
                            sources.push(code);
                        }
                    });
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

    fn device_command(
        &mut self,
        address: String,
        command: fn(unbound_bluetooth::Device) -> BluetoothCommand,
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
            PaneId::Sound => self
                .commands
                .push(Command::Audio(AudioCommand::Listen(true))),
            _ => {}
        }
        self.pane = pane;
        self.adding_source = None;
        self.scroll.set(0.0);
        self.stale = true;
    }

    fn step_pane(&mut self, step: isize) {
        let visible = self.visible_panes();
        let Some(at) = visible.iter().position(|pane| *pane == self.pane) else {
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
            && (!visible.contains(&self.pane) || !self.hits.is_empty())
        {
            self.select_pane(first);
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
            (id, value) => Command::Set(id, value),
        }
    }

    fn change(&mut self, id: SettingId, value: Value) {
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
        self.message = None;
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
            .find(|placements| unbound_config::same_displays(placements, &identities));
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
        let typing_source = focused == Some(Anchor::SourceSearch);
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
            if typing_source {
                let query = self.adding_source.clone().unwrap_or_default();
                self.ui.perform(&request(
                    Action::SetValue,
                    target,
                    Some(ActionData::Value(query.into())),
                ));
                self.intents
                    .borrow_mut()
                    .retain(|intent| !matches!(intent, Intent::SourceQuery(_)));
            }
        }
        self.stale = false;
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
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
