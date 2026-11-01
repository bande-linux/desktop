#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use bande_graphics::{Key, Modifiers, Palette, Point, Scheme, TextSize, TextSystem};
use bande_widgets::{Menu, MenuItem, MenuOutcome, MenuPanel, MenuTracker};

fn app_menus() -> Menu {
    Menu::new(vec![
        MenuItem::submenu(
            "Files",
            Menu::new(vec![
                MenuItem::new("About Files", "about"),
                MenuItem::new("Quit Files", "quit").shortcut("Ctrl+Q"),
            ]),
        ),
        MenuItem::submenu(
            "Edit",
            Menu::new(vec![
                MenuItem::new("Copy", "copy").shortcut("Ctrl+C"),
                MenuItem::new("Find", "find").shortcut("Ctrl+F"),
                MenuItem::new("Find Next", "find-next").enabled(false),
            ]),
        ),
        MenuItem::submenu(
            "View",
            Menu::new(vec![MenuItem::submenu(
                "Sort By",
                Menu::new(vec![MenuItem::new("Find Order", "sort-find")]),
            )]),
        ),
    ])
    .searchable("Type to search", "Shift+Super+/", "No Results")
}

fn tracker(menu: Menu, text: &mut TextSystem) -> MenuTracker {
    MenuTracker::new(
        MenuPanel::new(menu, text, &Palette::for_scheme(Scheme::Light)),
        false,
    )
}

fn typed(tracker: &mut MenuTracker, text: &mut TextSystem, keys: &str, now: Instant) {
    for character in keys.chars() {
        match tracker.key(Key::Character(character), Modifiers::default(), now) {
            MenuOutcome::Search(menu) => tracker.searched(MenuPanel::new(
                menu,
                text,
                &Palette::for_scheme(Scheme::Light),
            )),
            outcome => panic!("typing searched nothing: {outcome:?}"),
        }
    }
}

fn titles(tracker: &MenuTracker) -> Vec<String> {
    tracker.panels()[0]
        .menu()
        .items
        .iter()
        .map(|item| item.title.clone())
        .collect()
}

#[test]
fn typing_in_a_searchable_menu_lists_every_enabled_item_with_its_path_and_enter_runs_it() {
    let mut text = TextSystem::new();
    let mut tracker = tracker(app_menus(), &mut text);
    let now = Instant::now();
    let size = tracker.panels()[0].size();
    typed(&mut tracker, &mut text, "fin", now);

    assert_eq!(
        titles(&tracker),
        ["Edit › Find", "View › Sort By › Find Order"]
    );
    assert_eq!(
        tracker.panels()[0].menu().items[0].shortcut.as_deref(),
        Some("Ctrl+F")
    );
    assert!(tracker.panels()[0].size().width >= size.width);
    assert_eq!(
        tracker.key(Key::Enter, Modifiers::default(), now),
        MenuOutcome::Activate("find".to_owned())
    );
}

#[test]
fn backspace_and_escape_walk_the_search_back_to_the_full_menu() {
    let mut text = TextSystem::new();
    let mut tracker = tracker(app_menus(), &mut text);
    let now = Instant::now();
    typed(&mut tracker, &mut text, "zz", now);
    assert_eq!(titles(&tracker), ["No Results"]);

    let MenuOutcome::Search(narrower) = tracker.key(Key::Backspace, Modifiers::default(), now)
    else {
        panic!("backspace edits the search");
    };
    assert_eq!(
        narrower.search.as_ref().map(|search| search.query.as_str()),
        Some("z")
    );
    tracker.searched(MenuPanel::new(
        narrower,
        &mut text,
        &Palette::for_scheme(Scheme::Light),
    ));
    let MenuOutcome::Search(full) = tracker.key(Key::Escape, Modifiers::default(), now) else {
        panic!("escape clears the search");
    };
    assert_eq!(full.items.len(), 3);
    tracker.searched(MenuPanel::new(
        full,
        &mut text,
        &Palette::for_scheme(Scheme::Light),
    ));
    assert_eq!(
        tracker.key(Key::Escape, Modifiers::default(), now),
        MenuOutcome::Dismiss
    );
}

