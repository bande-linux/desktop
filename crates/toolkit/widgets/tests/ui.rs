#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, Role, Toggled, TreeId};
use bande_graphics::{Palette, Point, Rect, Scheme, TextSystem};
use bande_widgets::{
    Axis, BUTTON_HEIGHT, Button, ClipboardRequest, Context, ContextMenu, Cursor, DISCLOSURE_HEIGHT,
    Disclosure, Event, FreeLayout, Key, Label, Menu, MenuItem, Modifiers, Pointer, PointerButton,
    SWITCH_SIZE, Scroll, SecureField, Size, Slider, Stack, TEXT_FIELD_HEIGHT, TextField, Toggle,
    Ui, Widget, WidgetId,
};

const NONE: Modifiers = Modifiers {
    shift: false,
    alt: false,
    control: false,
    logo: false,
};
const SHIFT: Modifiers = Modifiers {
    shift: true,
    ..NONE
};
const CTRL: Modifiers = Modifiers {
    control: true,
    ..NONE
};
const AREA: Size = Size::new(320.0, 240.0);

fn context() -> Context {
    Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light))
}

fn laid_out(widget: &mut dyn Widget, cx: &mut Context) {
    widget.measure(cx, AREA);
    widget.place(cx, Rect::new(0.0, 0.0, AREA.width, AREA.height));
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, modifiers }
}

fn centre(frame: Rect) -> Point {
    Point::new(frame.x + frame.width / 2.0, frame.y + frame.height / 2.0)
}

fn node(ui: &Ui, id: WidgetId) -> Node {
    let wanted = ui
        .accessibility()
        .nodes
        .into_iter()
        .find(|(node, _)| *node == NodeId::from(id));
    wanted.expect("the widget has a node").1
}

struct Field {
    cx: Context,
    field: TextField,
    changes: Rc<RefCell<Vec<String>>>,
}

impl Field {
    fn with(text: &str) -> Self {
        let changes = Rc::new(RefCell::new(Vec::new()));
        let log = changes.clone();
        let mut field = TextField::new("Name", text, move |text| {
            log.borrow_mut().push(text.to_owned())
        });
        let mut cx = context();
        laid_out(&mut field, &mut cx);
        let mut this = Self { cx, field, changes };
        this.press(Key::Down, NONE);
        this
    }

    fn press(&mut self, pressed: Key, modifiers: Modifiers) {
        self.field.handle(&mut self.cx, &key(pressed, modifiers));
        laid_out(&mut self.field, &mut self.cx);
    }

    fn selected(&self) -> String {
        self.field.text()[self.field.selection()].to_owned()
    }
}

#[test]
fn home_and_end_move_to_the_line_ends_and_shift_extends() {
    let mut field = Field::with("hello brave world");
    field.press(Key::Home, NONE);
    assert_eq!(field.field.selection(), 0..0);
    field.press(Key::End, SHIFT);
    assert_eq!(field.selected(), "hello brave world");
    field.press(Key::End, NONE);
    assert_eq!(field.field.selection(), 17..17);
}

#[test]
fn ctrl_arrows_move_by_word_and_shift_extends() {
    let mut field = Field::with("hello brave world");
    field.press(Key::Left, CTRL);
    assert_eq!(field.field.selection(), 12..12);
    field.press(
        Key::Left,
        Modifiers {
            shift: true,
            ..CTRL
        },
    );
    assert_eq!(field.selected(), "brave ");
    field.press(Key::Right, CTRL);
    field.press(Key::Left, SHIFT);
    assert_eq!(field.selected(), "e");
}

#[test]
fn deletion_follows_the_same_units() {
    let mut field = Field::with("hello brave world");
    field.press(Key::Backspace, CTRL);
    assert_eq!(field.field.text(), "hello brave ");
    field.press(Key::Backspace, NONE);
    assert_eq!(field.field.text(), "hello brave");
    field.press(Key::Home, SHIFT);
    field.press(Key::Backspace, NONE);
    assert_eq!(field.field.text(), "");
    assert_eq!(
        field.changes.borrow().as_slice(),
        ["hello brave ", "hello brave", ""]
    );
}

