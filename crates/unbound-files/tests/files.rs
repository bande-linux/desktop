#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

use unbound_files::{Browser, FilesView, Kind, Preview, format_size, list, preview};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{Item, Palette, Scheme, TextSystem};

fn names(folder: &Path, show_hidden: bool) -> Vec<String> {
    list(folder, show_hidden)
        .expect("listing")
        .into_iter()
        .map(|entry| entry.name)
        .collect()
}

fn tree() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("temporary folder");
    fs::create_dir_all(root.path().join("Documents/Reports")).expect("folders");
    fs::write(root.path().join("Documents/a.txt"), "first\n").expect("file");
    fs::create_dir(root.path().join("Empty")).expect("empty folder");
    fs::write(root.path().join("notes.txt"), "notes\n").expect("file");
    fs::write(root.path().join(".profile"), "hidden\n").expect("hidden file");
    root
}

fn selected_name(browser: &Browser) -> Option<String> {
    browser.selected().map(|entry| entry.name.clone())
}

fn folders(browser: &Browser) -> Vec<String> {
    browser
        .columns()
        .iter()
        .map(|column| {
            column
                .folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect()
}

#[test]
fn folders_list_in_finder_order_without_hidden_files() {
    let root = tempfile::tempdir().expect("temporary folder");
    for name in ["file10", "File2", "beta", "Alpha", ".hidden", "file1"] {
        fs::write(root.path().join(name), "").expect("file");
    }

    assert_eq!(
        names(root.path(), false),
        ["Alpha", "beta", "file1", "File2", "file10"]
    );
    assert_eq!(names(root.path(), true)[0], ".hidden");
}

#[test]
fn down_selects_and_shows_the_selected_folder_in_the_next_column() {
    let root = tree();
    let mut browser = Browser::open(root.path()).expect("browser");
    assert_eq!(browser.selected(), None);

    browser.down();

    assert_eq!(selected_name(&browser).as_deref(), Some("Documents"));
    assert_eq!(browser.columns().len(), 2);
    assert_eq!(
        names(&browser.columns()[1].folder, false),
        ["a.txt", "Reports"]
    );
    assert_eq!(browser.active(), 0);
}

#[test]
fn right_drills_into_the_folder_and_left_retreats_keeping_it_selected() {
    let root = tree();
    let mut browser = Browser::open(root.path()).expect("browser");
    browser.down();

    assert!(browser.right());
    assert_eq!(browser.active(), 1);
    assert_eq!(selected_name(&browser).as_deref(), Some("a.txt"));
    browser.down();
    assert_eq!(selected_name(&browser).as_deref(), Some("Reports"));
    assert_eq!(browser.columns().len(), 3);

    assert!(browser.left());
    assert_eq!(browser.active(), 0);
    assert_eq!(selected_name(&browser).as_deref(), Some("Documents"));
    assert_eq!(browser.columns().len(), 2);
    assert_eq!(browser.columns()[1].selected, None);
    assert!(!browser.left());
}

#[test]
fn up_and_down_stop_at_the_ends_of_the_column() {
    let root = tree();
    let mut browser = Browser::open(root.path()).expect("browser");
    browser.up();
    assert_eq!(selected_name(&browser).as_deref(), Some("notes.txt"));
    browser.down();
    assert_eq!(selected_name(&browser).as_deref(), Some("notes.txt"));
    browser.up();
    browser.up();
    browser.up();
    assert_eq!(selected_name(&browser).as_deref(), Some("Documents"));
}

#[test]
fn a_selected_file_or_an_empty_folder_cannot_be_entered() {
    let root = tree();
    let mut browser = Browser::open(root.path()).expect("browser");
    browser.down();
    browser.down();
    assert_eq!(selected_name(&browser).as_deref(), Some("Empty"));
    assert_eq!(browser.columns().len(), 2);
    assert!(!browser.right());

    browser.down();
    assert_eq!(selected_name(&browser).as_deref(), Some("notes.txt"));
    assert_eq!(browser.columns().len(), 1);
    assert!(!browser.right());
}

#[test]
fn the_enclosing_folder_opens_with_the_previous_folder_selected() {
    let root = tree();
    let mut browser = Browser::open(&root.path().join("Documents")).expect("browser");

    assert!(browser.enclosing_folder());

    assert_eq!(browser.columns()[0].folder, root.path());
    assert_eq!(selected_name(&browser).as_deref(), Some("Documents"));
    assert_eq!(folders(&browser)[1], "Documents");
}

#[test]
fn hidden_files_can_be_shown_without_losing_the_selection() {
    let root = tree();
    let mut browser = Browser::open(root.path()).expect("browser");
    browser.down();
    browser.right();

    browser.toggle_hidden();

    assert!(browser.shows_hidden());
    assert!(
        browser.columns()[0]
            .entries
            .iter()
            .any(|entry| entry.name == ".profile")
    );
    assert_eq!(browser.active(), 1);
    assert_eq!(selected_name(&browser).as_deref(), Some("a.txt"));
}

#[test]
fn quick_look_previews_images_text_folders_and_other_files() {
    let root = tree();
    let picture = root.path().join("picture.png");
    image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 255]))
        .save(&picture)
        .expect("png");
    fs::write(root.path().join("data.bin"), [0u8, 1, 2, 3, 4]).expect("binary");
    let entries = list(root.path(), false).expect("listing");
    let entry = |name: &str| {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .expect("entry")
    };

    match preview(entry("picture.png")) {
        Preview::Image(image) => assert_eq!((image.width(), image.height()), (3, 2)),
        other => panic!("{other:?}"),
    }
    match preview(entry("notes.txt")) {
        Preview::Text(text) => assert_eq!(text, "notes\n"),
        other => panic!("{other:?}"),
    }
    assert_eq!(entry("Documents").kind, Kind::Folder);
    assert!(matches!(
        preview(entry("Documents")),
        Preview::Folder { items: 2 }
    ));
    assert!(matches!(
        preview(entry("data.bin")),
        Preview::Other { size: 5 }
    ));
}

