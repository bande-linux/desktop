use std::cell::RefCell;
use std::os::fd::BorrowedFd;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bande_l10n::tr;

use crate::panel::ui_with;
use crate::results::{self, Answer};
use accesskit::{Node, Role};
use bande_app::View;
use bande_graphics::{Item, Rect};
use bande_inbox::Inbox;
use bande_toolkit::ui::{
    Axis, Context, ScrollView, SearchField, Size, Stack, Ui, Widget, WidgetId,
};
use bande_toolkit::{Appearance, Scene, Spacing, TextSystem, Window, WindowEvent};

pub const WINDOW_SIZE: (u32, u32) = (560, 520);
const ONLINE_AFTER: Duration = Duration::from_millis(600);

type Slot = Rc<RefCell<Option<Box<dyn Widget>>>>;

struct Swappable {
    id: WidgetId,
    child: [Box<dyn Widget>; 1],
    next: Slot,
}

impl Widget for Swappable {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        if let Some(next) = self.next.borrow_mut().take() {
            self.child = [next];
        }
        self.child[0].measure(cx, available)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.child[0].place(cx, frame);
    }

    fn frame(&self) -> Rect {
        self.child[0].frame()
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.child[0].paint(cx, items);
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::GenericContainer)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.child
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.child
    }
}

pub struct DictionaryWindow {
    ui: Ui,
    query: Rc<RefCell<String>>,
    asked: String,
    answer: Option<Answer>,
    answers: Option<Inbox<Answer>>,
    online_at: Option<Instant>,
    results: Slot,
}

impl DictionaryWindow {
    pub fn new(appearance: Appearance, word: &str) -> Self {
        let query = Rc::new(RefCell::new(word.to_owned()));
        let typed = query.clone();
        let search = SearchField::new(&tr!("Search"), word, move |text| {
            text.clone_into(&mut typed.borrow_mut());
        });
        let results: Slot = Rc::new(RefCell::new(None));
        let swappable = Swappable {
            id: WidgetId::next(),
            child: [Box::new(Stack::new(Axis::Vertical, 0.0, 0.0, Vec::new()))],
            next: results.clone(),
        };
        let padding = Spacing::M as f32;
        let root = Stack::new(
            Axis::Vertical,
            padding,
            padding,
            vec![Box::new(search), Box::new(ScrollView::new(swappable))],
        );
        let mut window = Self {
            ui: ui_with(Box::new(root), TextSystem::new(), appearance),
            query,
            asked: String::new(),
            answer: None,
            answers: None,
            online_at: None,
            results,
        };
        window.ask(false);
        window
    }

    fn ask(&mut self, online: bool) {
        let word = self.query.borrow().trim().to_owned();
        if word == self.asked && !online {
            return;
        }
        self.asked.clone_from(&word);
        self.answer = None;
        self.online_at = (!online && !word.is_empty()).then(|| Instant::now() + ONLINE_AFTER);
        if word.is_empty() {
            self.answers = None;
            self.show();
            return;
        }
        let Ok(answers) = Inbox::new() else {
            return;
        };
        let outbox = answers.outbox();
        self.answers = Some(answers);
        std::thread::spawn(move || results::find(&word, None, online, &outbox));
    }

    fn show(&mut self) {
        let hits = match &self.answer {
            Some(Ok(hits)) => hits.iter().collect(),
            _ => Vec::new(),
        };
        let shown = results::shown(&self.asked, self.answer.as_ref(), hits);
        *self.results.borrow_mut() = Some(shown);
    }
}

impl View for DictionaryWindow {
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
        self.ask(false);
        handled
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.answers.iter().map(Inbox::fd).collect()
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) -> bool {
        let Some(answers) = &mut self.answers else {
            return false;
        };
        let arrived = answers.drain();
        if arrived.is_empty() {
            return false;
        }
        results::merge(&mut self.answer, arrived);
        self.show();
        true
    }

    fn deadline(&self) -> Option<Instant> {
        self.online_at
    }

    fn tick(&mut self, _window: &mut Window, now: Instant) -> bool {
        if self.online_at.is_some_and(|at| at <= now) {
            self.online_at = None;
            self.asked.clear();
            self.ask(true);
        }
        false
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }
}
