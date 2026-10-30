#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use unbound_bar::menu_search::MenuSearch;
use unbound_bar::menus::{AppMenus, cascade};
use unbound_status::Command;
use unbound_status::dbusmenu::Node;
use unbound_toolkit::ui::{Event, Key, Menu, Modifiers};
use unbound_toolkit::{Appearance, CpuRenderer, Scheme, TextSystem};

const SIZE: (u32, u32) = (800, 600);
const MIN_SSIM: f64 = 0.99;
const WALLPAPER: u8 = 0x80;

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
        item: ":1.7/org/unbound/MenuBar",
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

fn opened(scheme: Scheme) -> (MenuSearch, Rc<Cell<Instant>>) {
    let mut search = MenuSearch::new(
        Appearance {
            scheme,
            ..Appearance::default()
        },
        TextSystem::new(),
    );
    let clock = Rc::new(Cell::new(Instant::now()));
    let time = clock.clone();
    search.set_clock(move || time.get());
    search.open(menus());
    (search, clock)
}

#[test]
fn every_enabled_item_is_listed_with_its_path_and_shortcut() {
    let (search, _) = opened(Scheme::Light);
    let results = search.results();
    assert!(results.contains(&("Edit › Find".to_owned(), Some("Super+F".to_owned()))));
    assert!(results.contains(&("File › New Window".to_owned(), Some("Super+N".to_owned()))));
    assert!(results.contains(&("Files › About Files".to_owned(), None)));
    assert!(results.iter().all(|(title, _)| !title.ends_with("Paste")));
}

#[test]
fn typing_ranks_items_that_start_with_the_query_first_and_return_runs_the_selection() {
    let (mut search, _) = opened(Scheme::Light);
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
            item: ":1.7/org/unbound/MenuBar".to_owned(),
            id: 31
        })
    );
}

#[test]
fn escape_clears_the_query_first_and_closes_the_card_second() {
    let (mut search, _) = opened(Scheme::Light);
    typed(&mut search, "zzz");
    assert!(search.results().is_empty());
    search.handle(&key(Key::Escape));
    assert!(!search.take_closing());
    assert!(!search.results().is_empty());
    search.handle(&key(Key::Escape));
    assert!(search.take_closing());
}

#[test]
fn the_card_matches_its_goldens_in_light_and_dark_at_one_and_one_and_a_half() {
    for (scheme, scale, name) in [
        (Scheme::Light, 1.0, "menu-search-light"),
        (Scheme::Dark, 1.0, "menu-search-dark"),
        (Scheme::Light, 1.5, "menu-search-light-1.5x"),
        (Scheme::Dark, 1.5, "menu-search-dark-1.5x"),
    ] {
        let (mut search, clock) = opened(scheme);
        typed(&mut search, "f");
        search.scene(SIZE.0, SIZE.1);
        clock.set(clock.get() + Duration::from_secs(1));
        let scene = search.scene(SIZE.0, SIZE.1);
        let pixels = (
            (f64::from(SIZE.0) * scale).round() as u32,
            (f64::from(SIZE.1) * scale).round() as u32,
        );
        let mut rgba = vec![0; (pixels.0 * pixels.1 * 4) as usize];
        CpuRenderer::new()
            .render_into(&scene, scale, &mut rgba)
            .expect("frame");
        check_golden(name, pixels, rgba);
    }
}

fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    let over_wallpaper = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| {
            let behind = u16::from(255 - pixel[3]) * u16::from(WALLPAPER) / 255;
            [0, 1, 2].map(|channel| (u16::from(pixel[channel]) + behind).min(255) as u8)
        })
        .collect();
    let actual = image::RgbImage::from_raw(size.0, size.1, over_wallpaper).expect("frame");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
    if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some() {
        actual.save(&path).expect("write golden");
    }
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    actual.save(&target).expect("save the frame");
    let expected = image::open(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .to_rgb8();
    let similarity = image_compare::rgb_hybrid_compare(&expected, &actual).expect("same size");
    assert!(
        similarity.score >= MIN_SSIM,
        "{name} differs from its golden: {:.4} (frame in {})",
        similarity.score,
        target.display()
    );
}