#[test]
fn typing_replaces_the_selection() {
    let mut field = Field::with("hello world");
    field.press(
        Key::Left,
        Modifiers {
            shift: true,
            ..CTRL
        },
    );
    for character in "there".chars() {
        field.press(Key::Character(character), NONE);
    }
    field.press(Key::Space, NONE);
    assert_eq!(field.field.text(), "hello there ");
    field.press(Key::Character('a'), CTRL);
    assert_eq!(field.selected(), "hello there ");
}

#[test]
fn single_line_fields_move_to_the_text_ends_with_up_and_down() {
    let mut field = Field::with("one two");
    field.press(Key::Up, NONE);
    assert_eq!(field.field.selection(), 0..0);
    field.press(Key::Down, SHIFT);
    assert_eq!(field.selected(), "one two");
}

fn form(keyboard_navigation: bool) -> (Ui, Vec<WidgetId>) {
    let first = TextField::new("First", "", |_| {});
    let button = Button::new("Apply", || {});
    let toggle = Toggle::new("Wi-Fi", false, |_| {});
    let slider = Slider::new("Volume", 0.5, (0.0, 1.0), 0.1, |_| {});
    let second = TextField::new("Second", "", |_| {});
    let ids = vec![
        first.id(),
        button.id(),
        toggle.id(),
        slider.id(),
        second.id(),
    ];
    let stack = Stack::new(
        Axis::Vertical,
        8.0,
        16.0,
        vec![
            Box::new(first),
            Box::new(button),
            Box::new(toggle),
            Box::new(slider),
            Box::new(second),
        ],
    );
    let mut cx = context();
    cx.keyboard_navigation = keyboard_navigation;
    let mut ui = Ui::new(Box::new(stack), cx);
    ui.layout(AREA);
    (ui, ids)
}

#[test]
fn tab_cycles_through_text_fields_only_by_default() {
    let (mut ui, ids) = form(false);
    let mut order = Vec::new();
    for _ in 0..3 {
        ui.handle(&key(Key::Tab, NONE));
        order.push(ui.focus());
    }
    assert_eq!(order, [Some(ids[0]), Some(ids[4]), Some(ids[0])]);
    ui.handle(&key(Key::Tab, SHIFT));
    assert_eq!(ui.focus(), Some(ids[4]));
}

#[test]
fn keyboard_navigation_tabs_through_every_control_in_order() {
    let (mut ui, ids) = form(true);
    let order: Vec<_> = (0..5)
        .map(|_| {
            ui.handle(&key(Key::Tab, NONE));
            ui.focus().expect("focus")
        })
        .collect();
    assert_eq!(order, ids);
}

#[test]
fn a_vertical_stack_places_children_below_each_other_with_its_spacing() {
    let (mut ui, ids) = form(false);
    let frames: Vec<Rect> = ids
        .iter()
        .map(|id| ui.widget(*id).expect("widget").frame())
        .collect();
    assert_eq!(frames[0], Rect::new(16.0, 16.0, 288.0, TEXT_FIELD_HEIGHT));
    assert_eq!(frames[1].y, 16.0 + TEXT_FIELD_HEIGHT + 8.0);
    assert_eq!(frames[1].height, BUTTON_HEIGHT);
    assert_eq!(frames[2].y, frames[1].y + BUTTON_HEIGHT + 8.0);
    assert_eq!(
        (frames[2].width, frames[2].height),
        (SWITCH_SIZE.width, SWITCH_SIZE.height)
    );
}

#[test]
fn a_horizontal_stack_centres_children_vertically() {
    let label = Label::new("Volume");
    let toggle = Toggle::new("Mute", false, |_| {});
    let (label_id, toggle_id) = (label.id(), toggle.id());
    let mut ui = Ui::new(
        Box::new(Stack::new(
            Axis::Horizontal,
            12.0,
            0.0,
            vec![Box::new(label), Box::new(toggle)],
        )),
        context(),
    );
    ui.layout(AREA);
    let label = ui.widget(label_id).expect("label").frame();
    let toggle = ui.widget(toggle_id).expect("toggle").frame();
    assert_eq!(toggle.x, label.x + label.width + 12.0);
    assert!((centre(label).y - centre(toggle).y).abs() <= 0.5);
}

