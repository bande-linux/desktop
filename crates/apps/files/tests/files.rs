#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use bande_files::{Browser, FilesView, Location, Sheet};
use bande_fs::Trash;
use bande_toolkit::ui::{Event, Key, Modifiers};
use bande_toolkit::{Palette, Scheme, TextSystem};
use bande_xdg::Apps;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use tempfile::TempDir;

fn view(browser: Browser) -> FilesView {
    let mut view = FilesView::new(
        browser,
        Vec::new(),
        Apps::load(Vec::new(), &[]),
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
    run_until(view, |view| view.operations().running().is_empty());
}

fn run_until(view: &mut FilesView, done: impl Fn(&FilesView) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(view) {
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

#[test]
fn a_duplicate_that_cannot_be_written_asks_and_stops_without_leaving_a_copy() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let root = TempDir::new().expect("temporary folder");
    let locked = root.path().join("Locked");
    fs::create_dir(&locked).expect("folder");
    fs::write(locked.join("notes.txt"), "notes\n").expect("file");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).expect("lock");
    let trash = Trash::new(root.path().join("Trash"), 0);
    let mut files = view(Browser::open(Location::Folder(locked.clone()), trash));
    key(&mut files, Key::Down, false, false);
    assert!(files.menu_action("duplicate"));
    run_until(&mut files, |files| {
        matches!(files.sheet(), Some(Sheet::Failure))
    });
    key(&mut files, Key::Escape, false, false);
    finish(&mut files);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("unlock");
    assert_eq!(fs::read_dir(&locked).expect("listing").count(), 1);
}
