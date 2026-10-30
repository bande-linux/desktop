#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use accesskit::{
    Action, ActionData, ActionRequest, NodeId, Role, TextPosition, TextSelection, TreeId,
};
use unbound_graphics::{Item, Palette, Point, Rect, Scheme, TextSystem};
use unbound_widgets::{
    ClipboardRequest, Context, Event, Key, Modifiers, Pointer, PointerButton, SearchField, Size,
    TextField, TextView, Ui, Widget, WidgetId,
};

const NONE: Modifiers = Modifiers {
    shift: false,
    alt: false,
    control: false,
    logo: false,
};
const LOGO: Modifiers = Modifiers { logo: true, ..NONE };
const SHIFT_LOGO: Modifiers = Modifiers {
    shift: true,
    logo: true,
    ..NONE
};
const AREA: Size = Size::new(320.0, 240.0);

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, modifiers }
}

struct Editing {
    ui: Ui,
    id: WidgetId,
    changes: Rc<RefCell<Vec<String>>>,
}

impl Editing {
    fn field(text: &str) -> Self {
        Self::with(|log| {
            Box::new(TextField::new("Name", text, move |text| {
                log.borrow_mut().push(text.to_owned())
            }))
        })
    }

    fn view(text: &str) -> Self {
        Self::with(|log| {
            Box::new(TextView::new("Notes", text, move |text| {
                log.borrow_mut().push(text.to_owned())
            }))
        })
    }

    fn with(make: impl FnOnce(Rc<RefCell<Vec<String>>>) -> Box<dyn Widget>) -> Self {
        let changes = Rc::new(RefCell::new(Vec::new()));
        let widget = make(changes.clone());
        let id = widget.id();
        let mut ui = Ui::new(
            widget,
            Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
        );
        ui.layout(AREA);
        ui.perform(&request(id.into(), Action::Focus, None));
        Self { ui, id, changes }
    }

    fn press(&mut self, pressed: Key, modifiers: Modifiers) -> bool {
        let handled = self.ui.handle(&key(pressed, modifiers));
        self.ui.layout(AREA);
        handled
    }

    fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            match character {
                ' ' => self.press(Key::Space, NONE),
                '\n' => self.press(Key::Enter, NONE),
                character => self.press(Key::Character(character), NONE),
            };
        }
    }

    fn send(&mut self, event: Event) -> bool {
        let handled = self.ui.handle(&event);
        self.ui.layout(AREA);
        handled
    }

    fn text(&mut self) -> String {
        if let Some(search) = self.ui.widget_as::<SearchField>(self.id) {
            return search.text();
        }
        match self.ui.widget_as::<TextField>(self.id) {
            Some(field) => field.text(),
            None => self.ui.widget_as::<TextView>(self.id).expect("view").text(),
        }
    }

    fn selection(&mut self) -> std::ops::Range<usize> {
        match self.ui.widget_as::<TextField>(self.id) {
            Some(field) => field.selection(),
            None => self
                .ui
                .widget_as::<TextView>(self.id)
                .expect("view")
                .selection(),
        }
    }

    fn selected(&mut self) -> String {
        let range = self.selection();
        self.text()[range].to_owned()
    }

    fn caret(&mut self) -> Rect {
        self.ui.text_input_area().expect("a focused text widget")
    }

    fn frame(&mut self) -> Rect {
        self.ui.widget(self.id).expect("widget").frame()
    }

    fn items(&mut self) -> Vec<Item> {
        self.ui.scene(AREA.width as u32, AREA.height as u32).items
    }
}

fn request(target_node: NodeId, action: Action, data: Option<ActionData>) -> ActionRequest {
    ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node,
        data,
    }
}

fn fills(items: &[Item], out: &mut Vec<Rect>) {
    for item in items {
        match item {
            Item::Fill { shape, .. } => out.push(shape.rect),
            Item::Clip { items, .. } => fills(items, out),
            _ => {}
        }
    }
}