#[test]
fn a_secondary_click_asks_the_widget_under_the_pointer_for_its_menu_and_routes_the_choice_back() {
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let sink = chosen.clone();
    let area = ContextMenu::new(
        Label::new("Report.pdf"),
        || Menu::new(vec![MenuItem::new("Rename", "rename")]),
        move |action| sink.borrow_mut().push(action.to_owned()),
    );
    let id = area.id();
    let mut ui = Ui::new(Box::new(area), context());
    ui.layout(AREA);
    let at = centre(ui.widget(id).expect("area").frame());
    ui.handle(&Event::PointerDown(Pointer::at(at)));
    assert!(
        ui.take_menu_request().is_none(),
        "a primary click has no menu"
    );
    ui.handle(&Event::PointerUp(Pointer::at(at)));
    let secondary = Pointer {
        button: PointerButton::Secondary,
        ..Pointer::at(at)
    };
    ui.handle(&Event::PointerDown(secondary));
    let (menu, point) = ui.take_menu_request().expect("a menu");
    assert_eq!(point, at);
    assert_eq!(menu.items.len(), 1);
    assert!(ui.menu_action("rename"));
    assert_eq!(*chosen.borrow(), ["rename"]);
    assert!(!ui.menu_action("rename"), "a choice is delivered once");
}

#[test]
fn return_and_escape_press_the_primary_and_cancel_buttons_from_a_text_field() {
    let pressed = Rc::new(RefCell::new(Vec::new()));
    let (save, cancel) = (pressed.clone(), pressed.clone());
    let field = TextField::new("Name", "", |_| {});
    let stack = Stack::new(
        Axis::Vertical,
        8.0,
        0.0,
        vec![
            Box::new(field),
            Box::new(Button::new("Cancel", move || cancel.borrow_mut().push("cancel")).cancel()),
            Box::new(Button::new("Save", move || save.borrow_mut().push("save")).primary()),
        ],
    );
    let mut ui = Ui::new(Box::new(stack), context());
    ui.layout(AREA);
    ui.handle(&key(Key::Tab, NONE));
    ui.handle(&key(Key::Character('x'), NONE));
    ui.handle(&key(Key::Enter, NONE));
    ui.handle(&key(Key::Escape, NONE));
    ui.handle(&key(Key::Enter, SHIFT));
    assert_eq!(*pressed.borrow(), ["save", "cancel"]);
}

#[test]
fn ctrl_c_x_and_v_go_through_the_clipboard() {
    let field = TextField::new("Name", "Quarterly report", |_| {});
    let id = field.id();
    let mut ui = Ui::new(Box::new(field), context());
    ui.layout(AREA);
    ui.handle(&key(Key::Tab, NONE));
    ui.handle(&key(Key::Down, NONE));
    ui.handle(&key(
        Key::Left,
        Modifiers {
            shift: true,
            ..CTRL
        },
    ));
    ui.handle(&key(Key::Character('c'), CTRL));
    assert_eq!(
        ui.take_clipboard_request(),
        Some(ClipboardRequest::Copy("report".to_owned()))
    );
    ui.handle(&key(Key::Character('x'), CTRL));
    assert_eq!(
        ui.take_clipboard_request(),
        Some(ClipboardRequest::Copy("report".to_owned()))
    );
    assert!(!ui.pasted("ignored"), "nothing was asked for");
    ui.handle(&key(Key::Character('v'), CTRL));
    assert_eq!(ui.take_clipboard_request(), Some(ClipboardRequest::Paste));
    assert!(ui.pasted("summary"));
    let text = ui
        .widget_as::<TextField>(id)
        .expect("field")
        .text()
        .to_owned();
    assert_eq!(text, "Quarterly summary");
}