#[test]
fn sizes_read_as_in_the_finder() {
    assert_eq!(format_size(0), "Zero bytes");
    assert_eq!(format_size(1), "1 byte");
    assert_eq!(format_size(999), "999 bytes");
    assert_eq!(format_size(1_000), "1 KB");
    assert_eq!(format_size(12_400), "12 KB");
    assert_eq!(format_size(999_600), "1 MB");
    assert_eq!(format_size(1_500_000), "1.5 MB");
    assert_eq!(format_size(2_000_000), "2 MB");
    assert_eq!(format_size(2_340_000_000), "2.34 GB");
}

fn key(key: Key) -> Event {
    Event::Key {
        key,
        modifiers: Modifiers::default(),
    }
}

#[test]
fn space_shows_the_selected_image_in_quick_look_and_closes_it_again() {
    let root = tree();
    image::RgbaImage::from_pixel(40, 30, image::Rgba([0, 0, 255, 255]))
        .save(root.path().join("picture.png"))
        .expect("png");
    let mut view = FilesView::new(
        Browser::open(root.path()).expect("browser"),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    );
    view.scene(800, 600);
    view.handle(&key(Key::Up));
    assert_eq!(
        view.browser().selected().map(|entry| entry.name.as_str()),
        Some("picture.png")
    );

    assert!(view.handle(&key(Key::Space)));
    let scene = view.scene(800, 600);
    let panel = view.quick_look_frame();
    let image = scene.items.iter().find_map(|item| match item {
        Item::Clip { items, .. } => items.iter().find_map(|item| match item {
            Item::Image { rect, .. } => Some(*rect),
            _ => None,
        }),
        _ => None,
    });
    let image = image.expect("the picture is drawn");
    assert_eq!((image.width, image.height), (40.0, 30.0));
    assert!(image.x > panel.x && image.x + image.width < panel.x + panel.width);

    view.handle(&key(Key::Up));
    assert_eq!(
        view.quick_look().map(|shown| shown.entry.name.as_str()),
        Some("notes.txt")
    );

    assert!(view.handle(&key(Key::Space)));
    assert!(view.quick_look().is_none());
}