#[test]
fn undo_takes_back_typing_a_word_at_a_time_and_redo_brings_it_back() {
    let mut editing = Editing::field("");
    editing.type_text("hello brave world");
    editing.press(Key::Character('z'), LOGO);
    assert_eq!(editing.text(), "hello brave ");
    editing.press(Key::Character('z'), LOGO);
    assert_eq!(editing.text(), "hello ");
    editing.press(Key::Character('z'), SHIFT_LOGO);
    assert_eq!(editing.text(), "hello brave ");
    editing.press(Key::Character('z'), LOGO);
    editing.press(Key::Character('z'), LOGO);
    assert_eq!(editing.text(), "");
    editing.press(Key::Character('z'), SHIFT_LOGO);
    editing.press(Key::Character('z'), SHIFT_LOGO);
    editing.press(Key::Character('z'), SHIFT_LOGO);
    assert_eq!(editing.text(), "hello brave world");
    assert_eq!(
        editing.changes.borrow().last().map(String::as_str),
        Some("hello brave world")
    );
}

#[test]
fn moving_the_caret_ends_an_undo_step_and_new_typing_clears_redo() {
    let mut editing = Editing::field("");
    editing.type_text("ab");
    editing.press(Key::Left, NONE);
    editing.type_text("X");
    assert_eq!(editing.text(), "aXb");
    editing.press(Key::Character('z'), LOGO);
    assert_eq!(editing.text(), "ab");
    assert_eq!(editing.selection(), 1..1, "undo puts the caret back");
    editing.type_text("Y");
    editing.press(Key::Character('z'), SHIFT_LOGO);
    assert_eq!(
        editing.text(),
        "aYb",
        "typing after an undo forgets the redo"
    );
}

#[test]
fn deleting_and_pasting_undo_as_whole_steps_restoring_the_selection() {
    let mut editing = Editing::field("Quarterly report draft");
    for _ in 0..6 {
        editing.press(Key::Backspace, NONE);
    }
    assert_eq!(editing.text(), "Quarterly report");
    editing.press(Key::Character('z'), LOGO);
    assert_eq!(editing.text(), "Quarterly report draft");

    editing.press(Key::Character('a'), LOGO);
    editing.press(Key::Character('v'), LOGO);
    assert_eq!(
        editing.ui.take_clipboard_request(),
        Some(ClipboardRequest::Paste)
    );
    assert!(editing.ui.pasted("Annual\nsummary"));
    editing.ui.layout(AREA);
    assert_eq!(
        editing.text(),
        "Annual summary",
        "a field keeps to one line"
    );
    editing.press(Key::Character('z'), LOGO);
    assert_eq!(editing.text(), "Quarterly report draft");
    assert_eq!(editing.selected(), "Quarterly report draft");
}

#[test]
fn the_context_menu_offers_editing_commands_that_reflect_what_is_possible() {
    let mut editing = Editing::field("Invoice");
    let at = editing.caret();
    let secondary =
        Pointer::at(Point::new(at.x - 10.0, at.y + 4.0)).with_button(PointerButton::Secondary);
    editing.send(Event::PointerDown(secondary));
    let (menu, _) = editing.ui.take_menu_request().expect("a text menu");
    let state: Vec<(String, bool)> = menu
        .items
        .iter()
        .filter(|item| !item.separator)
        .map(|item| (item.action.clone(), item.enabled))
        .collect();
    assert_eq!(
        state,
        [
            ("undo".to_owned(), false),
            ("redo".to_owned(), false),
            ("cut".to_owned(), false),
            ("copy".to_owned(), false),
            ("paste".to_owned(), true),
            ("select-all".to_owned(), true),
            ("emoji-and-symbols".to_owned(), true),
        ]
    );
    assert!(editing.ui.menu_action("select-all"));
    assert_eq!(editing.selected(), "Invoice");

    editing.send(Event::PointerDown(secondary));
    let (menu, _) = editing.ui.take_menu_request().expect("a text menu");
    assert!(
        menu.items
            .iter()
            .any(|item| item.action == "cut" && item.enabled)
    );
    assert!(editing.ui.menu_action("cut"));
    assert_eq!(
        editing.ui.take_clipboard_request(),
        Some(ClipboardRequest::Copy("Invoice".to_owned()))
    );
    assert_eq!(editing.text(), "");
    editing.send(Event::PointerDown(secondary));
    editing.ui.take_menu_request().expect("a text menu");
    assert!(!editing.ui.take_emoji_picker_request());
    assert!(editing.ui.menu_action("emoji-and-symbols"));
    assert!(editing.ui.take_emoji_picker_request());
    editing.send(Event::PointerDown(secondary));
    editing.ui.take_menu_request().expect("a text menu");
    assert!(editing.ui.menu_action("undo"));
    assert_eq!(editing.text(), "Invoice");
}