#[test]
fn shift_f10_opens_the_menu_of_the_focused_control_below_it() {
    let field = TextField::new("Name", "", |_| {});
    let field_id = field.id();
    let area = ContextMenu::new(
        field,
        || Menu::new(vec![MenuItem::new("Paste", "paste")]),
        |_| {},
    );
    let mut ui = Ui::new(Box::new(area), context());
    ui.layout(Size::new(200.0, TEXT_FIELD_HEIGHT));
    ui.handle(&key(Key::Function(10), NONE));
    assert!(
        ui.take_menu_request().is_none(),
        "F10 alone is not a menu key"
    );
    ui.handle(&key(Key::Tab, NONE));
    assert_eq!(ui.focus(), Some(field_id));
    ui.handle(&key(Key::Function(10), SHIFT));
    let frame = ui.widget(field_id).expect("field").frame();
    let (_, at) = ui.take_menu_request().expect("a menu");
    assert_eq!(at, Point::new(frame.x, frame.y + frame.height));
}

#[test]
fn buttons_click_on_release_inside_like_macos() {
    let clicks = Rc::new(Cell::new(0));
    let counter = clicks.clone();
    let button = Button::new("Apply", move || counter.set(counter.get() + 1));
    let id = button.id();
    let mut ui = Ui::new(Box::new(button), context());
    ui.layout(Size::new(120.0, BUTTON_HEIGHT));
    let frame = ui.widget(id).expect("button").frame();
    ui.handle(&Event::PointerDown(Pointer::at(centre(frame))));
    assert_eq!(clicks.get(), 0, "nothing happens on press");
    ui.handle(&Event::PointerUp(Pointer::at(centre(frame))));
    assert_eq!(clicks.get(), 1);
    ui.handle(&Event::PointerDown(Pointer::at(centre(frame))));
    ui.handle(&Event::PointerMove(Pointer::at(Point::new(500.0, 500.0))));
    ui.handle(&Event::PointerUp(Pointer::at(Point::new(500.0, 500.0))));
    assert_eq!(clicks.get(), 1, "releasing outside cancels");
}

#[test]
fn switches_flip_on_click_and_report_their_state() {
    let (mut ui, ids) = form(true);
    let toggle = ids[2];
    assert_eq!(node(&ui, toggle).toggled(), Some(Toggled::False));
    let frame = ui.widget(toggle).expect("toggle").frame();
    ui.handle(&Event::PointerDown(Pointer::at(centre(frame))));
    ui.handle(&Event::PointerUp(Pointer::at(centre(frame))));
    assert_eq!(node(&ui, toggle).toggled(), Some(Toggled::True));
    ui.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: NodeId::from(toggle),
        data: None,
    });
    assert_eq!(node(&ui, toggle).toggled(), Some(Toggled::False));
}

#[test]
fn sliders_step_with_arrows_and_accessibility_actions() {
    let (mut ui, ids) = form(true);
    let slider = ids[3];
    for _ in 0..4 {
        ui.handle(&key(Key::Tab, NONE));
    }
    assert_eq!(ui.focus(), Some(slider));
    ui.handle(&key(Key::Right, NONE));
    assert!((node(&ui, slider).numeric_value().expect("value") - 0.6).abs() < 1e-9);
    let request = |action, data| ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: NodeId::from(slider),
        data,
    };
    ui.perform(&request(Action::Decrement, None));
    ui.perform(&request(Action::Decrement, None));
    assert!((node(&ui, slider).numeric_value().expect("value") - 0.4).abs() < 1e-9);
    ui.perform(&request(
        Action::SetValue,
        Some(ActionData::NumericValue(7.0)),
    ));
    assert_eq!(node(&ui, slider).numeric_value(), Some(1.0));
    let frame = ui.widget(slider).expect("slider").frame();
    ui.handle(&Event::PointerDown(Pointer::at(Point::new(
        frame.x,
        centre(frame).y,
    ))));
    ui.handle(&Event::PointerUp(Pointer::at(Point::new(
        frame.x,
        centre(frame).y,
    ))));
    assert_eq!(node(&ui, slider).numeric_value(), Some(0.0));
}

