#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, NodeId, Role, TreeId};
use unbound_graphics::{Color, Item, Paint, Palette, Point, Scheme, TextSystem};
use unbound_widgets::{
    Cell, ClipboardRequest, Column, Context, Event, HEADER_HEIGHT, Key, Menu, MenuItem, Modifiers,
    Pointer, PointerButton, ROW_HEIGHT, Row, Size, Sort, Table, Ui, Widget, WidgetId,
};

const AREA: Size = Size::new(400.0, HEADER_HEIGHT + 10.0 * ROW_HEIGHT);
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
const LOGO: Modifiers = Modifiers { logo: true, ..NONE };
const ALT: Modifiers = Modifiers { alt: true, ..NONE };

#[derive(Default)]
struct Log {
    selections: Vec<Vec<String>>,
    activated: Vec<Vec<String>>,
    sorts: Vec<Sort>,
    actions: Vec<(String, Vec<String>)>,
}

struct Fixture {
    ui: Ui,
    table: WidgetId,
    log: Rc<RefCell<Log>>,
}

fn rows(names: &[&str]) -> Vec<Row> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            Row::new(
                *name,
                vec![Cell::new(*name), Cell::new(format!("{}", index * 100))],
            )
        })
        .collect()
}

fn keys(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

impl Fixture {
    fn new(names: &[&str]) -> Self {
        Self::with(names, |table| table)
    }

    fn with(names: &[&str], configure: impl FnOnce(Table) -> Table) -> Self {
        let log = Rc::new(RefCell::new(Log::default()));
        let (selection, activate, sort, action) =
            (log.clone(), log.clone(), log.clone(), log.clone());
        let table = Table::new(vec![
            Column::new("Name", 200.0),
            Column::new("Size", 100.0).numeric(),
        ])
        .on_selection(move |keys| selection.borrow_mut().selections.push(keys.to_vec()))
        .on_activate(move |keys| activate.borrow_mut().activated.push(keys.to_vec()))
        .on_sort(move |order| sort.borrow_mut().sorts.push(order))
        .context_menu(
            |keys| (!keys.is_empty()).then(|| Menu::new(vec![MenuItem::new("Open", "open")])),
            move |name, keys| {
                action
                    .borrow_mut()
                    .actions
                    .push((name.to_owned(), keys.to_vec()));
            },
        );
        let mut table = configure(table);
        table.set_rows(rows(names));
        let id = table.id();
        let mut ui = Ui::new(
            Box::new(table),
            Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
        );
        ui.scene(AREA.width as u32, AREA.height as u32);
        Self { ui, table: id, log }
    }

    fn table(&mut self) -> &mut Table {
        self.ui.widget_as::<Table>(self.table).expect("table")
    }

    fn row_point(index: usize) -> Point {
        Point::new(
            40.0,
            HEADER_HEIGHT + index as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0,
        )
    }

    fn click(&mut self, index: usize, modifiers: Modifiers, count: u8) {
        let pointer = Pointer::at(Self::row_point(index))
            .with_modifiers(modifiers)
            .with_count(count);
        self.ui.handle(&Event::PointerDown(pointer));
        self.ui.handle(&Event::PointerUp(pointer));
        self.ui.scene(AREA.width as u32, AREA.height as u32);
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.ui.handle(&Event::Key { key, modifiers });
        self.ui.scene(AREA.width as u32, AREA.height as u32);
    }

    fn selection(&mut self) -> Vec<String> {
        self.table().selection()
    }
}

const NAMES: [&str; 5] = ["alpha", "bravo", "charlie", "delta", "echo"];

#[test]
fn clicks_select_and_shift_and_super_extend_like_a_finder_list() {
    let mut fixture = Fixture::new(&NAMES);
    fixture.click(1, NONE, 1);
    assert_eq!(fixture.selection(), keys(&["bravo"]));
    fixture.click(3, SHIFT, 1);
    assert_eq!(fixture.selection(), keys(&["bravo", "charlie", "delta"]));
    fixture.click(2, LOGO, 1);
    assert_eq!(fixture.selection(), keys(&["bravo", "delta"]));
    fixture.click(0, NONE, 1);
    assert_eq!(fixture.selection(), keys(&["alpha"]));
    let clicked_below = Point::new(40.0, AREA.height - 2.0);
    fixture
        .ui
        .handle(&Event::PointerDown(Pointer::at(clicked_below)));
    assert!(
        fixture.selection().is_empty(),
        "a click on empty space clears"
    );
    assert_eq!(fixture.log.borrow().selections.last(), Some(&Vec::new()));
}

#[test]
fn arrows_move_and_extend_the_selection_and_return_opens_it() {
    let mut fixture = Fixture::new(&NAMES);
    fixture.click(0, NONE, 1);
    fixture.key(Key::Down, NONE);
    fixture.key(Key::Down, SHIFT);
    assert_eq!(fixture.selection(), keys(&["bravo", "charlie"]));
    fixture.key(Key::Up, NONE);
    assert_eq!(fixture.selection(), keys(&["bravo"]));
    fixture.key(Key::Enter, NONE);
    fixture.key(Key::Character('a'), LOGO);
    assert_eq!(fixture.selection(), keys(&NAMES));
    assert_eq!(fixture.log.borrow().activated, [keys(&["bravo"])]);
}

#[test]
fn a_double_click_opens_the_row() {
    let mut fixture = Fixture::new(&NAMES);
    fixture.click(2, NONE, 1);
    fixture.click(2, NONE, 2);
    assert_eq!(fixture.log.borrow().activated, [keys(&["charlie"])]);
}

#[test]
fn typing_jumps_to_the_first_matching_name() {
    let mut fixture = Fixture::new(&NAMES);
    fixture.key(Key::Tab, NONE);
    fixture.key(Key::Character('d'), NONE);
    assert_eq!(fixture.selection(), keys(&["delta"]));
    fixture.key(Key::Character('C'), NONE);
    assert_eq!(fixture.selection(), keys(&["delta"]), "dc matches nothing");
}

#[test]
fn moving_past_the_bottom_scrolls_the_cursor_into_view() {
    let names = (0..50)
        .map(|index| format!("row {index:02}"))
        .collect::<Vec<_>>();
    let names = names.iter().map(String::as_str).collect::<Vec<_>>();
    let mut fixture = Fixture::new(&names);
    fixture.key(Key::Tab, NONE);
    fixture.key(Key::End, NONE);
    assert_eq!(fixture.selection(), keys(&["row 49"]));
    for _ in 0..60 {
        fixture.ui.scene(AREA.width as u32, AREA.height as u32);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let tree = fixture.ui.accessibility();
    let focus = tree
        .nodes
        .iter()
        .find(|(id, _)| *id == tree.focus)
        .expect("the focused row is in the tree");
    assert_eq!(focus.1.role(), Role::Row);
    assert_eq!(focus.1.row_index(), Some(49));
    let bounds = focus.1.bounds().expect("bounds");
    assert!(
        bounds.y1 <= f64::from(AREA.height) + 0.5,
        "the row is visible: {bounds:?}"
    );
}

#[test]
fn new_rows_keep_the_selection_by_key() {
    let mut fixture = Fixture::new(&NAMES);
    fixture.click(1, NONE, 1);
    fixture
        .table()
        .set_rows(rows(&["echo", "delta", "charlie", "bravo", "alpha"]));
    assert_eq!(fixture.selection(), keys(&["bravo"]));
    fixture.key(Key::Down, NONE);
    assert_eq!(fixture.selection(), keys(&["alpha"]));
}

#[test]
fn header_clicks_sort_and_a_second_click_reverses() {
    let mut fixture = Fixture::new(&NAMES);
    let name = Point::new(40.0, HEADER_HEIGHT / 2.0);
    let size = Point::new(300.0, HEADER_HEIGHT / 2.0);
    for at in [name, name, size] {
        fixture.ui.handle(&Event::PointerDown(Pointer::at(at)));
        fixture.ui.handle(&Event::PointerUp(Pointer::at(at)));
    }
    assert_eq!(
        fixture.log.borrow().sorts,
        [
            Sort {
                column: 0,
                ascending: true
            },
            Sort {
                column: 0,
                ascending: false
            },
            Sort {
                column: 1,
                ascending: false
            },
        ],
        "numbers start with the largest first"
    );
}

#[test]
fn a_right_click_on_an_unselected_row_selects_it_for_the_menu() {
    let mut fixture = Fixture::new(&NAMES);
    fixture.click(0, NONE, 1);
    let secondary = Pointer {
        button: PointerButton::Secondary,
        ..Pointer::at(Fixture::row_point(3))
    };
    fixture.ui.handle(&Event::PointerDown(secondary));
    assert!(fixture.ui.take_menu_request().is_some());
    assert_eq!(fixture.selection(), keys(&["delta"]));
    fixture.ui.menu_action("open");
    assert_eq!(
        fixture.log.borrow().actions,
        [("open".to_owned(), keys(&["delta"]))]
    );
}

#[test]
fn rows_and_headers_are_exposed_to_assistive_technology() {
    let mut fixture = Fixture::new(&NAMES);
    let tree = fixture.ui.accessibility();
    let table = tree
        .nodes
        .iter()
        .find(|(id, _)| *id == NodeId::from(fixture.table))
        .expect("table node");
    assert_eq!(table.1.role(), Role::Table);
    assert_eq!(table.1.row_count(), Some(5));
    let headers = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::ColumnHeader)
        .map(|(_, node)| node.label().expect("title").to_owned())
        .collect::<Vec<_>>();
    assert_eq!(headers, ["Name", "Size"]);
    let charlie = tree
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Row && node.label() == Some("charlie"))
        .expect("row node")
        .0;
    assert!(fixture.ui.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: charlie,
        data: None,
    }));
    assert_eq!(fixture.selection(), keys(&["charlie"]));
    assert_eq!(fixture.ui.accessibility().focus, charlie);
}

