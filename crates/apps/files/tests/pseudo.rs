#![allow(clippy::expect_used)]

use std::fs;

use tempfile::TempDir;
use unbound_files::{Browser, FilesView, Location, rows};
use unbound_fs::Trash;
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{Palette, Scheme, TextSystem};
use unbound_xdg::Apps;

const SIZE: (u32, u32) = (900, 560);
const DATA: [&str; 6] = ["Documents", "Reports", "notes.txt", "a.txt", "Empty", "home"];

#[test]
fn the_folder_views_fit_longer_translations_and_have_no_hard_coded_text() {
    unbound_golden::use_pseudo_locale();
    let root = TempDir::new().expect("temporary folder");
    let home = root.path().join("home");
    fs::create_dir_all(home.join("Documents/Reports")).expect("folders");
    fs::create_dir(home.join("Empty")).expect("empty folder");
    fs::write(home.join("Documents/a.txt"), "first\n").expect("file");
    fs::write(home.join("notes.txt"), "notes\n").expect("file");
    let trash = Trash::new(
        root.path().join("Trash"),
        rustix::process::getuid().as_raw(),
    );
    let mut view = FilesView::new(
        Browser::open(Location::Folder(home.clone()), trash),
        rows(&home, None, None, None),
        Apps::load(Vec::new(), &[]),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    )
    .expect("view");
    let mut problems = Vec::new();
    for (key, name) in [('3', "files-columns"), ('2', "files-list"), ('1', "files-icons")] {
        view.handle(&Event::Key {
            key: Key::Character(key),
            modifiers: Modifiers {
                logo: true,
                ..Modifiers::default()
            },
        });
        view.scene(SIZE.0, SIZE.1);
        let scene = view.scene(SIZE.0, SIZE.1);
        problems.extend(unbound_golden::pseudo_problems!(name, &scene, &DATA));
    }
    assert!(problems.is_empty(), "{problems:#?}");
}
