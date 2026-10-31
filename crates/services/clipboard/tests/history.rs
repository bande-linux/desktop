#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::fs::PermissionsExt;

use bande_clipboard::{Contents, History, KEPT_ITEMS, KEPT_SECONDS, MAX_ITEM_BYTES, summary};

const NOW: u64 = 1_800_000_000;
const TEXT: &str = "text/plain;charset=utf-8";

fn text(value: &str) -> Contents {
    vec![
        (TEXT.to_owned(), value.as_bytes().to_vec()),
        (
            "text/html".to_owned(),
            format!("<b>{value}</b>").into_bytes(),
        ),
    ]
}

fn summaries(history: &History) -> Vec<String> {
    history
        .entries()
        .iter()
        .map(|entry| summary(&entry.contents))
        .collect()
}

#[test]
fn copies_are_kept_newest_first_and_a_repeated_copy_moves_to_the_top() {
    let folder = tempfile::tempdir().unwrap();
    let mut history = History::load(folder.path().join("clipboard"), NOW);
    assert!(history.record(text("first"), NOW));
    assert!(history.record(text("second"), NOW + 1));
    assert!(history.record(text("first"), NOW + 2));
    assert_eq!(summaries(&history), ["first", "second"]);
    assert_eq!(history.entries()[0].contents, text("first"));
}

#[test]
fn passwords_and_huge_copies_are_never_kept() {
    let folder = tempfile::tempdir().unwrap();
    let mut history = History::load(folder.path().join("clipboard"), NOW);
    let mut password = text("hunter2");
    password.push(("x-kde-passwordManagerHint".to_owned(), b"secret".to_vec()));
    assert!(!history.record(password, NOW));
    let concealed = vec![
        (TEXT.to_owned(), b"hunter2".to_vec()),
        (
            "application/x-nspasteboard-concealed-type".to_owned(),
            Vec::new(),
        ),
    ];
    assert!(!history.record(concealed, NOW));
    let huge = vec![("image/png".to_owned(), vec![0; MAX_ITEM_BYTES + 1])];
    assert!(!history.record(huge, NOW));
    assert!(history.entries().is_empty());
    assert!(
        std::fs::read_dir(folder.path().join("clipboard"))
            .map(|files| files.count() == 0)
            .unwrap_or(true)
    );
}

#[test]
fn the_history_keeps_a_week_and_at_most_a_hundred_items() {
    let folder = tempfile::tempdir().unwrap();
    let mut history = History::load(folder.path().join("clipboard"), NOW);
    history.record(text("old"), NOW - KEPT_SECONDS - 10);
    for index in 0..KEPT_ITEMS + 5 {
        history.record(text(&format!("copy {index}")), NOW);
    }
    assert_eq!(history.entries().len(), KEPT_ITEMS);
    assert!(!summaries(&history).contains(&"old".to_owned()));
    assert!(!summaries(&history).contains(&"copy 4".to_owned()));
    assert_eq!(
        std::fs::read_dir(folder.path().join("clipboard"))
            .unwrap()
            .count(),
        KEPT_ITEMS
    );
}

#[test]
fn the_history_survives_a_restart_and_only_the_user_can_read_it() {
    let folder = tempfile::tempdir().unwrap();
    let dir = folder.path().join("clipboard");
    let mut history = History::load(dir.clone(), NOW);
    history.record(text("kept"), NOW);
    history.record(
        vec![("image/png".to_owned(), vec![137, 80, 78, 71])],
        NOW + 1,
    );
    let reloaded = History::load(dir.clone(), NOW + 60);
    assert_eq!(reloaded.entries(), history.entries());
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for file in std::fs::read_dir(&dir).unwrap() {
        let mode = file.unwrap().metadata().unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn promoting_removing_and_clearing_change_what_is_kept() {
    let folder = tempfile::tempdir().unwrap();
    let dir = folder.path().join("clipboard");
    let mut history = History::load(dir.clone(), NOW);
    history.record(text("one"), NOW);
    history.record(text("two"), NOW + 1);
    history.record(text("three"), NOW + 2);
    let one = history.entries()[2].id;
    assert_eq!(history.promote(one, NOW + 3), Some(text("one")));
    assert_eq!(summaries(&history), ["one", "three", "two"]);
    let three = history.entries()[1].id;
    assert!(history.remove(three));
    assert_eq!(
        summaries(&History::load(dir.clone(), NOW + 4)),
        ["one", "two"]
    );
    history.clear();
    assert!(History::load(dir, NOW + 5).entries().is_empty());
}
