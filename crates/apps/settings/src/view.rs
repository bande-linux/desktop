use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;

use accesskit::{Action, ActionData, ActionRequest, TreeId, TreeUpdate};
use unbound_audio::{Direction, Snapshot};
use unbound_config::{MAX_INPUT_SOURCES, Output};
use unbound_toolkit::ui::{
    Context, Event, Key, Modifiers, PointerButton, SearchField, Ui, Widget, WidgetId,
};
use unbound_toolkit::{Image, Palette, Rect, Scene, TextSystem};

use crate::L10N;
use crate::about::{About, AboutCommand};
use crate::battery::{BatteryCommand, BatteryState};
use crate::follow::{Command, Update};
use crate::idle::Idle;
use crate::login::{LoginCommand, LoginItems, login_matches};
use crate::pictures::{Pair, PictureCommand};
use crate::pointing::PointingDevices;
use crate::services::{AudioCommand, BluetoothCommand, NetworkCommand};
use crate::state::{Answer, BluetoothState, Display, Joined, Question, WifiState};
use crate::storage::{Storage, StorageCommand};
use crate::time::{TimeCommand, TimeState, parse_manual, zone_matches};
use crate::widgets::{Column, Root};
use unbound_keyboard::Layouts;
use unbound_settings_panes::{AUTOMATIC, Pane, PaneId, SettingId, Value, panes};
use unbound_settings_panes::{PaneHit, search};

