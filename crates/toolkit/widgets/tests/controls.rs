#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, NodeId, Role, Toggled, TreeId};
use unbound_graphics::{Item, Palette, Point, Rect, Scheme, TextSystem};
use unbound_widgets::{
    Axis, BUTTON_HEIGHT, Button, CheckState, Checkbox, ComboBox, Context, Event, Key, Modifiers,
    Pointer, PopUpButton, ProgressBar, RadioGroup, SegmentedControl, Size, Spinner, Stack, Stepper,
    TOOLTIP_DELAY, Ui, Widget, WidgetId,
};

const NONE: Modifiers = Modifiers {
    shift: false,
    alt: false,
    control: false,
    logo: false,
};
const AREA: Size = Size::new(320.0, 240.0);

struct Harness {
    ui: Ui,
    clock: Rc<Cell<Instant>>,
}

impl Harness {
    fn new(widget: impl Widget + 'static) -> Self {
        Self::with(vec![Box::new(widget)], false)
    }

    fn with(children: Vec<Box<dyn Widget>>, keyboard_navigation: bool) -> Self {
        let mut cx = Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light));
        cx.keyboard_navigation = keyboard_navigation;
        let mut ui = Ui::new(
            Box::new(Stack::new(Axis::Vertical, 8.0, 16.0, children)),
            cx,
        );
        let clock = Rc::new(Cell::new(Instant::now()));
        let time = clock.clone();
        ui.set_clock(move || time.get());
        ui.layout(AREA);
        Self { ui, clock }
    }

    fn advance(&mut self, by: Duration) {
        self.clock.set(self.clock.get() + by);
        self.ui.scene(AREA.width as u32, AREA.height as u32);
    }

    fn frame(&mut self, id: WidgetId) -> Rect {
        self.ui.widget(id).expect("widget").frame()
    }

    fn click(&mut self, at: Point) {
        self.ui.handle(&Event::PointerDown(Pointer::at(at)));
        self.ui.handle(&Event::PointerUp(Pointer::at(at)));
        self.ui.layout(AREA);
    }

    fn key(&mut self, key: Key) {
        self.ui.handle(&Event::Key {
            key,
            modifiers: NONE,
        });
        self.ui.layout(AREA);
    }

    fn node(&self, id: NodeId) -> Node {
        self.ui
            .accessibility()
            .nodes
            .into_iter()
            .find(|(node, _)| *node == id)
            .expect("node")
            .1
    }

    fn act(&mut self, target: NodeId, action: Action) -> bool {
        self.ui.perform(&ActionRequest {
            action,
            target_tree: TreeId::ROOT,
            target_node: target,
            data: None,
        })
    }

    fn texts(&mut self) -> usize {
        let scene = self.ui.scene(AREA.width as u32, AREA.height as u32);
        fn count(items: &[Item]) -> usize {
            items
                .iter()
                .map(|item| match item {
                    Item::Text { .. } => 1,
                    Item::Clip { items, .. }
                    | Item::Opacity { items, .. }
                    | Item::Translate { items, .. } => count(items),
                    _ => 0,
                })
                .sum()
        }
        count(&scene.items)
    }
}

fn centre(frame: Rect) -> Point {
    Point::new(frame.x + frame.width / 2.0, frame.y + frame.height / 2.0)
}

fn recorder<T: Clone + 'static>() -> (Rc<RefCell<Vec<T>>>, impl FnMut(T) + 'static) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    (log, move |value| sink.borrow_mut().push(value))
}

#[test]
fn a_checkbox_toggles_from_its_box_or_label_and_a_mixed_one_turns_on() {
    let (changes, record) = recorder();
    let checkbox = Checkbox::new("Show hidden files", CheckState::Mixed, record);
    let id = checkbox.id();
    let mut harness = Harness::new(checkbox);
    assert_eq!(harness.node(id.into()).toggled(), Some(Toggled::Mixed));
    let frame = harness.frame(id);
    harness.click(Point::new(frame.x + frame.width - 4.0, centre(frame).y));
    assert_eq!(*changes.borrow(), [true]);
    assert_eq!(harness.node(id.into()).toggled(), Some(Toggled::True));
    harness.click(Point::new(frame.x + 4.0, centre(frame).y));
    assert!(harness.act(id.into(), Action::Click));
    assert_eq!(*changes.borrow(), [true, false, true]);
    assert_eq!(harness.node(id.into()).role(), Role::CheckBox);
    assert_eq!(harness.node(id.into()).label(), Some("Show hidden files"));
}