#[test]
fn every_widget_appears_in_the_accessibility_tree_with_its_role_and_bounds() {
    let (mut ui, ids) = form(false);
    ui.handle(&key(Key::Tab, NONE));
    let tree = ui.accessibility();
    assert_eq!(tree.focus, NodeId::from(ids[0]));
    let roles: Vec<Role> = ids.iter().map(|id| node(&ui, *id).role()).collect();
    assert_eq!(
        roles,
        [
            Role::TextInput,
            Role::Button,
            Role::Switch,
            Role::Slider,
            Role::TextInput
        ]
    );
    assert_eq!(node(&ui, ids[1]).label(), Some("Apply"));
    assert_eq!(node(&ui, ids[2]).label(), Some("Wi-Fi"));
    let root = tree.tree.expect("tree info").root;
    let (_, root_node) = tree
        .nodes
        .iter()
        .find(|(id, _)| *id == root)
        .expect("root node");
    assert_eq!(root_node.children().len(), ids.len());
    for id in &ids {
        let frame = ui.widget(*id).expect("widget").frame();
        let bounds = node(&ui, *id).bounds().expect("bounds");
        assert_eq!(
            (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
            (
                f64::from(frame.x),
                f64::from(frame.y),
                f64::from(frame.x + frame.width),
                f64::from(frame.y + frame.height)
            )
        );
    }
}

#[test]
fn assistive_technology_can_set_a_text_fields_value() {
    let (mut ui, ids) = form(false);
    ui.perform(&ActionRequest {
        action: Action::SetValue,
        target_tree: TreeId::ROOT,
        target_node: NodeId::from(ids[0]),
        data: Some(ActionData::Value("Ada".into())),
    });
    ui.layout(AREA);
    assert_eq!(node(&ui, ids[0]).value(), Some("Ada"));
    ui.perform(&ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: NodeId::from(ids[4]),
        data: None,
    });
    assert_eq!(ui.focus(), Some(ids[4]));
}

struct Secret {
    cx: Context,
    field: SecureField,
    submitted: Rc<RefCell<Vec<String>>>,
}

impl Secret {
    fn new() -> Self {
        let submitted = Rc::new(RefCell::new(Vec::new()));
        let log = submitted.clone();
        let mut field = SecureField::new("Password", 13.0, move |secret| {
            log.borrow_mut().push(secret.to_owned())
        });
        let mut cx = context();
        laid_out(&mut field, &mut cx);
        Self {
            cx,
            field,
            submitted,
        }
    }

    fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            self.press(Key::Character(character), NONE);
        }
    }

    fn press(&mut self, pressed: Key, modifiers: Modifiers) -> bool {
        self.field.handle(&mut self.cx, &key(pressed, modifiers))
    }

    fn submit(&mut self) -> String {
        self.press(Key::Enter, NONE);
        self.submitted
            .borrow()
            .last()
            .cloned()
            .expect("a submission")
    }
}

#[test]
fn a_secure_field_submits_what_was_typed_and_backspace_removes_one_character() {
    let mut secret = Secret::new();
    secret.type_text("hunter2");
    secret.press(Key::Backspace, NONE);
    assert_eq!(secret.submit(), "hunter");
}

#[test]
fn ctrl_backspace_clears_a_secure_field_since_its_words_are_hidden() {
    let mut secret = Secret::new();
    secret.type_text("two words");
    secret.press(Key::Backspace, CTRL);
    assert_eq!(secret.submit(), "");
}

#[test]
fn a_secure_field_does_not_take_copy_or_paste_shortcuts() {
    let mut secret = Secret::new();
    secret.type_text("abc");
    assert!(!secret.press(Key::Character('c'), CTRL));
    assert!(!secret.press(Key::Character('v'), CTRL));
    assert_eq!(secret.submit(), "abc");
}

#[test]
fn a_secure_field_is_a_password_input_that_never_exposes_its_value() {
    let mut secret = Secret::new();
    secret.type_text("abc");
    let node = secret.field.access(&secret.cx);
    assert_eq!(node.role(), Role::PasswordInput);
    assert_eq!(node.value(), None);
}

struct Probe {
    id: WidgetId,
    frame: Rect,
    log: Rc<RefCell<Vec<String>>>,
    takes_scroll: bool,
}

impl Probe {
    fn boxed(log: &Rc<RefCell<Vec<String>>>, takes_scroll: bool) -> (WidgetId, Box<dyn Widget>) {
        let id = WidgetId::next();
        let probe = Self {
            id,
            frame: Rect::default(),
            log: log.clone(),
            takes_scroll,
        };
        (id, Box::new(probe))
    }
}