const PAGE_STEP: f32 = 240.0;
const PICK_RESULTS: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Intent {
    SelectPane(PaneId),
    StepPane(isize),
    Query(String),
    Set(SettingId, Value),
    Display(String, SettingId, Value),
    Default(Direction, String),
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
    Back,
    Draft(SettingId, String),
    AskEmptyTrash(bool),
    EmptyTrash,
    Reveal(PathBuf),
    Pick(Option<Picker>),
    PickQuery(String),
    Picked(String),
    RemoveLoginItem(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Picker {
    Zone,
    LoginApp,
}

impl Picker {
    fn setting(self) -> SettingId {
        match self {
            Self::Zone => SettingId::TimeZone,
            Self::LoginApp => SettingId::LoginItems,
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
    SourceSearch,
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

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Join {
    pub asking: Option<Vec<u8>>,
    pub joining: Option<Vec<u8>>,
    pub error: Option<String>,
}

pub struct SettingsView {
    pub(crate) panes: Vec<Pane>,
    pub(crate) values: HashMap<SettingId, Value>,
    pub(crate) outputs: BTreeMap<String, Output>,
    pub(crate) displays: Vec<Display>,
    pub(crate) audio: Option<Snapshot>,
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
    pub(crate) about: Option<About>,
    pub(crate) storage: Option<Storage>,
    pub(crate) drafts: HashMap<SettingId, String>,
    pub(crate) confirming_empty_trash: bool,
    pub(crate) time: Option<TimeState>,
    pub(crate) picking: Option<(Picker, String)>,
    pub(crate) login: Option<LoginItems>,
    pub(crate) battery: Option<BatteryState>,
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
            outputs: BTreeMap::new(),
            displays: Vec::new(),
            audio: None,
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
            about: None,
            storage: None,
            drafts: HashMap::new(),
            confirming_empty_trash: false,
            time: None,
            picking: None,
            login: None,
            battery: None,
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
            Update::Values(values) => {
                for (id, value) in values {
                    self.values.insert(id, value);
                }
            }
            Update::Outputs(outputs) => self.outputs = outputs,
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
                    self.picking = None;
                }
                self.storage = Some(storage);
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
            PaneId::About => self.about.is_some(),
            PaneId::Battery => self.battery.is_some(),
            PaneId::Appearance
            | PaneId::Wallpaper
            | PaneId::Dock
            | PaneId::LockScreen
            | PaneId::Keyboard
            | PaneId::General
            | PaneId::Storage
            | PaneId::DateTime
            | PaneId::LoginItems => true,
            PaneId::TrackpadMouse => self.pointing.trackpad || self.pointing.mouse,
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

    pub(crate) fn pick_results(&self) -> Vec<String> {
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
        }
        let visible = self.visible_panes();
        if !self.shown_in(&visible)
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
                key: Key::Character('['),
                modifiers,
            } if modifiers.logo && self.pane.parent().is_some() => {
                self.intents.borrow_mut().push(Intent::Back);
                true
            }
            Event::Key {
                key: Key::Left,
                modifiers,
            } if modifiers.alt && self.pane.parent().is_some() => {
                self.intents.borrow_mut().push(Intent::Back);
                true
            }
            Event::PointerDown(pointer)
                if pointer.button == PointerButton::Back && self.pane.parent().is_some() =>
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
            } if self.anchor_focused(&Anchor::SourceSearch) => {
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
                    if let Some(parent) = self.pane.parent() {
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
                Intent::EmptyTrash => {
                    self.confirming_empty_trash = false;
                    self.commands
                        .push(Command::Storage(StorageCommand::EmptyTrash));
                }
                Intent::Reveal(path) => self.commands.push(Command::Reveal(path)),
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
                    self.message = None;
                    self.picked(picker, key);
                }
                Intent::SelectPane(pane) => {
                    if pane.parent() == Some(self.pane) {
                        self.focus_next = Some(Anchor::Back);
                    }
                    self.select_pane(pane);
                }
                Intent::StepPane(step) => self.step_pane(step),
                Intent::Query(query) => self.set_query(query),
                Intent::Set(id, value) => self.change(id, value),
                Intent::Display(connector, id, value) => self.change_display(connector, id, value),
                Intent::Default(direction, name) => {
                    self.commands
                        .push(Command::Audio(AudioCommand::Default(direction, name)));
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

    fn picked(&mut self, picker: Picker, key: String) {
        match picker {
            Picker::Zone => {
                if let Some(time) = &mut self.time {
                    time.zone = key.clone();
                }
                self.commands.push(Command::Time(TimeCommand::Zone(key)));
            }
            Picker::LoginApp => self.commands.push(Command::Login(LoginCommand::Add(key))),
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
        match pane {
            PaneId::Wifi => self.commands.push(Command::Network(NetworkCommand::Scan)),
            PaneId::Bluetooth => self
                .commands
                .push(Command::Bluetooth(BluetoothCommand::Discover(true))),
            PaneId::Storage => self
                .commands
                .push(Command::Storage(StorageCommand::Measure)),
            PaneId::LoginItems => self.commands.push(Command::Login(LoginCommand::Refresh)),
            _ => {}
        }
        self.pane = pane;
        self.adding_source = None;
        self.confirming_empty_trash = false;
        self.scroll.set(0.0);
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
            (id, value) => Command::Set(id, value),
        }
    }

    fn commit_drafts(&mut self) {
        for (id, text) in std::mem::take(&mut self.drafts) {
            let text = text.trim().to_owned();
            if !text.is_empty() {
                self.change(id, Value::Text(text));
            }
        }
        self.invalidate();
    }

    fn change(&mut self, id: SettingId, value: Value) {
        if let (SettingId::DateAndTime, Value::Text(text)) = (id, &value) {
            let zone = self
                .time
                .as_ref()
                .map(|time| time.zone.clone())
                .unwrap_or_default();
            self.message = match parse_manual(text, &zone) {
                Some(at) => {
                    self.commands.push(Command::Time(TimeCommand::Set(at)));
                    None
                }
                None => Some(L10N.tr("manual-time-invalid")),
            };
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
        self.message = None;
    }

    fn change_display(&mut self, connector: String, id: SettingId, value: Value) {
        let mut output = self.outputs.get(&connector).copied().unwrap_or_default();
        match (id, value) {
            (SettingId::DisplayScale, Value::Number(percent)) => {
                output.scale = Some(f64::from(percent) / 100.0);
            }
            (SettingId::DisplayScale, Value::Keyword(keyword)) if keyword == AUTOMATIC => {
                output.scale = None;
            }
            (SettingId::RefreshRate, Value::Number(hertz)) => {
                output.refresh_rate = Some(f64::from(hertz));
            }
            (SettingId::VariableRefreshRate, Value::Flag(on)) => {
                output.variable_refresh_rate = on.then_some(true);
            }
            _ => return,
        }
        self.outputs.insert(connector.clone(), output);
        self.message = None;
        self.commands.push(Command::Output(connector, output));
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
        let typed = match &focused {
            Some(Anchor::SourceSearch) => Some(self.adding_source.clone().unwrap_or_default()),
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
                self.intents.borrow_mut().retain(|intent| {
                    !matches!(
                        intent,
                        Intent::SourceQuery(_) | Intent::PickQuery(_) | Intent::Draft(..)
                    )
                });
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