#[test]
fn a_radio_group_selects_by_click_arrows_and_assistive_click() {
    let (choices, record) = recorder();
    let group = RadioGroup::new("Appearance", &["Light", "Dark", "Auto"], Some(0), record);
    let id = group.id();
    let mut harness = Harness::with(vec![Box::new(group)], true);
    let frame = harness.frame(id);
    harness.click(Point::new(frame.x + 4.0, frame.y + frame.height * 0.5));
    assert_eq!(*choices.borrow(), [1]);
    harness.key(Key::Tab);
    harness.key(Key::Down);
    assert_eq!(*choices.borrow(), [1, 2]);
    harness.key(Key::Down);
    assert_eq!(*choices.borrow(), [1, 2], "the last choice stays chosen");
    let group = harness.node(id.into());
    assert_eq!(group.role(), Role::RadioGroup);
    let radios = group.children().to_vec();
    assert_eq!(radios.len(), 3);
    assert_eq!(harness.node(radios[2]).toggled(), Some(Toggled::True));
    assert_eq!(harness.node(radios[0]).label(), Some("Light"));
    assert!(harness.act(radios[0], Action::Click));
    assert_eq!(*choices.borrow(), [1, 2, 0]);
}

#[test]
fn a_segmented_control_slides_its_selection_and_reduce_motion_jumps() {
    let (choices, record) = recorder();
    let control = SegmentedControl::new("View", &["Icons", "List", "Columns"], Some(0), record);
    let id = control.id();
    let mut harness = Harness::new(control);
    let frame = harness.frame(id);
    harness.click(Point::new(frame.x + frame.width - 6.0, centre(frame).y));
    assert_eq!(*choices.borrow(), [2]);
    assert!(
        harness.ui.is_animating() || {
            harness.ui.scene(320, 240);
            harness.ui.is_animating()
        }
    );
    harness.advance(Duration::from_secs(2));
    harness.ui.scene(320, 240);
    assert!(!harness.ui.is_animating(), "the selection settles");

    harness.ui.context().reduce_motion = true;
    harness.click(Point::new(frame.x + 6.0, centre(frame).y));
    harness.ui.scene(320, 240);
    assert!(
        !harness.ui.is_animating(),
        "Reduce Motion moves the selection at once"
    );
    let node = harness.node(id.into());
    assert_eq!(node.value(), Some("Icons"));
    let segments = node.children().to_vec();
    assert!(harness.act(segments[1], Action::Click));
    assert_eq!(*choices.borrow(), [2, 0, 1]);
}

#[test]
fn a_pop_up_button_opens_its_menu_with_the_current_choice_over_the_button() {
    let (choices, record) = recorder();
    let button = PopUpButton::new("Sort by", &["Name", "Date", "Size"], 1, record);
    let id = button.id();
    let mut harness = Harness::new(button);
    let frame = harness.frame(id);
    harness
        .ui
        .handle(&Event::PointerDown(Pointer::at(centre(frame))));
    let (menu, at) = harness.ui.take_menu_request().expect("the choices");
    assert_eq!(
        menu.items
            .iter()
            .map(|item| item.checked)
            .collect::<Vec<_>>(),
        [false, true, false]
    );
    let row_top = at.y + 4.0 + BUTTON_HEIGHT;
    assert!(
        (row_top - frame.y).abs() < 0.5,
        "the checked row covers the button"
    );
    assert!(harness.ui.menu_action("2"));
    assert_eq!(*choices.borrow(), [2]);
    let node = harness.node(id.into());
    assert_eq!(node.role(), Role::ComboBox);
    assert_eq!(node.value(), Some("Size"));

    harness.key(Key::Tab);
    harness.ui.context().keyboard_navigation = true;
    harness.key(Key::Tab);
    harness.key(Key::Space);
    assert!(
        harness.ui.take_menu_request().is_some(),
        "Space opens the menu"
    );
}

#[test]
fn a_stepper_steps_clamps_and_repeats_while_held() {
    let (values, record) = recorder();
    let stepper = Stepper::new("Copies", 1.0, (1.0, 5.0), 1.0, record);
    let id = stepper.id();
    let mut harness = Harness::new(stepper);
    let frame = harness.frame(id);
    let down = Point::new(frame.x + 6.0, centre(frame).y);
    harness.click(down);
    assert!(values.borrow().is_empty(), "the stepper is at its minimum");
    let up = Point::new(frame.x + frame.width - 6.0, centre(frame).y);
    harness.ui.handle(&Event::PointerDown(Pointer::at(up)));
    assert_eq!(*values.borrow(), [2.0]);
    harness.advance(Duration::from_millis(300));
    assert_eq!(*values.borrow(), [2.0]);
    for _ in 0..10 {
        harness.advance(Duration::from_millis(100));
    }
    assert_eq!(
        *values.borrow(),
        [2.0, 3.0, 4.0, 5.0],
        "holding repeats up to the maximum"
    );
    harness.ui.handle(&Event::PointerUp(Pointer::at(up)));
    let node = harness.node(id.into());
    assert_eq!(node.role(), Role::SpinButton);
    assert_eq!(node.numeric_value(), Some(5.0));
    assert!(harness.act(id.into(), Action::Decrement));
    assert_eq!(values.borrow().last(), Some(&4.0));
}