#[test]
fn composition_shows_underlined_in_place_without_changing_the_value_until_committed() {
    let mut editing = Editing::field("Tokyo ");
    let before = editing.caret();
    let underlines = |editing: &mut Editing| {
        let mut rects = Vec::new();
        fills(&editing.items(), &mut rects);
        rects
            .into_iter()
            .filter(|rect| rect.height <= 2.0 && rect.width > 4.0)
            .count()
    };
    assert_eq!(underlines(&mut editing), 0);
    assert!(editing.send(Event::Preedit {
        text: "とうきょう".to_owned(),
        cursor: Some((15, 15)),
    }));
    assert_eq!(editing.text(), "Tokyo ");
    assert!(editing.changes.borrow().is_empty());
    assert!(underlines(&mut editing) >= 1, "the reading is underlined");
    let composing = editing.caret();
    assert_eq!(
        composing.x, before.x,
        "the input area starts where composing began"
    );
    assert!(composing.width > 30.0, "the input area spans the reading");

    editing.send(Event::Commit("東京".to_owned()));
    editing.send(Event::Preedit {
        text: String::new(),
        cursor: None,
    });
    assert_eq!(editing.text(), "Tokyo 東京");
    assert_eq!(underlines(&mut editing), 0);
    editing.press(Key::Character('z'), LOGO);
    assert_eq!(editing.text(), "Tokyo ");
}

#[test]
fn clicking_while_composing_keeps_the_reading_and_resets_the_input_method() {
    let mut editing = Editing::field("");
    editing.send(Event::Preedit {
        text: "ni".to_owned(),
        cursor: Some((2, 2)),
    });
    let frame = editing.frame();
    editing.send(Event::PointerDown(Pointer::at(Point::new(
        frame.x + 2.0,
        frame.y + 8.0,
    ))));
    assert_eq!(editing.text(), "ni");
    assert!(editing.ui.take_input_method_reset());
    assert!(!editing.ui.take_input_method_reset());
}

#[test]
fn arrows_move_visually_through_mixed_left_to_right_and_right_to_left_text() {
    let mut editing = Editing::field("abc אבג def");
    editing.press(Key::Left, LOGO);
    let mut positions = vec![editing.caret().x];
    for _ in 0..11 {
        editing.press(Key::Right, NONE);
        positions.push(editing.caret().x);
    }
    assert!(
        positions.windows(2).all(|pair| pair[1] > pair[0]),
        "each Right moves the caret further right: {positions:?}"
    );
    for _ in 0..11 {
        editing.press(Key::Left, NONE);
    }
    assert_eq!(editing.caret().x, positions[0]);
}

#[test]
fn a_right_to_left_field_starts_at_the_right_and_left_moves_forward() {
    let mut editing = Editing::field("שלום עולם");
    let frame = editing.frame();
    editing.press(Key::Up, NONE);
    assert_eq!(editing.selection(), 0..0);
    let start = editing.caret();
    assert!(
        start.x > frame.x + frame.width / 2.0,
        "the text sits at the right edge"
    );
    editing.press(Key::Left, NONE);
    assert!(editing.selection().start > 0);
    assert!(editing.caret().x < start.x);
}

#[test]
fn double_click_drag_extends_by_whole_words() {
    let mut editing = Editing::field("hello brave new world");
    let frame = editing.frame();
    let y = frame.y + frame.height / 2.0;
    editing.press(Key::Up, NONE);
    let start = editing.caret().x;
    editing.press(Key::Right, LOGO);
    let end = editing.caret().x;
    let at = |fraction: f32| Point::new(start + (end - start) * fraction, y);
    editing.send(Event::PointerDown(Pointer::at(at(0.05)).with_count(2)));
    assert_eq!(editing.selected(), "hello");
    editing.send(Event::PointerMove(Pointer::at(at(0.45))));
    assert_eq!(editing.selected(), "hello brave");
    editing.send(Event::PointerUp(Pointer::at(at(0.45))));
    editing.send(Event::PointerDown(Pointer::at(at(0.9)).with_count(3)));
    assert_eq!(editing.selected(), "hello brave new world");
}