#[test]
fn dragging_a_header_divider_resizes_the_column() {
    let mut fixture = Fixture::new(&NAMES);
    let divider = Point::new(200.0, HEADER_HEIGHT / 2.0);
    fixture.ui.handle(&Event::PointerDown(Pointer::at(divider)));
    fixture
        .ui
        .handle(&Event::PointerMove(Pointer::at(Point::new(260.0, 5.0))));
    fixture
        .ui
        .handle(&Event::PointerUp(Pointer::at(Point::new(260.0, 5.0))));
    fixture.ui.scene(AREA.width as u32, AREA.height as u32);
    assert!(
        fixture.log.borrow().sorts.is_empty(),
        "resizing does not sort"
    );
    fixture
        .ui
        .handle(&Event::PointerDown(Pointer::at(Point::new(230.0, 5.0))));
    assert_eq!(
        fixture.log.borrow().sorts,
        [Sort {
            column: 0,
            ascending: true
        }],
        "the name column now reaches past 200"
    );
}

#[test]
fn super_c_copies_the_selected_rows_as_tab_separated_lines() {
    let mut fixture = Fixture::new(&NAMES);
    fixture.click(1, NONE, 1);
    fixture.click(2, SHIFT, 1);
    fixture.key(Key::Character('c'), LOGO);
    assert_eq!(
        fixture.ui.take_clipboard_request(),
        Some(ClipboardRequest::Copy(
            "bravo\t100\ncharlie\t200".to_owned()
        ))
    );
}

