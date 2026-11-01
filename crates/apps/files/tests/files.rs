#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use bande_files::{Browser, FilesSettings, FilesView, Location, Sheet};
use bande_fs::Trash;
use bande_toolkit::ui::{Event, Key, Modifiers};
use bande_toolkit::{Palette, Scheme, TextSystem};
use bande_xdg::Apps;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use tempfile::TempDir;

fn view(browser: Browser) -> FilesView {
    view_with(browser, Apps::load(Vec::new(), &[]))
}

fn view_with(browser: Browser, apps: Apps) -> FilesView {
    let mut view = FilesView::new(
        browser,
        Vec::new(),
        apps,
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    )
    .expect("view");
    view.scene(1000, 600);
    view
}

fn key(view: &mut FilesView, key: Key, control: bool, shift: bool) {
    view.handle(&Event::Key {
        key,
        modifiers: Modifiers {
            control,
            shift,
            ..Modifiers::default()
        },
    });
}

fn finish(view: &mut FilesView) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !view.operations().running().is_empty() {
        assert!(Instant::now() < deadline, "the operation did not finish");
        let ready = {
            let fds = view.fds();
            let mut polls = fds
                .iter()
                .map(|fd| PollFd::new(fd, PollFlags::IN))
                .collect::<Vec<_>>();
            poll(
                &mut polls,
                Some(&Timespec {
                    tv_sec: 0,
                    tv_nsec: 20_000_000,
                }),
            )
            .expect("poll");
            polls
                .iter()
                .enumerate()
                .filter(|(_, fd)| !fd.revents().is_empty())
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
        };
        for index in ready.into_iter().rev() {
            view.source_ready(index);
        }
        view.tick(Instant::now());
    }
}

fn selected(view: &FilesView) -> Option<&str> {
    view.browser().selected().map(|entry| entry.name.as_str())
}

#[test]
fn a_file_goes_to_the_trash_and_back_and_a_new_folder_is_named_in_place_and_undone() {
    let root = TempDir::new().expect("temporary folder");
    let home = root.path().join("home");
    fs::create_dir_all(home.join("Documents")).expect("folders");
    fs::write(home.join("notes.txt"), "notes\n").expect("file");
    let trash = Trash::new(
        root.path().join("Trash"),
        rustix::process::getuid().as_raw(),
    );
    let exists = |name: &str| Path::exists(&home.join(name));
    let mut files = view(Browser::open(Location::Folder(home.clone()), trash.clone()));
    key(&mut files, Key::Up, false, false);
    assert_eq!(selected(&files), Some("notes.txt"));
    key(&mut files, Key::Delete, false, false);
    finish(&mut files);
    assert!(!exists("notes.txt"));

    let mut in_trash = Browser::open(Location::Trash, trash.clone());
    in_trash.down();
    let mut trash_window = view(in_trash);
    assert_eq!(selected(&trash_window), Some("notes.txt"));
    assert!(trash_window.menu_action("put-back"));
    finish(&mut trash_window);
    assert_eq!(
        fs::read_to_string(home.join("notes.txt")).expect("back"),
        "notes\n"
    );
    assert!(trash.is_empty());

    let mut files = view(Browser::open(Location::Folder(home.clone()), trash.clone()));
    key(&mut files, Key::Character('N'), true, true);
    assert!(files.renaming().is_some());
    for character in "Plans".chars() {
        key(&mut files, Key::Character(character), false, false);
    }
    key(&mut files, Key::Enter, false, false);
    assert!(home.join("Plans").is_dir());
    assert_eq!(selected(&files), Some("Plans"));
    key(&mut files, Key::Character('z'), true, false);
    finish(&mut files);
    assert!(exists("untitled folder") && !exists("Plans"));
    key(&mut files, Key::Character('z'), true, false);
    finish(&mut files);
    assert!(!exists("untitled folder"));
}

fn type_name(view: &mut FilesView, text: &str) {
    for character in text.chars() {
        key(view, Key::Character(character), false, false);
    }
    key(view, Key::Enter, false, false);
}

#[test]
fn renaming_keeps_a_hidden_extension_and_asks_before_changing_a_shown_one() {
    let root = TempDir::new().expect("temporary folder");
    let (home, data) = (root.path().join("home"), root.path().join("data"));
    fs::create_dir_all(data.join("mime")).expect("mime folder");
    fs::write(data.join("mime/globs2"), "50:text/plain:*.txt\n").expect("globs");
    fs::create_dir_all(&home).expect("home");
    fs::write(home.join("notes.txt"), "notes\n").expect("file");
    let trash = Trash::new(root.path().join("Trash"), 0);
    let mut files = view_with(
        Browser::open(Location::Folder(home.clone()), trash),
        Apps::load(vec![data], &[]),
    );
    key(&mut files, Key::Down, false, false);
    key(&mut files, Key::Function(2), false, false);
    key(&mut files, Key::Character('a'), true, false);
    type_name(&mut files, "plans");
    assert!(home.join("plans.txt").is_file());

    files.apply_settings(&FilesSettings {
        show_extensions: true,
        ..FilesSettings::default()
    });
    key(&mut files, Key::Function(2), false, false);
    key(&mut files, Key::Character('a'), true, false);
    type_name(&mut files, "plans.md");
    assert!(matches!(files.sheet(), Some(Sheet::ChangeExtension { .. })));
    assert!(home.join("plans.txt").is_file());
    key(&mut files, Key::Escape, false, false);
    assert!(home.join("plans.txt").is_file() && files.sheet().is_none());
}