#[test]
fn a_long_field_scrolls_to_keep_the_caret_visible_and_drags_scroll_it() {
    let text = "a long line that is wider than the field so it has to scroll sideways to show the caret at the end";
    let mut editing = Editing::field(text);
    let frame = editing.frame();
    editing.press(Key::Right, LOGO);
    let caret = editing.caret();
    assert!(caret.x <= frame.x + frame.width && caret.x >= frame.x);
    editing.press(Key::Left, LOGO);
    let caret = editing.caret();
    assert!(caret.x >= frame.x && caret.x < frame.x + 10.0);

    let y = frame.y + frame.height / 2.0;
    editing.send(Event::PointerDown(Pointer::at(Point::new(
        frame.x + 20.0,
        y,
    ))));
    editing.send(Event::PointerMove(Pointer::at(Point::new(
        frame.x + frame.width + 60.0,
        y,
    ))));
    for _ in 0..60 {
        editing.ui.layout(AREA);
    }
    assert_eq!(
        editing.selection().end,
        text.len(),
        "holding past the edge scrolls to the end"
    );
}

#[test]
fn a_text_field_publishes_its_text_and_caret_to_assistive_technology() {
    let mut editing = Editing::field("Draft");
    editing.press(Key::Left, NONE);
    let tree = editing.ui.accessibility();
    let field = tree
        .nodes
        .iter()
        .find(|(node, _)| *node == NodeId::from(editing.id))
        .map(|(_, node)| node.clone())
        .expect("field node");
    let run_id = field.children()[0];
    let run = tree
        .nodes
        .iter()
        .find(|(node, _)| *node == run_id)
        .map(|(_, node)| node.clone())
        .expect("run node");
    assert_eq!(run.role(), Role::TextRun);
    assert_eq!(run.value(), Some("Draft"));
    let selection = field.text_selection().expect("a caret");
    assert_eq!(selection.focus.node, run_id);
    assert_eq!(selection.focus.character_index, 4);

    let select = TextSelection {
        anchor: TextPosition {
            node: run_id,
            character_index: 0,
        },
        focus: TextPosition {
            node: run_id,
            character_index: 2,
        },
    };
    assert!(editing.ui.perform(&request(
        editing.id.into(),
        Action::SetTextSelection,
        Some(ActionData::SetTextSelection(select)),
    )));
    assert_eq!(editing.selected(), "Dr");
}

#[test]
fn a_text_view_wraps_inserts_new_lines_and_moves_between_lines_keeping_the_column() {
    let mut editing = Editing::view("");
    editing.type_text("first line\nsecond\nthird line here");
    assert_eq!(editing.text(), "first line\nsecond\nthird line here");
    editing.press(Key::Up, LOGO);
    assert_eq!(editing.selection(), 0..0);
    editing.press(Key::Right, LOGO);
    assert_eq!(editing.selection(), 10..10);
    editing.press(Key::Down, NONE);
    assert_eq!(
        editing.selection(),
        17..17,
        "a shorter line puts the caret at its end"
    );
    editing.press(Key::Down, NONE);
    let column = editing.selection().start;
    assert!(
        (26..=29).contains(&column),
        "the next line returns to the remembered column: {column}"
    );
    editing.press(
        Key::Up,
        Modifiers {
            shift: true,
            ..LOGO
        },
    );
    assert_eq!(editing.selected(), editing.text()[..column].to_owned());
    editing.press(Key::Character('z'), LOGO);
    assert_eq!(editing.text(), "first line\nsecond\nthird line ");
}

#[test]
fn a_triple_click_selects_one_paragraph_of_a_text_view() {
    let mut editing = Editing::view("one\ntwo\nthree");
    let first = editing.caret();
    editing.send(Event::PointerDown(
        Pointer::at(Point::new(first.x + 4.0, first.y + first.height * 1.5)).with_count(3),
    ));
    assert_eq!(editing.selected(), "two\n");
}

#[test]
fn a_text_view_scrolls_to_follow_the_caret() {
    let text = (1..=200)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut editing = Editing::view(&text);
    let frame = editing.frame();
    editing.press(Key::Down, LOGO);
    let caret = editing.caret();
    assert!(
        caret.y >= frame.y && caret.y + caret.height <= frame.y + frame.height,
        "the caret at the end is on screen: {caret:?} in {frame:?}"
    );
    editing.press(Key::PageUp, NONE);
    let paged = editing.caret();
    assert!(paged.y >= frame.y, "page up stays on screen");
    assert!(editing.selection().start < text.len() - 100);
}