#[test]
fn pressing_a_selected_row_keeps_the_group_until_release() {
    let mut fixture = Fixture::new(&NAMES);
    fixture.click(1, NONE, 1);
    fixture.click(3, SHIFT, 1);
    let pointer = Pointer::at(Fixture::row_point(2));
    fixture.ui.handle(&Event::PointerDown(pointer));
    assert_eq!(fixture.selection(), keys(&["bravo", "charlie", "delta"]));
    fixture.ui.handle(&Event::PointerUp(pointer));
    assert_eq!(fixture.selection(), keys(&["charlie"]));
}

#[test]
fn dragging_from_a_selected_row_hands_the_group_to_the_drag_hook() {
    let started = Rc::new(RefCell::new(Vec::new()));
    let log = started.clone();
    let mut fixture = Fixture::with(&NAMES, |table| {
        table.on_drag(move |keys, at| log.borrow_mut().push((keys.to_vec(), at)))
    });
    fixture.click(0, NONE, 1);
    fixture.click(1, SHIFT, 1);
    let from = Fixture::row_point(1);
    fixture.ui.handle(&Event::PointerDown(Pointer::at(from)));
    fixture
        .ui
        .handle(&Event::PointerMove(Pointer::at(Point::new(
            from.x + 20.0,
            from.y + 30.0,
        ))));
    assert_eq!(*started.borrow(), [(keys(&["alpha", "bravo"]), from)]);
    assert_eq!(fixture.selection(), keys(&["alpha", "bravo"]));
}

