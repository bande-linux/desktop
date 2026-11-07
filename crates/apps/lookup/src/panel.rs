use std::cell::Cell;
use std::os::fd::BorrowedFd;
use std::rc::Rc;

use bande_app::View;
use bande_inbox::Inbox;
use bande_toolkit::ui::{Axis, Button, Context, Event, Key, Label, ScrollView, Stack, Ui, Widget};
use bande_toolkit::{Appearance, Scene, Spacing, TextSystem, Window, WindowEvent};

use crate::L10N;
use crate::dict::{self, Hit, Preferences, discover};

pub const PANEL_SIZE: (u32, u32) = (420, 340);
pub const LANGUAGE_ARGUMENT: &str = "--language=";
const PACK_FOLDER: &str = "bande/dict";

pub type Answer = Result<Vec<Hit>, String>;

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

fn find(word: &str, hint: Option<String>) -> Answer {
    let folders: Vec<_> = bande_xdg::data_dirs()
        .into_iter()
        .map(|folder| folder.join(PACK_FOLDER))
        .collect();
    let packs = discover(&folders);
    if packs.is_empty() {
        return Err(L10N.tr("no-packs"));
    }
    let ui_languages = bande_l10n::preferred_languages();
    let word_languages: Vec<String> = hint
        .into_iter()
        .chain(ui_languages.iter().cloned())
        .collect();
    let wanted = Preferences {
        word_languages: &word_languages,
        ui_languages: &ui_languages,
        user_order: &[],
    };
    dict::look_up(&packs, word, wanted).map_err(|error| error.to_string())
}

fn ui_with(root: Box<dyn Widget>, text: TextSystem, appearance: Appearance) -> Ui {
    let mut ui = Ui::new(root, Context::new(text, appearance.palette()));
    ui.set_appearance(appearance);
    ui
}

fn part_of_speech(pos: &str) -> String {
    match pos {
        "noun" | "verb" | "adj" | "adv" => L10N.tr(&format!("pos-{pos}")),
        other => other.to_owned(),
    }
}

fn shown_hits(hits: &[Hit], expanded: bool) -> Vec<&Hit> {
    if expanded {
        return hits.iter().collect();
    }
    let mut kinds = Vec::new();
    hits.iter()
        .filter(|hit| {
            let first_of_kind = !kinds.contains(&hit.kind);
            kinds.push(hit.kind);
            first_of_kind
        })
        .collect()
}

fn hit_widgets(hit: &Hit) -> Vec<Box<dyn Widget>> {
    let mut widgets: Vec<Box<dyn Widget>> = Vec::new();
    if let Some((first, ending)) = &hit.derived {
        let note = L10N.tr_args(
            "derived",
            [
                ("first", first.as_str().into()),
                ("ending", ending.as_str().into()),
            ],
        );
        widgets.push(Box::new(Label::new(&note).secondary()));
    }
    for entry in &hit.entries {
        let title = match entry.pos.is_empty() {
            true => entry.word.clone(),
            false => format!("{} · {}", entry.word, part_of_speech(&entry.pos)),
        };
        widgets.push(Box::new(Label::new(&title).secondary()));
        for line in &entry.lines {
            let text = match line.kind {
                'O' => L10N.tr_args("form-of", [("word", line.text.as_str().into())]),
                _ => line.text.clone(),
            };
            widgets.push(Box::new(Label::new(&text)));
        }
    }
    widgets
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
        std::thread::spawn(move || outbox.send(find(&asked, hint)));
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

    fn content(&self) -> Vec<Box<dyn Widget>> {
        let padding = Spacing::M as f32;
        let mut parts: Vec<Box<dyn Widget>> = vec![Box::new(Label::heading(&self.word))];
        match &self.answer {
            None => {}
            Some(Err(message)) => parts.push(Box::new(Label::new(message).secondary())),
            Some(Ok(hits)) if hits.is_empty() => {
                let message = L10N.tr_args("no-entries", [("word", self.word.as_str().into())]);
                parts.push(Box::new(Label::new(&message).secondary()));
            }
            Some(Ok(hits)) => {
                let expanded = self.expanded.get();
                let mut body: Vec<Box<dyn Widget>> = Vec::new();
                for hit in shown_hits(hits, expanded) {
                    body.push(Box::new(Label::new(&hit.source).secondary()));
                    body.extend(hit_widgets(hit));
                    body.push(Box::new(Label::new(&hit.attribution).secondary()));
                }
                parts.push(Box::new(ScrollView::new(Stack::new(
                    Axis::Vertical,
                    Spacing::XS as f32,
                    0.0,
                    body,
                ))));
                if !expanded && shown_hits(hits, false).len() < hits.len() {
                    let more = self.expanded.clone();
                    parts.push(Box::new(Button::new(&L10N.tr("more"), move || {
                        more.set(true);
                    })));
                }
            }
        }
        vec![Box::new(Stack::new(
            Axis::Vertical,
            padding / 2.0,
            padding,
            parts,
        ))]
    }

    fn rebuild(&mut self) {
        self.shown_expanded = self.expanded.get();
        let text = self.ui.context().text.sharing_fonts();
        let mut content = self.content();
        if let Some(root) = content.pop() {
            self.ui = ui_with(root, text, self.appearance);
        }
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
        match self.answers.drain().pop() {
            Some(answer) => {
                self.answer = Some(answer);
                self.rebuild();
                true
            }
            None => false,
        }
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }
}