#[test]
fn a_read_only_text_view_selects_and_copies_but_never_edits() {
    let mut editing =
        Editing::with(|_| Box::new(TextView::new("", "Release notes", |_| {}).read_only()));
    assert!(!editing.press(Key::Character('x'), NONE));
    editing.send(Event::Commit("typed".to_owned()));
    assert_eq!(editing.text(), "Release notes");
    editing.press(Key::Character('a'), LOGO);
    editing.press(Key::Character('c'), LOGO);
    assert_eq!(
        editing.ui.take_clipboard_request(),
        Some(ClipboardRequest::Copy("Release notes".to_owned()))
    );
    let frame = editing.frame();
    editing.send(Event::PointerDown(
        Pointer::at(Point::new(frame.x + 10.0, frame.y + 8.0))
            .with_button(PointerButton::Secondary),
    ));
    let (menu, _) = editing.ui.take_menu_request().expect("menu");
    assert!(menu.items.iter().all(|item| item.action != "paste"));
    let node = editing
        .ui
        .accessibility()
        .nodes
        .into_iter()
        .find(|(node, _)| *node == NodeId::from(editing.id))
        .expect("node")
        .1;
    assert!(node.is_read_only());
    assert_eq!(node.role(), Role::MultilineTextInput);
}

#[test]
fn typing_in_a_hundred_thousand_line_text_view_stays_instant() {
    let text = (0..100_000)
        .map(|line| format!("{line}: the quick brown fox jumps over the lazy dog"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut editing = Editing::view(&text);
    editing.press(Key::Down, LOGO);
    let started = Instant::now();
    for character in "typing at the end".chars() {
        editing.press(Key::Character(character), NONE);
        editing.items();
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed.as_millis() < 17 * 100,
        "17 keystrokes with layout and paint took {elapsed:?}"
    );
    assert!(editing.text().ends_with("dogtyping at the end"));
}

#[test]
fn a_search_field_searches_as_you_type_and_escape_or_the_clear_button_empties_it() {
    let mut editing = Editing::with(|log| {
        Box::new(SearchField::new("Search", "", move |text| {
            log.borrow_mut().push(text.to_owned())
        }))
    });
    editing.type_text("tax");
    assert_eq!(*editing.changes.borrow(), ["t", "ta", "tax"]);
    assert!(editing.press(Key::Escape, NONE));
    assert_eq!(editing.text(), "");
    assert_eq!(
        editing.changes.borrow().last().map(String::as_str),
        Some("")
    );
    assert!(
        !editing.press(Key::Escape, NONE),
        "Escape in an empty search field goes on to the window"
    );
    editing.press(Key::Character('z'), LOGO);
    assert_eq!(editing.text(), "tax", "clearing can be undone");

    let frame = editing.frame();
    let clear = Point::new(frame.x + frame.width - 15.0, frame.y + frame.height / 2.0);
    editing.send(Event::PointerDown(Pointer::at(clear)));
    assert_eq!(editing.text(), "tax", "the clear button acts on release");
    editing.send(Event::PointerUp(Pointer::at(clear)));
    assert_eq!(editing.text(), "");
}

#[test]
fn a_search_field_is_a_search_input_with_a_clear_button_for_assistive_technology() {
    let mut editing = Editing::with(|_| Box::new(SearchField::new("Search", "report", |_| {})));
    let tree = editing.ui.accessibility();
    let node = |id: NodeId| {
        tree.nodes
            .iter()
            .find(|(node, _)| *node == id)
            .map(|(_, node)| node.clone())
            .expect("node")
    };
    let field = node(editing.id.into());
    assert_eq!(field.role(), Role::SearchInput);
    let clear = field
        .children()
        .iter()
        .copied()
        .find(|child| node(*child).role() == Role::Button)
        .expect("a clear button");
    assert_eq!(node(clear).label(), Some("Clear"));
    assert!(editing.ui.perform(&request(clear, Action::Click, None)));
    assert_eq!(editing.text(), "");
}

#[test]
fn tabbing_into_a_field_selects_its_text_so_typing_replaces_it() {
    let first = TextField::new("First", "Ada", |_| {});
    let second = TextField::new("Last", "Lovelace", |_| {});
    let second_id = second.id();
    let mut ui = Ui::new(
        Box::new(unbound_widgets::Stack::new(
            unbound_widgets::Axis::Vertical,
            8.0,
            0.0,
            vec![Box::new(first), Box::new(second)],
        )),
        Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
    );
    ui.layout(AREA);
    ui.handle(&key(Key::Tab, NONE));
    ui.handle(&key(Key::Tab, NONE));
    ui.handle(&key(Key::Character('B'), NONE));
    let field = ui.widget_as::<TextField>(second_id).expect("field");
    assert_eq!(field.text(), "B");
}
