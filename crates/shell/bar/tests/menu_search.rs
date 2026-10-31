#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bande_bar::menu_search::MenuSearch;
use bande_bar::menus::{AppMenus, cascade};
use bande_status::Command;
use bande_status::dbusmenu::Node;
use bande_toolkit::ui::{Event, Key, Menu, Modifiers};
use bande_toolkit::{Appearance, CpuRenderer, TextSystem};

use support::check_golden;

const SIZE: (u32, u32) = (800, 600);

fn node(id: i32, label: &str, children: Vec<Node>) -> Node {
    Node {
        id,
        label: label.to_owned(),
        enabled: true,
        visible: true,
        children,
        ..Node::default()
    }
}

fn shortcut(node: Node, keys: &str) -> Node {
    Node {
        shortcut: Some(keys.to_owned()),
        ..node
    }
}

fn menus() -> Menu {
    let root = node(
        0,
        "",
        vec![
            node(1, "Files", vec![node(10, "About Files", Vec::new())]),
            node(
                2,
                "File",
                vec![
                    shortcut(node(20, "New Window", Vec::new()), "Super+N"),
                    node(21, "Find in Folder", Vec::new()),
                ],
            ),
            node(
                3,
                "Edit",
                vec![
                    shortcut(node(30, "Copy", Vec::new()), "Super+C"),
                    shortcut(node(31, "Find", Vec::new()), "Super+F"),
                    Node {
                        enabled: false,
                        ..node(32, "Paste", Vec::new())
                    },
                ],
            ),
        ],
    );
    cascade(&AppMenus {
        name: "Files",
        root: Some(&root),
        item: ":1.7/org/bande/MenuBar",
        actions: &[],
        windows: &[],
    })
}

fn key(key: Key) -> Event {
    Event::Key {
        key,
        modifiers: Modifiers::default(),
    }
}

fn typed(search: &mut MenuSearch, text: &str) {
    for character in text.chars() {
        search.handle(&key(Key::Character(character)));
    }
}

fn opened() -> (MenuSearch, Rc<Cell<Instant>>) {
    let mut search = MenuSearch::new(Appearance::default(), TextSystem::new());
    let clock = Rc::new(Cell::new(Instant::now()));
    let time = clock.clone();
    search.set_clock(move || time.get());
    search.open(menus());
    (search, clock)
}

#[test]
fn every_enabled_item_is_listed_with_its_path_and_shortcut() {
    let (search, _) = opened();
    let results = search.results();
    assert!(results.contains(&("Edit › Find".to_owned(), Some("Super+F".to_owned()))));
    assert!(results.contains(&("File › New Window".to_owned(), Some("Super+N".to_owned()))));
    assert!(results.contains(&("Files › About Files".to_owned(), None)));
    assert!(results.iter().all(|(title, _)| !title.ends_with("Paste")));
}

#[test]
fn typing_ranks_items_that_start_with_the_query_first_and_return_runs_the_selection() {
    let (mut search, _) = opened();
    typed(&mut search, "fi");
    let titles: Vec<String> = search
        .results()
        .into_iter()
        .map(|(title, _)| title)
        .collect();
    assert_eq!(
        titles,
        [
            "File › Find in Folder",
            "Edit › Find",
            "Files › About Files"
        ]
    );
    search.handle(&key(Key::Down));
    assert_eq!(search.selected(), 1);
    search.handle(&key(Key::Enter));
    let chosen = search.take_chosen().expect("an item was chosen");
    assert_eq!(
        Command::parse(&chosen),
        Some(Command::Item {
            item: ":1.7/org/bande/MenuBar".to_owned(),
            id: 31
        })
    );
}

#[test]
fn escape_clears_the_query_first_and_closes_the_card_second() {
    let (mut search, _) = opened();
    typed(&mut search, "zzz");
    assert!(search.results().is_empty());
    search.handle(&key(Key::Escape));
    assert!(!search.take_closing());
    assert!(!search.results().is_empty());
    search.handle(&key(Key::Escape));
    assert!(search.take_closing());
}

#[test]
fn the_card_matches_its_golden() {
    let (mut search, clock) = opened();
    typed(&mut search, "f");
    search.scene(SIZE.0, SIZE.1);
    clock.set(clock.get() + Duration::from_secs(1));
    let scene = search.scene(SIZE.0, SIZE.1);
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    check_golden("menu-search-light", SIZE, frame.rgba);
}
