#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, NodeId, Role, TreeId};
use unbound_graphics::{Palette, Point, Scheme, TextSystem};
use unbound_widgets::{
    Alert, Axis, Button, Context, Event, Key, Modifiers, Pointer, Size, Stack, TextField, Ui,
    Widget, WidgetId,
};

const AREA: Size = Size::new(480.0, 320.0);

struct Window {
    ui: Ui,
    now: Rc<Cell<Instant>>,
    field: WidgetId,
    button: WidgetId,
    clicks: Rc<Cell<u32>>,
    choices: Rc<RefCell<Vec<String>>>,
}

impl Window {
    fn new() -> Self {
        let clicks = Rc::new(Cell::new(0));
        let counter = clicks.clone();
        let field = TextField::new("Name", "Report", |_| {});
        let button = Button::new("Delete", move || counter.set(counter.get() + 1));
        let (field_id, button_id) = (field.id(), button.id());
        let stack = Stack::new(
            Axis::Vertical,
            8.0,
            20.0,
            vec![Box::new(field), Box::new(button)],
        );
        let mut ui = Ui::new(
            Box::new(stack),
            Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
        );
        let now = Rc::new(Cell::new(Instant::now()));
        let shared = now.clone();
        ui.set_clock(move || shared.get());
        ui.context().keyboard_navigation = true;
        let mut window = Self {
            ui,
            now,
            field: field_id,
            button: button_id,
            clicks,
            choices: Rc::new(RefCell::new(Vec::new())),
        };
        window.settle();
        window
    }

    fn settle(&mut self) {
        for _ in 0..40 {
            self.now.set(self.now.get() + Duration::from_millis(16));
            self.ui.scene(AREA.width as u32, AREA.height as u32);
        }
    }

    fn key(&mut self, key: Key) {
        self.ui.handle(&Event::Key {
            key,
            modifiers: Modifiers::default(),
        });
    }

    fn present(&mut self) {
        let choices = self.choices.clone();
        self.ui.present_sheet(
            Alert::new(
                "Delete “Report”?",
                "This item will be deleted immediately. You can’t undo this action.",
                move |choice| choices.borrow_mut().push(choice.to_owned()),
            )
            .cancel("Cancel", "cancel")
            .primary("Delete", "delete"),
        );
        self.settle();
    }

    fn click(&mut self, id: WidgetId) {
        let frame = self.ui.widget(id).expect("widget").frame();
        let at = Point::new(frame.x + 4.0, frame.y + 4.0);
        self.ui.handle(&Event::PointerDown(Pointer::at(at)));
        self.ui.handle(&Event::PointerUp(Pointer::at(at)));
    }
}

#[test]
fn return_chooses_the_primary_button_and_focus_comes_back_after_the_sheet_leaves() {
    let mut window = Window::new();
    window.key(Key::Tab);
    assert_eq!(window.ui.focus(), Some(window.field));
    window.present();
    assert!(window.ui.has_sheet());
    window.key(Key::Enter);
    assert_eq!(*window.choices.borrow(), ["delete"]);
    assert!(!window.ui.has_sheet());
    assert_eq!(window.ui.focus(), Some(window.field));
    window.settle();
    window.key(Key::Enter);
    assert_eq!(*window.choices.borrow(), ["delete"], "chosen once");
}

#[test]
fn escape_chooses_cancel() {
    let mut window = Window::new();
    window.present();
    window.key(Key::Escape);
    assert_eq!(*window.choices.borrow(), ["cancel"]);
}

#[test]
fn the_window_behind_a_sheet_ignores_clicks_tab_and_assistive_actions() {
    let mut window = Window::new();
    window.present();
    window.click(window.button);
    assert_eq!(window.clicks.get(), 0);
    for _ in 0..4 {
        window.key(Key::Tab);
        let focus = window.ui.focus().expect("focus stays in the sheet");
        assert_ne!(focus, window.field);
        assert_ne!(focus, window.button);
    }
    assert!(!window.ui.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: NodeId::from(window.button),
        data: None,
    }));
    let tree = window.ui.accessibility();
    let alert = tree
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::AlertDialog)
        .expect("the alert is in the tree");
    assert_eq!(alert.1.label(), Some("Delete “Report”?"));
    assert!(alert.1.is_modal());
}

#[test]
fn a_dialog_floats_in_the_upper_middle_instead_of_hanging_from_the_top() {
    let mut window = Window::new();
    let alert = Alert::new("Delete?", "", |_| {}).primary("Delete", "delete");
    let id = alert.id();
    window.ui.present_dialog(alert);
    window.settle();
    let frame = window.ui.widget(id).expect("dialog").frame();
    assert!(frame.y > 0.0, "it does not touch the top: {frame:?}");
    let above = frame.y;
    let below = AREA.height - frame.y - frame.height;
    assert!(above < below, "it sits above the middle: {frame:?}");
    window.key(Key::Enter);
    assert!(!window.ui.has_sheet());
}
