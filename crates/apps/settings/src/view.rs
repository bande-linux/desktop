use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use accesskit::{Action, ActionData, ActionRequest, TreeId, TreeUpdate};
use unbound_audio::{Direction, Snapshot};
use unbound_config::Output;
use unbound_toolkit::ui::{Context, Event, Key, Modifiers, SearchField, Ui, Widget, WidgetId};
use unbound_toolkit::{Image, Palette, Rect, Scene, TextSystem};

use crate::L10N;
use crate::follow::{Command, Update};
use crate::idle::Idle;
use crate::pictures::{Pair, PictureCommand};
use crate::services::{AudioCommand, BluetoothCommand, NetworkCommand};
use crate::state::{Answer, BluetoothState, Display, Joined, Question, WifiState};
use crate::widgets::{Column, Root};
use unbound_settings_panes::{AUTOMATIC, Pane, PaneId, SettingId, Value, panes};
use unbound_settings_panes::{PaneHit, search};

const PAGE_STEP: f32 = 240.0;

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
}

pub(crate) type Intents = Rc<RefCell<Vec<Intent>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Anchor {
    Pane(PaneId),
    Setting(SettingId, Option<String>),
    Item(String),
    Password,
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
        match pane {
            PaneId::Wifi => self.commands.push(Command::Network(NetworkCommand::Scan)),
            PaneId::Bluetooth => self
                .commands
                .push(Command::Bluetooth(BluetoothCommand::Discover(true))),
            _ => {}
        }
        self.pane = pane;
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