#[test]
fn rows_report_where_they_are_after_scrolling() {
    let names = (0..50)
        .map(|index| format!("row {index:02}"))
        .collect::<Vec<_>>();
    let names = names.iter().map(String::as_str).collect::<Vec<_>>();
    let mut fixture = Fixture::new(&names);
    fixture.table().scroll_to_row(30);
    for _ in 0..60 {
        fixture.ui.scene(AREA.width as u32, AREA.height as u32);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let table = fixture.table();
    assert!(table.scroll_offset() > 0.0);
    let bounds = table.row_bounds(30).expect("row 30");
    assert!(bounds.y >= HEADER_HEIGHT && bounds.y + bounds.height <= AREA.height + 0.5);
    let middle = Point::new(10.0, bounds.y + bounds.height / 2.0);
    assert_eq!(table.row_at(middle), Some(30));
    assert_eq!(table.cell_bounds(30, 1).expect("size cell").x, 200.0);
}

fn tree() -> Vec<Row> {
    let leaf = |name: &str| Row::new(name, vec![Cell::new(name), Cell::new("1")]);
    vec![
        Row::new("mail", vec![Cell::new("mail"), Cell::new("3")]).children(vec![
            leaf("mail helper"),
            Row::new("mail sync", vec![Cell::new("mail sync"), Cell::new("1")])
                .children(vec![leaf("mail sync worker")]),
        ]),
        leaf("notes"),
    ]
}

fn outline() -> Fixture {
    let mut fixture = Fixture::new(&[]);
    fixture.table().set_rows(tree());
    fixture.ui.scene(AREA.width as u32, AREA.height as u32);
    fixture
}

#[test]
fn right_and_left_open_and_close_branches_and_walk_the_outline() {
    let mut fixture = outline();
    fixture.click(0, NONE, 1);
    fixture.key(Key::Down, NONE);
    assert_eq!(
        fixture.selection(),
        keys(&["notes"]),
        "children start hidden"
    );
    fixture.key(Key::Up, NONE);
    fixture.key(Key::Right, NONE);
    fixture.key(Key::Right, NONE);
    assert_eq!(fixture.selection(), keys(&["mail helper"]));
    fixture.key(Key::Down, NONE);
    fixture.key(Key::Right, NONE);
    fixture.key(Key::Down, NONE);
    assert_eq!(fixture.selection(), keys(&["mail sync worker"]));
    fixture.key(Key::Left, NONE);
    assert_eq!(fixture.selection(), keys(&["mail sync"]));
    fixture.key(Key::Left, NONE);
    fixture.key(Key::Down, NONE);
    assert_eq!(fixture.selection(), keys(&["notes"]), "mail sync closed");
}

#[test]
fn closing_a_branch_moves_a_hidden_selection_to_the_branch() {
    let mut fixture = outline();
    fixture.click(0, NONE, 1);
    fixture.key(Key::Right, ALT);
    fixture.key(Key::End, NONE);
    fixture.key(Key::Up, SHIFT);
    assert_eq!(fixture.selection(), keys(&["mail sync worker", "notes"]));
    let mail = Fixture::row_point(0);
    let triangle = Point::new(12.0, mail.y);
    fixture
        .ui
        .handle(&Event::PointerDown(Pointer::at(triangle)));
    fixture.ui.handle(&Event::PointerUp(Pointer::at(triangle)));
    assert_eq!(fixture.selection(), keys(&["mail", "notes"]));
    assert_eq!(
        fixture.log.borrow().selections.last(),
        Some(&keys(&["mail", "notes"]))
    );
}

#[test]
fn an_outline_is_a_tree_grid_with_levels_and_expandable_rows() {
    let mut fixture = outline();
    let tree = fixture.ui.accessibility();
    let table = tree
        .nodes
        .iter()
        .find(|(id, _)| *id == NodeId::from(fixture.table))
        .expect("table node");
    assert_eq!(table.1.role(), Role::TreeGrid);
    let mail = tree
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Row && node.label() == Some("mail"))
        .expect("mail row");
    assert_eq!(mail.1.level(), Some(1));
    assert_eq!(mail.1.is_expanded(), Some(false));
    assert!(fixture.ui.perform(&ActionRequest {
        action: Action::Expand,
        target_tree: TreeId::ROOT,
        target_node: mail.0,
        data: None,
    }));
    let helper = fixture
        .ui
        .accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| node.role() == Role::Row && node.label() == Some("mail helper"))
        .expect("child row");
    assert_eq!(helper.1.level(), Some(2));
    assert!(fixture.selection().is_empty(), "expanding selects nothing");
}

