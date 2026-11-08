use std::cell::{Cell, RefCell};
use std::fs;
use std::rc::Rc;

use bande_l10n::{Text, text, tr};

use crate::panel::ui_with;
use bande_app::View;
use bande_dict::{Choices, Pack};
use bande_toolkit::ui::{Axis, Button, CheckState, Checkbox, Label, ScrollView, Stack, Ui, Widget};
use bande_toolkit::{Appearance, Scene, Spacing, TextSystem, Window, WindowEvent};

pub const SETTINGS_SIZE: (u32, u32) = (520, 460);
const ANY_LANGUAGE: &str = "*";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Change {
    Enable(String, bool),
    Move(usize, usize),
    Remove(String),
}

pub struct SourcesWindow {
    ui: Ui,
    appearance: Appearance,
    choices: Choices,
    change: Rc<RefCell<Option<Change>>>,
    changed: Rc<Cell<bool>>,
}

pub fn sources_window(appearance: Appearance) -> Box<dyn View> {
    let mut window = SourcesWindow {
        ui: ui_with(
            Box::new(Stack::new(Axis::Vertical, 0.0, 0.0, Vec::new())),
            TextSystem::new(),
            appearance,
        ),
        appearance,
        choices: Choices::load(),
        change: Rc::new(RefCell::new(None)),
        changed: Rc::new(Cell::new(false)),
    };
    window.rebuild();
    Box::new(window)
}

fn languages_of(source: &Pack) -> String {
    match source
        .languages
        .iter()
        .any(|language| language == ANY_LANGUAGE)
    {
        true => tr!("All languages"),
        false => source.languages.join(", "),
    }
}

fn detail(source: &Pack) -> String {
    let languages = languages_of(source);
    if source.online.is_some() {
        return tr!(
            "{languages}. Online: every word you look up is sent to {name}.",
            languages = languages,
            name = source.name.as_str()
        );
    }
    match source.is_users() {
        true => tr!("{languages}. Added by you.", languages = languages),
        false => languages,
    }
}

impl SourcesWindow {
    fn button(&self, label: Text, change: Change) -> Box<dyn Widget> {
        let slot = self.change.clone();
        let changed = self.changed.clone();
        Box::new(Button::new(&label.tr(), move || {
            *slot.borrow_mut() = Some(change.clone());
            changed.set(true);
        }))
    }

    fn rebuild(&mut self) {
        let languages = bande_dict::user_languages();
        let sources = self.choices.arrange(bande_dict::installed(), &languages);
        let last = sources.len().saturating_sub(1);
        let mut rows: Vec<Box<dyn Widget>> =
            vec![Box::new(Label::new(&tr!("Look Up and Dictionary show results from the sources turned on here, in this order.")).secondary())];
        for (index, source) in sources.iter().enumerate() {
            let on = self.choices.enabled(source, &languages);
            let id = source.id.clone();
            let slot = self.change.clone();
            let changed = self.changed.clone();
            let state = match on {
                true => CheckState::On,
                false => CheckState::Off,
            };
            let check = Checkbox::new(&source.name, state, move |on| {
                *slot.borrow_mut() = Some(Change::Enable(id.clone(), on));
                changed.set(true);
            });
            let mut controls: Vec<Box<dyn Widget>> = Vec::new();
            if index > 0 {
                controls.push(self.button(text!("Move Up"), Change::Move(index, index - 1)));
            }
            if index < last {
                controls.push(self.button(text!("Move Down"), Change::Move(index, index + 1)));
            }
            if source.is_users() {
                controls.push(self.button(text!("Remove"), Change::Remove(source.id.clone())));
            }
            rows.push(Box::new(Stack::new(
                Axis::Vertical,
                Spacing::XS as f32,
                0.0,
                vec![
                    Box::new(check),
                    Box::new(Label::new(&detail(source)).secondary()),
                    Box::new(Stack::new(
                        Axis::Horizontal,
                        Spacing::S as f32,
                        0.0,
                        controls,
                    )),
                ],
            )));
        }
        let padding = Spacing::L as f32;
        let root = ScrollView::new(Stack::new(Axis::Vertical, Spacing::M as f32, padding, rows));
        let text = self.ui.context().text.sharing_fonts();
        self.ui = ui_with(Box::new(root), text, self.appearance);
    }

    fn apply(&mut self, change: Change) {
        let languages = bande_dict::user_languages();
        let sources = self.choices.arrange(bande_dict::installed(), &languages);
        match change {
            Change::Enable(id, on) => self.choices.set_enabled(&id, on),
            Change::Move(from, to) => {
                let mut order: Vec<String> =
                    sources.iter().map(|source| source.id.clone()).collect();
                if from < order.len() && to < order.len() {
                    order.swap(from, to);
                }
                self.choices.order = order;
            }
            Change::Remove(id) => {
                if let Some(source) = sources
                    .iter()
                    .find(|source| source.id == id && source.is_users())
                    && let Err(error) = fs::remove_file(&source.path)
                {
                    tracing::warn!("cannot remove {}: {error}", source.path.display());
                }
                self.choices.order.retain(|known| *known != id);
                self.choices.on.retain(|known| *known != id);
                self.choices.off.retain(|known| *known != id);
            }
        }
        let saved = bande_xdg::config_home()
            .ok_or_else(|| std::io::Error::other("no configuration folder"))
            .and_then(|folder| {
                bande_xdg::replace_file(
                    &folder.join("bande").join(Choices::FILE),
                    self.choices.to_text(),
                )
            });
        if let Err(error) = saved {
            tracing::warn!("cannot save the dictionary settings: {error}");
        }
    }
}

impl View for SourcesWindow {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let mut scene = self.ui.scene(width, height);
        scene.background = window.appearance().palette().sheet;
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        let handled = window
            .drive(&mut self.ui, event)
            .inspect_err(|error| tracing::warn!("{error}"))
            .is_ok_and(|unhandled| unhandled.is_none());
        if self.changed.replace(false) {
            let change = self.change.borrow_mut().take();
            if let Some(change) = change {
                self.apply(change);
            }
            self.rebuild();
            window.request_redraw();
        }
        handled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.appearance = appearance;
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }
}