#[test]
fn progress_bars_animate_to_new_values_and_indeterminate_ones_keep_moving() {
    let bar = ProgressBar::new("Copying", Some(0.25));
    let spinner = Spinner::new("Loading");
    let busy = ProgressBar::new("Preparing", None);
    let (bar_id, spinner_id, busy_id) = (bar.id(), spinner.id(), busy.id());
    let mut harness = Harness::with(
        vec![Box::new(bar), Box::new(spinner), Box::new(busy)],
        false,
    );
    assert_eq!(harness.node(bar_id.into()).numeric_value(), Some(25.0));
    assert_eq!(harness.node(busy_id.into()).numeric_value(), None);
    assert!(harness.node(busy_id.into()).is_busy());
    assert!(harness.node(spinner_id.into()).is_busy());
    harness.ui.scene(320, 240);
    assert!(harness.ui.is_animating(), "busy indicators keep animating");
    let context = Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light));
    let bar = harness.ui.widget_as::<ProgressBar>(bar_id).expect("bar");
    bar.set_value(&context, Some(2.0));
    assert_eq!(bar.value(), Some(1.0));
}

#[test]
fn a_tooltip_appears_after_resting_on_a_button_and_clicking_hides_it() {
    let button = Button::new("Stop", || {}).tooltip("Quit the selected process");
    let id = button.id();
    let mut harness = Harness::new(button);
    let at = centre(harness.frame(id));
    let base = harness.texts();
    harness.ui.handle(&Event::PointerMove(Pointer::at(at)));
    let shown_at = harness.ui.deadline().expect("a tooltip is pending");
    assert_eq!(shown_at, harness.clock.get() + TOOLTIP_DELAY);
    assert_eq!(harness.texts(), base);
    harness.advance(TOOLTIP_DELAY);
    assert_eq!(harness.texts(), base + 1, "the tooltip shows its text");
    assert_eq!(harness.ui.deadline(), None);
    harness.ui.handle(&Event::PointerDown(Pointer::at(at)));
    assert_eq!(harness.texts(), base, "clicking dismisses it");
    harness
        .ui
        .handle(&Event::PointerMove(Pointer::at(Point::new(
            at.x + 1.0,
            at.y,
        ))));
    assert_eq!(
        harness.ui.deadline(),
        None,
        "it stays away until the pointer leaves"
    );
    assert_eq!(
        harness.node(id.into()).description(),
        Some("Quit the selected process")
    );
}

#[test]
fn keyboard_focus_shows_a_tooltip_too() {
    let first = Button::new("Back", || {}).tooltip("Show the previous folder");
    let id = first.id();
    let mut harness = Harness::with(vec![Box::new(first)], true);
    let base = harness.texts();
    harness.key(Key::Tab);
    assert_eq!(harness.ui.focus(), Some(id));
    harness.advance(TOOLTIP_DELAY);
    assert_eq!(harness.texts(), base + 1);
}

#[test]
fn a_combo_box_takes_typing_or_a_choice_from_its_menu() {
    let (texts, record) = recorder::<String>();
    let mut record = record;
    let combo = ComboBox::new("Font size", "12", &["10", "12", "14"], move |text: &str| {
        record(text.to_owned())
    });
    let id = combo.id();
    let mut harness = Harness::new(combo);
    let frame = harness.frame(id);
    harness
        .ui
        .handle(&Event::PointerDown(Pointer::at(Point::new(
            frame.x + frame.width - 6.0,
            centre(frame).y,
        ))));
    let (menu, at) = harness.ui.take_menu_request().expect("the choices");
    assert!(menu.items[1].checked);
    assert!(
        at.y >= frame.y + frame.height,
        "the choices open below the field"
    );
    assert!(harness.ui.menu_action("2"));
    assert_eq!(*texts.borrow(), ["14"]);
    harness.key(Key::Character('z'));
    harness.ui.handle(&Event::Key {
        key: Key::Character('z'),
        modifiers: Modifiers { logo: true, ..NONE },
    });
    harness.ui.layout(AREA);
    assert_eq!(texts.borrow().last().map(String::as_str), Some("14"));
    let node = harness.node(id.into());
    assert_eq!(node.role(), Role::EditableComboBox);
    harness.key(Key::Down);
    assert!(
        harness.ui.take_menu_request().is_some(),
        "Down opens the choices"
    );
}