impl Widget for Probe {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, _available: Size) -> Size {
        Size::new(100.0, 40.0)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, _items: &mut Vec<bande_graphics::Item>) {}

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::GenericContainer)
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        let name = match event {
            Event::PointerEnter(_) => "enter",
            Event::PointerLeave => "leave",
            Event::Scroll(_) if self.takes_scroll => "scroll",
            _ => return false,
        };
        self.log.borrow_mut().push(format!("{name} {:?}", self.id));
        true
    }
}

#[test]
fn widgets_hear_the_pointer_enter_and_leave_as_it_moves_between_them() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let (top, first) = Probe::boxed(&log, false);
    let (bottom, second) = Probe::boxed(&log, false);
    let mut ui = Ui::new(
        Box::new(Stack::new(Axis::Vertical, 0.0, 0.0, vec![first, second])),
        context(),
    );
    ui.layout(AREA);
    ui.handle(&Event::PointerMove(Pointer::at(Point::new(10.0, 10.0))));
    ui.handle(&Event::PointerMove(Pointer::at(Point::new(20.0, 12.0))));
    ui.handle(&Event::PointerMove(Pointer::at(Point::new(10.0, 50.0))));
    ui.handle(&Event::PointerLeave);
    assert_eq!(
        *log.borrow(),
        [
            format!("enter {top:?}"),
            format!("leave {top:?}"),
            format!("enter {bottom:?}"),
            format!("leave {bottom:?}"),
        ]
    );
}

#[test]
fn scrolling_goes_to_the_innermost_widget_under_the_pointer_that_takes_it() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let (_, passive) = Probe::boxed(&log, false);
    let (taker, scrolling) = Probe::boxed(&log, true);
    let mut ui = Ui::new(
        Box::new(Stack::new(
            Axis::Vertical,
            0.0,
            0.0,
            vec![scrolling, passive],
        )),
        context(),
    );
    ui.layout(AREA);
    let scroll = Event::Scroll(Scroll {
        position: Point::new(10.0, 10.0),
        delta: Point::new(0.0, 10.0),
        ..Scroll::default()
    });
    assert!(ui.handle(&scroll));
    log.borrow_mut().retain(|entry| entry.starts_with("scroll"));
    assert_eq!(*log.borrow(), [format!("scroll {taker:?}")]);
}

#[test]
fn a_secondary_click_does_not_press_a_button() {
    let clicks = Rc::new(Cell::new(0));
    let counter = clicks.clone();
    let button = Button::new("Apply", move || counter.set(counter.get() + 1));
    let id = button.id();
    let mut ui = Ui::new(Box::new(button), context());
    ui.layout(Size::new(120.0, BUTTON_HEIGHT));
    let at = centre(ui.widget(id).expect("button").frame());
    let secondary = Pointer::at(at).with_button(PointerButton::Secondary);
    ui.handle(&Event::PointerDown(secondary));
    ui.handle(&Event::PointerUp(secondary));
    assert_eq!(clicks.get(), 0);
}

#[test]
fn the_pointer_shows_an_i_beam_over_text_fields_only() {
    let (mut ui, ids) = form(false);
    let field = ui.widget(ids[0]).expect("field").frame();
    let button = ui.widget(ids[1]).expect("button").frame();
    ui.handle(&Event::PointerMove(Pointer::at(centre(field))));
    assert_eq!(ui.cursor(), Cursor::Text);
    ui.handle(&Event::PointerMove(Pointer::at(centre(button))));
    assert_eq!(ui.cursor(), Cursor::Default);
}

#[test]
fn a_double_click_selects_a_word_and_a_triple_click_everything() {
    let mut field = Field::with("hello brave world");
    let frame = field.field.frame();
    let at = Point::new(frame.x + 50.0, centre(frame).y);
    for count in 1..=2 {
        let pointer = Pointer::at(at).with_count(count);
        field
            .field
            .handle(&mut field.cx, &Event::PointerDown(pointer));
        field
            .field
            .handle(&mut field.cx, &Event::PointerUp(pointer));
    }
    assert_eq!(field.selected(), "brave");
    let pointer = Pointer::at(at).with_count(3);
    field
        .field
        .handle(&mut field.cx, &Event::PointerDown(pointer));
    assert_eq!(field.selected(), "hello brave world");
}

