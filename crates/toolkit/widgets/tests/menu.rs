#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use unbound_graphics::{Key, Modifiers, Palette, Point, Scheme, TextSystem};
use unbound_widgets::{Menu, MenuItem, MenuOutcome, MenuPanel, MenuTracker};

fn app_menus() -> Menu {
    Menu::new(vec![
        MenuItem::submenu(
            "Files",
            Menu::new(vec![
                MenuItem::new("About Files", "about"),
                MenuItem::new("Quit Files", "quit").shortcut("Super+Q"),
            ]),
        ),
        MenuItem::submenu(
            "Edit",
            Menu::new(vec![
                MenuItem::new("Copy", "copy").shortcut("Super+C"),
                MenuItem::new("Find", "find").shortcut("Super+F"),
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
    .searchable("Search menus", "No Results")
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
        Some("Super+F")
    );
    assert!(tracker.panels()[0].size().height >= size.height);
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
