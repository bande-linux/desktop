use std::cell::Cell;
use std::os::fd::BorrowedFd;
use std::rc::Rc;

use bande_l10n::tr;

use crate::results::{self, Answer};
use bande_app::View;
use bande_inbox::Inbox;
use bande_toolkit::ui::{Axis, Button, Context, Event, Key, Label, ScrollView, Stack, Ui, Widget};
use bande_toolkit::{Appearance, Scene, Spacing, TextSystem, Window, WindowEvent};

pub const PANEL_SIZE: (u32, u32) = (420, 340);
pub const LANGUAGE_ARGUMENT: &str = "--language=";

pub struct Panel {
    ui: Ui,
    appearance: Appearance,
    word: String,
    answer: Option<Answer>,
    answers: Inbox<Answer>,
    expanded: Rc<Cell<bool>>,
    shown_expanded: bool,
    dismissed: Rc<Cell<bool>>,
}

pub fn ui_with(root: Box<dyn Widget>, text: TextSystem, appearance: Appearance) -> Ui {
    let mut ui = Ui::new(root, Context::new(text, appearance.palette()));
    ui.set_appearance(appearance);
    ui
}

impl Panel {
    pub fn new(
        appearance: Appearance,
        word: String,
        hint: Option<String>,
        answers: Inbox<Answer>,
        dismissed: Rc<Cell<bool>>,
    ) -> Self {
        let root = Box::new(Stack::new(Axis::Vertical, 0.0, 0.0, Vec::new()));
        let ui = ui_with(root, TextSystem::new(), appearance);
        let outbox = answers.outbox();
        let asked = word.clone();
        std::thread::spawn(move || results::find(&asked, hint, true, &outbox));
        Self {
            ui,
            appearance,
            word,
            answer: None,
            answers,
            expanded: Rc::new(Cell::new(false)),
            shown_expanded: false,
            dismissed,
        }
    }

    fn content(&self) -> Box<dyn Widget> {
        let padding = Spacing::M as f32;
        let expanded = self.expanded.get();
        let hits = match &self.answer {
            Some(Ok(hits)) => hits.as_slice(),
            _ => &[],
        };
        let shown = match expanded {
            true => hits.iter().collect(),
            false => results::compact(hits),
        };
        let more = !expanded && shown.len() < hits.len();
        let mut parts: Vec<Box<dyn Widget>> = vec![
            Box::new(Label::heading(&self.word)),
            Box::new(ScrollView::new(Stack::new(
                Axis::Vertical,
                0.0,
                0.0,
                vec![results::shown(&self.word, self.answer.as_ref(), shown)],
            ))),
        ];
        if more {
            let more = self.expanded.clone();
            parts.push(Box::new(Button::new(&tr!("More"), move || {
                more.set(true);
            })));
        }
        Box::new(Stack::new(Axis::Vertical, padding / 2.0, padding, parts))
    }

    fn rebuild(&mut self) {
        self.shown_expanded = self.expanded.get();
        let text = self.ui.context().text.sharing_fonts();
        self.ui = ui_with(self.content(), text, self.appearance);
    }
}

impl View for Panel {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        if let WindowEvent::Input(Event::Key {
            key: Key::Escape, ..
        }) = event
        {
            self.dismissed.set(true);
            return true;
        }
        let handled = window
            .drive(&mut self.ui, event)
            .inspect_err(|error| tracing::warn!("{error}"))
            .is_ok_and(|unhandled| unhandled.is_none());
        if self.expanded.get() != self.shown_expanded {
            self.rebuild();
        }
        handled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.appearance = appearance;
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        if !active {
            self.dismissed.set(true);
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.answers.fd()]
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) -> bool {
        let arrived = self.answers.drain();
        if arrived.is_empty() {
            return false;
        }
        results::merge(&mut self.answer, arrived);
        self.rebuild();
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }
}