#[test]
fn a_disclosure_hides_its_content_from_pointer_keyboard_and_assistive_tech_until_opened() {
    let toggles = Rc::new(RefCell::new(Vec::new()));
    let log = toggles.clone();
    let field = TextField::new("Proxy", "", |_| {});
    let field_id = field.id();
    let disclosure = Disclosure::new("Advanced", false, field, move |open| {
        log.borrow_mut().push(open)
    });
    let id = disclosure.id();
    let mut ui = Ui::new(Box::new(disclosure), context());
    ui.layout(AREA);
    ui.handle(&key(Key::Tab, NONE));
    assert_eq!(ui.focus(), None, "the hidden field takes no focus");
    assert_eq!(node(&ui, id).is_expanded(), Some(false));
    ui.handle(&Event::PointerDown(Pointer::at(Point::new(
        10.0,
        DISCLOSURE_HEIGHT / 2.0,
    ))));
    ui.scene(AREA.width as u32, AREA.height as u32);
    assert_eq!(node(&ui, id).is_expanded(), Some(true));
    ui.handle(&key(Key::Tab, NONE));
    assert_eq!(ui.focus(), Some(field_id));
    assert!(ui.perform(&ActionRequest {
        action: Action::Collapse,
        target_tree: TreeId::ROOT,
        target_node: id.into(),
        data: None,
    }));
    assert_eq!(*toggles.borrow(), [true, false]);
    assert_eq!(ui.focus(), None, "the hidden field gives up focus");
}

#[test]
fn collapsing_a_disclosure_around_the_focus_moves_focus_to_the_disclosure() {
    let field = TextField::new("Proxy", "", |_| {});
    let field_id = field.id();
    let disclosure = Disclosure::new("Advanced", true, field, |_| {});
    let id = disclosure.id();
    let mut cx = context();
    cx.keyboard_navigation = true;
    let mut ui = Ui::new(Box::new(disclosure), cx);
    ui.layout(AREA);
    ui.handle(&key(Key::Tab, NONE));
    ui.handle(&key(Key::Tab, NONE));
    assert_eq!(ui.focus(), Some(field_id));
    ui.perform(&ActionRequest {
        action: Action::Collapse,
        target_tree: TreeId::ROOT,
        target_node: id.into(),
        data: None,
    });
    assert_eq!(ui.focus(), Some(id));
}

#[test]
fn a_free_layout_places_children_where_asked_and_hides_empty_ones_from_the_pointer() {
    let clicks = Rc::new(RefCell::new(Vec::new()));
    let (first, second) = (clicks.clone(), clicks.clone());
    let mut layout = FreeLayout::new();
    let shown = layout.insert(
        Button::new("Shown", move || first.borrow_mut().push("shown")),
        Rect::new(40.0, 30.0, 100.0, BUTTON_HEIGHT),
    );
    let hidden = layout.insert(
        Button::new("Hidden", move || second.borrow_mut().push("hidden")),
        Rect::default(),
    );
    let layout_id = layout.id();
    let mut ui = Ui::new(Box::new(layout), context());
    ui.layout(AREA);
    let frame = ui.widget(shown).expect("shown").frame();
    assert_eq!((frame.x, frame.y, frame.width), (40.0, 30.0, 100.0));
    for at in [centre(frame), Point::new(0.0, 0.0)] {
        ui.handle(&Event::PointerDown(Pointer::at(at)));
        ui.handle(&Event::PointerUp(Pointer::at(at)));
    }
    assert_eq!(*clicks.borrow(), ["shown"]);
    let layout = ui.widget_as::<FreeLayout>(layout_id).expect("layout");
    layout.set_frame(hidden, Rect::new(0.0, 0.0, 30.0, BUTTON_HEIGHT));
    assert!(layout.remove(shown).is_some());
    ui.layout(AREA);
    ui.handle(&Event::PointerDown(Pointer::at(Point::new(5.0, 5.0))));
    ui.handle(&Event::PointerUp(Pointer::at(Point::new(5.0, 5.0))));
    assert_eq!(*clicks.borrow(), ["shown", "hidden"]);
}