#[test]
fn moving_diagonally_toward_an_open_submenu_keeps_it_until_the_pointer_rests() {
    let mut text = TextSystem::new();
    let menu = Menu::new(app_menus().items);
    let mut tracker = tracker(menu, &mut text);
    let now = Instant::now();
    let panel = &tracker.panels()[0];
    let files = panel.row_rect(0);
    let edit = panel.row_rect(1);
    let width = panel.size().width;
    let start = Point::new(width - 40.0, files.y + files.height / 2.0);
    tracker.pointer_move(0, start, now);
    assert_eq!(
        tracker.tick(now + Duration::from_millis(250)),
        MenuOutcome::OpenSubmenu { level: 0, index: 0 }
    );
    tracker.opened_submenu(
        MenuPanel::new(
            tracker.submenu(0, 0).expect("the Files menu"),
            &mut text,
            &Palette::for_scheme(Scheme::Light),
        ),
        false,
    );

    let later = now + Duration::from_millis(300);
    let diagonal = Point::new(width - 8.0, edit.y + edit.height / 2.0);
    assert_eq!(
        tracker.pointer_move(0, diagonal, later),
        MenuOutcome::Nothing
    );
    assert_eq!(tracker.panels()[0].highlighted(), Some(0));
    assert_eq!(
        tracker.tick(later + Duration::from_millis(400)),
        MenuOutcome::CloseFrom(1)
    );
    assert_eq!(tracker.panels()[0].highlighted(), Some(1));
}

#[test]
fn moving_straight_down_switches_rows_at_once() {
    let mut text = TextSystem::new();
    let mut tracker = tracker(Menu::new(app_menus().items), &mut text);
    let now = Instant::now();
    let panel = &tracker.panels()[0];
    let files = panel.row_rect(0);
    let edit = panel.row_rect(1);
    let x = panel.size().width / 2.0;
    tracker.pointer_move(0, Point::new(x, files.y + files.height / 2.0), now);
    tracker.tick(now + Duration::from_millis(250));
    tracker.opened_submenu(
        MenuPanel::new(
            tracker.submenu(0, 0).expect("the Files menu"),
            &mut text,
            &Palette::for_scheme(Scheme::Light),
        ),
        false,
    );
    assert_eq!(
        tracker.pointer_move(0, Point::new(x, edit.y + edit.height / 2.0), now),
        MenuOutcome::CloseFrom(1)
    );
}

const SHORTENED: f32 = 300.0;
const LONG_MENU_ROWS: usize = 40;

fn long_menu() -> Menu {
    Menu::new(
        (0..LONG_MENU_ROWS)
            .map(|index| MenuItem::new(&format!("Network {index}"), &format!("join-{index}")))
            .collect(),
    )
}

fn shortened(text: &mut TextSystem) -> MenuTracker {
    let mut tracker = tracker(long_menu(), text);
    tracker.placed(0, SHORTENED, false);
    tracker
}

fn row_centre(tracker: &MenuTracker, index: usize) -> Point {
    let row = tracker.panels()[0].row_rect(index);
    Point::new(row.width / 2.0, row.y + row.height / 2.0)
}

#[test]
fn a_menu_shortened_to_fit_the_screen_scrolls_to_every_item_by_keyboard() {
    let mut text = TextSystem::new();
    let mut tracker = shortened(&mut text);
    let now = Instant::now();
    assert_eq!(tracker.panels()[0].size().height, SHORTENED);
    assert!(tracker.panels()[0].is_scrollable());

    tracker.key(Key::End, Modifiers::default(), now);
    let last = LONG_MENU_ROWS - 1;
    let row = tracker.panels()[0].row_rect(last);
    assert!(row.y + row.height <= SHORTENED, "{row:?}");
    assert_eq!(
        tracker.panels()[0].item_at(row_centre(&tracker, last)),
        Some(last)
    );
    assert_eq!(
        tracker.key(Key::Enter, Modifiers::default(), now),
        MenuOutcome::Activate(format!("join-{last}"))
    );
}

