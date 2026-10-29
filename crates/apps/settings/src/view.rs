use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use accesskit::{Action, ActionData, ActionRequest, TreeId, TreeUpdate};
use unbound_toolkit::ui::{
    Context, Event, Key, Label, Modifiers, Slider, TextField, Ui, Widget, WidgetId,
};
use unbound_toolkit::{Palette, Rect, Scene, Spacing, TextSystem};

use crate::L10N;
use crate::follow::Update;
use crate::idle::Idle;
use crate::registry::{Kind, Pane, PaneId, Setting, SettingId, Value, panes};
use crate::search::{PaneHit, search};
use crate::widgets::{
    Caption, Captioned, Card, Column, Root, Row, Scroll, Segmented, SidebarRow, Text, Tone,
};

const PAGE_PADDING: f32 = Spacing::XL as f32;
const PAGE_SPACING: f32 = Spacing::S as f32;
const PAGE_STEP: f32 = 240.0;

#[derive(Debug, Clone, PartialEq)]
enum Intent {
    SelectPane(PaneId),
    StepPane(isize),
    Query(String),
    Set(SettingId, Value),
}

type Intents = Rc<RefCell<Vec<Intent>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Pane(PaneId),
    Setting(SettingId),
}

fn intent<T>(intents: &Intents, make: impl Fn(T) -> Intent + 'static) -> impl FnMut(T) + 'static {
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

pub struct SettingsView {
    panes: Vec<Pane>,
    values: HashMap<SettingId, Value>,
    query: String,
    hits: Vec<PaneHit>,
    pane: PaneId,
    ui: Ui,
    root: WidgetId,
    search: WidgetId,
    anchors: Vec<(Anchor, WidgetId)>,
    intents: Intents,
    changes: Vec<(SettingId, Value)>,
    message: Option<String>,
    pressed: bool,
    stale: bool,
    focus_next: Option<Anchor>,
    scroll: Rc<Cell<f32>>,
}

impl SettingsView {
    pub fn new(palette: Palette, text: TextSystem) -> Self {
        let intents: Intents = Rc::default();
        let queries = Rc::clone(&intents);
        let field = TextField::new(&L10N.tr("search"), "", move |query: &str| {
            queries.borrow_mut().push(Intent::Query(query.to_owned()));
        });
        let search = field.id();
        let placeholder = || Box::new(Column::new(0.0, 0.0, Vec::new())) as Box<dyn Widget>;
        let root = Root::new(Box::new(field), placeholder(), placeholder());
        let root_id = root.id();
        let mut cx = Context::new(text, palette);
        cx.keyboard_navigation = true;
        let panes = panes();
        let mut view = Self {
            pane: panes.first().map_or(PaneId::Appearance, |pane| pane.id),
            panes,
            values: HashMap::new(),
            query: String::new(),
            hits: Vec::new(),
            ui: Ui::new(Box::new(root), cx),
            root: root_id,
            search,
            anchors: Vec::new(),
            intents,
            changes: Vec::new(),
            message: None,
            pressed: false,
            stale: true,
            focus_next: None,
            scroll: Rc::default(),
        };
        view.rebuild();
        view
    }

    pub fn panes(&self) -> &[Pane] {
        &self.panes
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

    pub fn set_message(&mut self, message: Option<String>) {
        if self.message != message {
            self.message = message;
            self.invalidate();
        }
    }

    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Values(values) => {
                for (id, value) in values {
                    self.set_value(id, value);
                }
            }
            Update::Failed(error) => {
                self.set_message(Some(L10N.tr_args("not-changed", [("error", error.into())])))
            }
        }
    }

    pub fn take_changes(&mut self) -> Vec<(SettingId, Value)> {
        std::mem::take(&mut self.changes)
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn selected_pane(&self) -> PaneId {
        self.pane
    }

    pub fn visible_panes(&self) -> Vec<PaneId> {
        if self.query.trim().is_empty() {
            self.panes.iter().map(|pane| pane.id).collect()
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
            Anchor::Setting(setting) if *id == focus => Some(*setting),
            _ => None,
        })
    }

    pub fn search_focused(&self) -> bool {
        self.ui.focus() == Some(self.search)
    }

    fn hit(&self, pane: PaneId) -> Option<&PaneHit> {
        self.hits.iter().find(|hit| hit.pane == pane)
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
        self.process_intents();
        if self.stale && !self.pressed {
            self.rebuild();
        }
        handled
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
            self.focus_next = Some(Anchor::Setting(best.setting));
            self.invalidate();
        }
    }

    fn process_intents(&mut self) {
        let intents = std::mem::take(&mut *self.intents.borrow_mut());
        for intent in intents {
            match intent {
                Intent::SelectPane(pane) => self.select_pane(pane),
                Intent::StepPane(step) => self.step_pane(step),
                Intent::Query(query) => self.set_query(query),
                Intent::Set(id, value) => self.change(id, value),
            }
        }
    }

    fn select_pane(&mut self, pane: PaneId) {
        if pane != self.pane {
            self.pane = pane;
            self.scroll.set(0.0);
            self.stale = true;
        }
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

    fn set_query(&mut self, query: String) {
        if query == self.query {
            return;
        }
        self.hits = search(&self.panes, &query);
        self.query = query;
        let visible = self.visible_panes();
        if let Some(&first) = visible.first()
            && (!visible.contains(&self.pane) || !self.hits.is_empty())
        {
            self.select_pane(first);
        }
        self.stale = true;
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
            self.changes.push((id, value));
        }
        self.message = None;
        self.stale = true;
    }

    fn sidebar(&mut self) -> Box<dyn Widget> {
        let rows = self
            .visible_panes()
            .into_iter()
            .filter_map(|id| {
                let pane = self.panes.iter().find(|pane| pane.id == id)?;
                let selected = id == self.pane;
                let tone = if selected {
                    Tone::OnAccent
                } else {
                    Tone::Primary
                };
                let highlights = self.hit(id).map(|hit| hit.name.clone()).unwrap_or_default();
                let row = SidebarRow::new(
                    Text::new(&pane.name, tone).highlighted(highlights),
                    selected,
                    {
                        let mut select = intent(&self.intents, Intent::SelectPane);
                        move || select(id)
                    },
                    intent(&self.intents, Intent::StepPane),
                );
                self.anchors.push((Anchor::Pane(id), row.id()));
                Some(Box::new(row) as Box<dyn Widget>)
            })
            .collect();
        Box::new(Column::new(2.0, 0.0, rows))
    }

    fn control(&mut self, setting: &Setting) -> (Box<dyn Widget>, WidgetId) {
        let id = setting.id;
        let value = self.values.get(&id);
        let selected = value.and_then(|value| setting.kind.position(value));
        let choices = setting.kind.choices().to_vec();
        let choose = {
            let mut set = intent(&self.intents, move |value| Intent::Set(id, value));
            move |index: usize| {
                if let Some(choice) = choices.get(index) {
                    set(choice.value.clone());
                }
            }
        };
        match &setting.kind {
            Kind::Choices(choices) => {
                let labels: Vec<String> =
                    choices.iter().map(|choice| choice.label.clone()).collect();
                let control = Segmented::new(&setting.label, &labels, selected, choose);
                let id = control.id();
                (Box::new(control), id)
            }
            Kind::Stops { stops, low, high } => {
                let mut choose = choose;
                let slider = Slider::new(
                    &setting.label,
                    selected.unwrap_or(0) as f64,
                    (0.0, stops.len().saturating_sub(1) as f64),
                    1.0,
                    move |index| choose(index.round() as usize),
                );
                let id = slider.id();
                (Box::new(Captioned::new(Box::new(slider), low, high)), id)
            }
        }
    }

    fn content(&mut self) -> Box<dyn Widget> {
        let Some(pane) = self.panes.iter().find(|pane| pane.id == self.pane).cloned() else {
            return Box::new(Column::new(0.0, 0.0, Vec::new()));
        };
        let hit = self.hit(pane.id).cloned();
        let mut page: Vec<Box<dyn Widget>> = vec![Box::new(Label::heading(&pane.name))];
        for section in &pane.sections {
            if let Some(title) = &section.title {
                page.push(Box::new(Caption::new(
                    Text::new(title, Tone::Muted).caption().semibold(),
                )));
            }
            let rows = section
                .settings
                .iter()
                .map(|setting| {
                    let matched = hit.as_ref().and_then(|hit| hit.setting(setting.id));
                    let (control, focus) = self.control(setting);
                    self.anchors.push((Anchor::Setting(setting.id), focus));
                    let label = Text::new(&setting.label, Tone::Primary)
                        .highlighted(matched.map(|hit| hit.label.clone()).unwrap_or_default());
                    Box::new(Row::new(label, Some(control)).highlighted(matched.is_some()))
                        as Box<dyn Widget>
                })
                .collect();
            page.push(Box::new(Card::new(rows)));
        }
        if let Some(message) = &self.message {
            page.push(Box::new(Caption::new(Text::new(message, Tone::Muted))));
        }
        Box::new(Scroll::new(
            Box::new(Column::new(PAGE_SPACING, PAGE_PADDING, page)),
            Rc::clone(&self.scroll),
        ))
    }

    fn rebuild(&mut self) {
        let focused = self.focus_next.take().or_else(|| {
            let focus = self.ui.focus()?;
            self.anchors
                .iter()
                .find(|(_, id)| *id == focus)
                .map(|(anchor, _)| *anchor)
        });
        self.anchors.clear();
        let sidebar = self.sidebar();
        let content = self.content();
        if let Some(root) = self.ui.widget(self.root) {
            let children = root.children_mut();
            children[Root::PANES] = sidebar;
            children[Root::CONTENT] = content;
        }
        if let Some(target) = focused.and_then(|anchor| {
            self.anchors
                .iter()
                .find(|(candidate, _)| *candidate == anchor)
                .map(|(_, id)| *id)
        }) {
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
        self.process_intents();
        if self.stale && !self.pressed {
            self.rebuild();
        }
        handled
    }
}