#[test]
fn code_opens_and_closes_branches_and_a_hidden_selection_moves_to_its_branch() {
    let mut fixture = outline();
    assert!(fixture.table().set_row_expanded("mail", true));
    assert!(!fixture.table().set_row_expanded("mail", true));
    assert!(!fixture.table().set_row_expanded("notes", true));
    fixture.ui.scene(AREA.width as u32, AREA.height as u32);
    fixture.click(1, NONE, 1);
    assert_eq!(fixture.selection(), keys(&["mail helper"]));
    assert!(fixture.table().is_row_expanded("mail"));
    assert!(fixture.table().set_row_expanded("mail", false));
    assert_eq!(fixture.selection(), keys(&["mail"]));
    assert_eq!(
        fixture.log.borrow().selections.last(),
        Some(&keys(&["mail"]))
    );
}

#[test]
fn a_flexible_column_takes_the_spare_width_and_its_divider_follows_the_pointer() {
    let mut fixture = Fixture::new(&NAMES);
    fixture.table().set_columns(vec![
        Column::new("Name", 120.0).flexible(),
        Column::new("Size", 100.0).numeric(),
        Column::new("Kind", 80.0),
    ]);
    fixture.table().set_rows(rows(&NAMES));
    fixture.ui.scene(AREA.width as u32, AREA.height as u32);
    let size = fixture.table().cell_bounds(0, 1).expect("size cell");
    assert_eq!((size.x, size.width), (220.0, 100.0));
    let divider = Point::new(220.0, HEADER_HEIGHT / 2.0);
    let moved = Point::new(180.0, HEADER_HEIGHT / 2.0);
    fixture.ui.handle(&Event::PointerDown(Pointer::at(divider)));
    fixture.ui.handle(&Event::PointerMove(Pointer::at(moved)));
    fixture.ui.handle(&Event::PointerUp(Pointer::at(moved)));
    fixture.ui.scene(AREA.width as u32, AREA.height as u32);
    let size = fixture.table().cell_bounds(0, 1).expect("size cell");
    assert_eq!((size.x, size.width), (180.0, 140.0));
    let kind = fixture.table().cell_bounds(0, 2).expect("kind cell");
    assert_eq!((kind.x, kind.width), (320.0, 80.0));
}

fn selection_top(items: &[Item], accent: Color) -> Option<f32> {
    items.iter().find_map(|item| match item {
        Item::Fill {
            shape,
            paint: Paint::Solid(color),
        } if *color == accent => Some(shape.rect.y),
        Item::Clip { items, .. } => selection_top(items, accent),
        _ => None,
    })
}

#[test]
fn the_selection_glides_to_the_next_row_instead_of_jumping() {
    let mut fixture = Fixture::new(&NAMES);
    let clock = Rc::new(std::cell::Cell::new(Instant::now()));
    let time = clock.clone();
    fixture.ui.set_clock(move || time.get());
    fixture.click(0, NONE, 1);
    let accent = Palette::for_scheme(Scheme::Light).accent;
    let top = |fixture: &mut Fixture| {
        let scene = fixture.ui.scene(AREA.width as u32, AREA.height as u32);
        selection_top(&scene.items, accent).expect("a focused selection")
    };
    let start = top(&mut fixture);
    fixture.key(Key::Down, NONE);
    clock.set(clock.get() + Duration::from_millis(40));
    let gliding = top(&mut fixture);
    assert!(fixture.ui.is_animating());
    assert!(
        start < gliding && gliding < start + ROW_HEIGHT,
        "the block is on its way: {start} < {gliding} < {}",
        start + ROW_HEIGHT
    );
    clock.set(clock.get() + Duration::from_secs(1));
    assert!((top(&mut fixture) - (start + ROW_HEIGHT)).abs() < 0.01);
}