#[test]
fn resting_on_the_bottom_edge_of_a_shortened_menu_scrolls_it_until_the_end() {
    let mut text = TextSystem::new();
    let mut tracker = shortened(&mut text);
    let now = Instant::now();
    let edge = Point::new(tracker.panels()[0].size().width / 2.0, SHORTENED - 5.0);
    assert_eq!(
        tracker.panels()[0].item_at(edge),
        None,
        "the scroll zone is not an item"
    );

    tracker.pointer_move(0, edge, now);
    tracker.tick(now + Duration::from_millis(100));
    assert!(tracker.panels()[0].scroll_offset() > 0.0);
    tracker.tick(now + Duration::from_secs(10));
    let full = tracker.panels()[0].row_rect(LONG_MENU_ROWS - 1).y + 28.0 + 4.0;
    assert_eq!(full, SHORTENED);
    assert_eq!(tracker.deadline(), None);
}

#[test]
fn the_wheel_scrolls_a_shortened_menu_and_the_row_under_the_pointer_follows() {
    let mut text = TextSystem::new();
    let mut tracker = shortened(&mut text);
    let now = Instant::now();
    tracker.pointer_move(0, row_centre(&tracker, 3), now);
    assert_eq!(tracker.panels()[0].highlighted(), Some(3));

    assert_eq!(tracker.scroll(0, 56.0), MenuOutcome::Redraw);
    assert_eq!(tracker.panels()[0].highlighted(), Some(5));
    assert_eq!(tracker.scroll(0, -1000.0), MenuOutcome::Redraw);
    assert_eq!(tracker.scroll(0, -10.0), MenuOutcome::Nothing);
    assert_eq!(tracker.panels()[0].highlighted(), Some(3));
}

#[test]
fn a_menu_that_fits_does_not_scroll() {
    let mut text = TextSystem::new();
    let mut tracker = tracker(Menu::new(app_menus().items), &mut text);
    let height = tracker.panels()[0].size().height;
    tracker.placed(0, height, false);
    assert!(!tracker.panels()[0].is_scrollable());
    assert_eq!(tracker.scroll(0, 40.0), MenuOutcome::Nothing);
}

fn rows_left_on_a_tiny_screen(text_size: TextSize) -> usize {
    let mut text = TextSystem::new();
    text.set_size(text_size);
    let mut tracker = tracker(long_menu(), &mut text);
    tracker.placed(0, 20.0, false);
    (0..LONG_MENU_ROWS)
        .filter(|index| {
            let row = tracker.panels()[0].row_rect(*index);
            [row.y + 1.0, row.y + row.height - 1.0]
                .into_iter()
                .all(|y| tracker.panels()[0].item_at(Point::new(40.0, y)) == Some(*index))
        })
        .count()
}

#[test]
fn a_menu_is_never_shortened_below_three_whole_rows_at_any_text_size() {
    for percent in [100, 150, 200] {
        let visible = rows_left_on_a_tiny_screen(TextSize::from_percent(percent));
        assert!(visible >= 3, "{visible} whole rows remain at {percent} %");
    }
}

#[test]
fn larger_text_makes_menu_rows_taller_and_the_menu_wider() {
    let panel = |percent: u32| {
        let mut text = TextSystem::new();
        text.set_size(TextSize::from_percent(percent));
        MenuPanel::new(long_menu(), &mut text, &Palette::for_scheme(Scheme::Light))
    };
    let (regular, larger) = (panel(100), panel(150));
    assert_eq!(regular.row_rect(0).height, 28.0);
    assert_eq!(larger.row_rect(0).height, 37.0);
    assert!(larger.size().width > regular.size().width);
}

#[test]
fn moving_diagonally_toward_a_submenu_that_opened_on_the_left_keeps_it() {
    let mut text = TextSystem::new();
    let mut tracker = tracker(Menu::new(app_menus().items), &mut text);
    let now = Instant::now();
    let files = tracker.panels()[0].row_rect(0);
    let edit = tracker.panels()[0].row_rect(1);
    tracker.pointer_move(0, Point::new(40.0, files.y + files.height / 2.0), now);
    tracker.tick(now + Duration::from_millis(250));
    tracker.opened_submenu(
        MenuPanel::new(
            tracker.submenu(0, 0).expect("the Files menu"),
            &mut text,
            &Palette::for_scheme(Scheme::Light),
        ),
        false,
    );
    tracker.placed(1, 200.0, true);

    let later = now + Duration::from_millis(300);
    assert_eq!(
        tracker.pointer_move(0, Point::new(8.0, edit.y + edit.height / 2.0), later),
        MenuOutcome::Nothing
    );
    assert_eq!(tracker.panels()[0].highlighted(), Some(0));
}
