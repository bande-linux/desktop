#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use tempfile::TempDir;
use unbound_fs::{
    Action, Answer, Change, Entry, FolderChanges, FolderWatch, History, Job, Outcome, RenameError,
    Report, Resolution, Sort, Task, Trash, TrashDir, duplicate_name, format_size, keep_both_name,
    list, list_date, merge, new_folder, rename, untitled_folder,
};

struct Place {
    dir: TempDir,
    trash: Trash,
}

impl Place {
    fn new() -> Self {
        let dir = TempDir::new().expect("dir");
        let trash = Trash::new(dir.path().join("Trash"), rustix::process::getuid().as_raw());
        Self { dir, trash }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    fn file(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("folders");
        fs::write(&path, text).expect("write");
        path
    }

    fn run(&self, task: Task) -> Outcome {
        self.run_answering(task, &[])
    }

    fn run_answering(&self, task: Task, answers: &[Answer]) -> Outcome {
        let job = Job::start(task, self.trash.clone()).expect("job");
        let mut answers = answers.iter();
        loop {
            match job.wait() {
                Report::Finished(outcome) => return outcome,
                Report::Conflict(_) => job.answer(*answers.next().expect("an answer")),
                Report::Progress(_) => {}
            }
        }
    }
}

fn names(folder: &Path, show_hidden: bool) -> Vec<String> {
    list(folder, show_hidden, Sort::default())
        .expect("listing")
        .into_iter()
        .map(|entry| entry.name)
        .collect()
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read")
}

fn answer(resolution: Resolution, apply_to_all: bool) -> Answer {
    Answer {
        resolution,
        apply_to_all,
    }
}

#[test]
fn new_names_follow_the_finder_conventions() {
    let place = Place::new();
    let folder = place.dir.path();
    place.file("photo.jpg", "");
    fs::create_dir(place.path("v1.2")).expect("folder");

    assert_eq!(
        duplicate_name(folder, "photo.jpg", false),
        folder.join("photo copy.jpg")
    );
    place.file("photo copy.jpg", "");
    assert_eq!(
        duplicate_name(folder, "photo.jpg", false),
        folder.join("photo copy 2.jpg")
    );
    assert_eq!(
        duplicate_name(folder, "v1.2", true),
        folder.join("v1.2 copy")
    );
    assert_eq!(
        duplicate_name(folder, ".bashrc", false),
        folder.join(".bashrc copy")
    );
    assert_eq!(
        keep_both_name(folder, "photo.jpg", false),
        folder.join("photo 2.jpg")
    );
    place.file("report 2.txt", "");
    assert_eq!(
        keep_both_name(folder, "report 2.txt", false),
        folder.join("report 3.txt")
    );
    assert_eq!(untitled_folder(folder), folder.join("untitled folder"));
    fs::create_dir(place.path("untitled folder")).expect("folder");
    assert_eq!(untitled_folder(folder), folder.join("untitled folder 2"));
}

#[test]
fn trashed_items_remember_where_they_came_from() {
    let place = Place::new();
    let first = place.file("docs/a b.txt", "one");
    let item = place.trash.put(&first).expect("trash");
    let second = place.file("docs/a b.txt", "two");
    let again = place.trash.put(&second).expect("trash");

    assert!(!first.exists());
    assert_ne!(item.id, again.id);
    assert_eq!(read(&item.path()), "one");
    assert_eq!(read(&again.path()), "two");
    let info = read(&place.path("Trash/info/a b.txt.trashinfo"));
    assert!(info.starts_with("[Trash Info]\nPath="));
    assert!(info.contains("a%20b.txt\nDeletionDate="));
    let mut items = place.trash.items();
    items.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(items, [item.clone(), again]);
    assert_eq!(items[0].original, first);
    assert_eq!(items[0].name(), "a b.txt");
    assert!(items[0].deleted.is_some());
    assert!(place.trash.put(&item.path()).is_err());
}

#[test]
fn a_volume_trash_uses_a_shared_sticky_folder_or_a_private_one() {
    let place = Place::new();
    let uid = rustix::process::getuid().as_raw();
    let volume = place.path("volume");
    fs::create_dir(&volume).expect("volume");

    let private = TrashDir::for_volume(&volume, uid).expect("trash");
    assert_eq!(private.root(), volume.join(format!(".Trash-{uid}")));
    assert_eq!(
        fs::metadata(private.root())
            .expect("trash")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    fs::create_dir(volume.join(".Trash")).expect("shared");
    fs::set_permissions(volume.join(".Trash"), fs::Permissions::from_mode(0o1777)).expect("sticky");
    let shared = TrashDir::for_volume(&volume, uid).expect("trash");
    assert_eq!(shared.root(), volume.join(".Trash").join(uid.to_string()));

    let file = place.file("volume/music/song.ogg", "la");
    let item = shared
        .put(&file, jiff::civil::date(2026, 10, 29).at(12, 30, 0, 0))
        .expect("put");
    let info = read(&shared.root().join("info/song.ogg.trashinfo"));
    assert_eq!(
        info,
        "[Trash Info]\nPath=music/song.ogg\nDeletionDate=2026-10-29T12:30:00\n"
    );
    assert_eq!(shared.items(), [item]);
    assert_eq!(shared.items()[0].original, file);
}

#[test]
fn copies_keep_contents_permissions_times_and_links() {
    let place = Place::new();
    let script = place.file("src/bin/run.sh", "#!/bin/sh\n");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o750)).expect("mode");
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    fs::File::options()
        .write(true)
        .open(&script)
        .and_then(|file| file.set_modified(old))
        .expect("mtime");
    symlink("bin/run.sh", place.path("src/link")).expect("link");
    let destination = place.path("dest");
    fs::create_dir(&destination).expect("dest");

    let outcome = place.run(Task::Copy {
        sources: vec![place.path("src")],
        destination: destination.clone(),
    });

    assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
    assert_eq!(outcome.changes, [Change::Created(destination.join("src"))]);
    let copied = destination.join("src/bin/run.sh");
    assert_eq!(read(&copied), "#!/bin/sh\n");
    let metadata = fs::metadata(&copied).expect("copy");
    assert_eq!(metadata.permissions().mode() & 0o777, 0o750);
    assert_eq!(metadata.modified().expect("mtime"), old);
    assert_eq!(
        fs::read_link(destination.join("src/link")).expect("link"),
        Path::new("bin/run.sh")
    );
    assert!(script.exists());
}

#[test]
fn copying_into_the_same_folder_duplicates_and_into_itself_fails() {
    let place = Place::new();
    let file = place.file("a.txt", "a");

    let outcome = place.run(Task::Copy {
        sources: vec![file.clone()],
        destination: place.dir.path().to_path_buf(),
    });
    assert_eq!(outcome.changes, [Change::Created(place.path("a copy.txt"))]);

    let folder = place.path("folder");
    fs::create_dir_all(folder.join("inner")).expect("folder");
    let outcome = place.run(Task::Copy {
        sources: vec![folder.clone()],
        destination: folder.join("inner"),
    });
    assert!(outcome.changes.is_empty());
    assert_eq!(outcome.failures.len(), 1);
    assert!(!folder.join("inner/folder").exists());
}

#[test]
fn conflicts_are_resolved_by_the_answers() {
    let place = Place::new();
    let sources = ["x.txt", "y.txt", "z.txt"]
        .map(|name| place.file(&format!("from/{name}"), "new"))
        .to_vec();
    for name in ["x.txt", "y.txt", "z.txt"] {
        place.file(&format!("to/{name}"), "old");
    }
    let destination = place.path("to");

    let outcome = place.run_answering(
        Task::Copy {
            sources: sources.clone(),
            destination: destination.clone(),
        },
        &[
            answer(Resolution::KeepBoth, false),
            answer(Resolution::Replace, true),
        ],
    );

    assert!(outcome.failures.is_empty());
    assert_eq!(read(&destination.join("x.txt")), "old");
    assert_eq!(read(&destination.join("x 2.txt")), "new");
    assert_eq!(read(&destination.join("y.txt")), "new");
    assert_eq!(read(&destination.join("z.txt")), "new");
    let replaced = place.trash.items();
    assert_eq!(replaced.len(), 2);
    assert!(replaced.iter().all(|item| read(&item.path()) == "old"));

    let outcome = place.run_answering(
        Task::Copy {
            sources: sources.clone(),
            destination: destination.clone(),
        },
        &[
            answer(Resolution::Skip, false),
            answer(Resolution::Stop, false),
        ],
    );
    assert!(outcome.changes.is_empty());
    assert!(!outcome.cancelled);
}

#[test]
fn cancelling_while_a_conflict_waits_keeps_what_was_done() {
    let place = Place::new();
    let first = place.file("from/1.txt", "1");
    let second = place.file("from/2.txt", "2");
    place.file("to/2.txt", "old");
    let destination = place.path("to");
    let job = Job::start(
        Task::Copy {
            sources: vec![first, second],
            destination: destination.clone(),
        },
        place.trash.clone(),
    )
    .expect("job");

    let outcome = loop {
        match job.wait() {
            Report::Conflict(conflict) => {
                assert_eq!(conflict.target, destination.join("2.txt"));
                job.cancel();
            }
            Report::Finished(outcome) => break outcome,
            Report::Progress(_) => {}
        }
    };

    assert!(outcome.cancelled);
    assert_eq!(
        outcome.changes,
        [Change::Created(destination.join("1.txt"))]
    );
    assert_eq!(read(&destination.join("2.txt")), "old");
}

#[test]
fn moves_trash_and_copies_are_undone_and_redone() {
    let place = Place::new();
    let file = place.file("a/file.txt", "f");
    let destination = place.path("b");
    fs::create_dir(&destination).expect("b");

    let moved = place.run(Task::Move {
        sources: vec![file.clone()],
        destination: destination.clone(),
    });
    assert_eq!(
        moved.changes,
        [Change::Moved {
            from: file.clone(),
            to: destination.join("file.txt"),
        }]
    );
    let undone = place.run(Task::Revert {
        changes: moved.changes,
    });
    assert_eq!(read(&file), "f");
    let redone = place.run(Task::Revert {
        changes: undone.changes,
    });
    assert!(!file.exists());
    assert_eq!(read(&destination.join("file.txt")), "f");
    place.run(Task::Revert {
        changes: redone.changes,
    });

    let trashed = place.run(Task::Trash {
        sources: vec![file.clone()],
    });
    assert!(!file.exists());
    let restored = place.run(Task::Revert {
        changes: trashed.changes,
    });
    assert_eq!(read(&file), "f");
    assert_eq!(restored.changes, [Change::Created(file.clone())]);
    assert!(place.trash.is_empty());

    let copied = place.run(Task::Copy {
        sources: vec![file.clone()],
        destination: destination.clone(),
    });
    place.run(Task::Revert {
        changes: copied.changes,
    });
    assert!(!destination.join("file.txt").exists());
    assert_eq!(place.trash.items().len(), 1);
    assert_eq!(read(&file), "f");
}

#[test]
fn put_back_recreates_folders_and_asks_when_the_place_is_taken() {
    let place = Place::new();
    let file = place.file("deep/folder/note.txt", "note");
    place.run(Task::Trash {
        sources: vec![file.clone()],
    });
    fs::remove_dir_all(place.path("deep")).expect("remove");

    place.run(Task::Restore {
        items: place.trash.items(),
    });
    assert_eq!(read(&file), "note");

    place.run(Task::Trash {
        sources: vec![file.clone()],
    });
    place.file("deep/folder/note.txt", "newer");
    let outcome = place.run_answering(
        Task::Restore {
            items: place.trash.items(),
        },
        &[answer(Resolution::KeepBoth, false)],
    );
    assert_eq!(read(&file), "newer");
    assert_eq!(read(&place.path("deep/folder/note 2.txt")), "note");
    assert_eq!(
        outcome.changes,
        [Change::Created(place.path("deep/folder/note 2.txt"))]
    );
    assert!(place.trash.is_empty());
}

#[test]
fn emptying_the_trash_erases_everything_in_it() {
    let place = Place::new();
    let files = [place.file("a", "a"), place.file("b/c", "c")];
    let folder = place.path("b");
    place.run(Task::Trash {
        sources: vec![files[0].clone(), folder],
    });
    assert_eq!(place.trash.items().len(), 2);

    let outcome = place.run(Task::EmptyTrash);

    assert!(outcome.failures.is_empty());
    assert!(place.trash.is_empty());
    assert_eq!(
        fs::read_dir(place.path("Trash/files"))
            .expect("files")
            .count(),
        0
    );
}

#[test]
fn renaming_validates_the_new_name() {
    let place = Place::new();
    let file = place.file("old.txt", "x");
    place.file("taken.txt", "y");

    assert!(matches!(rename(&file, ""), Err(RenameError::Empty)));
    assert!(matches!(
        rename(&file, "a/b"),
        Err(RenameError::ContainsSlash)
    ));
    assert!(matches!(rename(&file, ".."), Err(RenameError::Reserved)));
    assert!(matches!(
        rename(&file, "taken.txt"),
        Err(RenameError::Exists)
    ));
    assert!(matches!(rename(&file, "old.txt"), Ok(None)));
    let change = rename(&file, "new.txt").expect("rename");
    assert_eq!(
        change,
        Some(Change::Moved {
            from: file,
            to: place.path("new.txt"),
        })
    );
    assert_eq!(
        new_folder(place.dir.path()).expect("folder"),
        Change::Created(place.path("untitled folder"))
    );
}

#[test]
fn history_redoes_only_what_was_undone_since_the_last_action() {
    let mut history = History::default();
    let created = |name: &str| vec![Change::Created(PathBuf::from(name))];
    history.record(Action::NewFolder, created("a"));
    history.record(Action::Rename, Vec::new());
    history.record(Action::Duplicate, created("b"));

    let step = history.take_undo().expect("undo");
    assert_eq!(step.action, Action::Duplicate);
    history.undone(step);
    assert_eq!(history.next_undo(), Some(Action::NewFolder));
    assert_eq!(history.next_redo(), Some(Action::Duplicate));
    let step = history.take_redo().expect("redo");
    history.redone(step);
    assert_eq!(history.next_undo(), Some(Action::Duplicate));
    let step = history.take_undo().expect("undo");
    history.undone(step);
    history.record(Action::Copy, created("c"));
    assert_eq!(history.next_redo(), None);
}

#[test]
fn watched_folders_report_bursts_of_changes_once() {
    let place = Place::new();
    let folder = place.path("watched");
    fs::create_dir(&folder).expect("folder");
    let mut watch = FolderWatch::new().expect("inotify");
    watch.watch(std::slice::from_ref(&folder));

    for index in 0..20 {
        place.file(&format!("watched/{index}"), "x");
    }
    place.file("elsewhere", "x");
    let timeout = Timespec {
        tv_sec: 5,
        tv_nsec: 0,
    };
    let fd = watch.fd();
    poll(&mut [PollFd::new(&fd, PollFlags::IN)], Some(&timeout)).expect("poll");
    watch.read();

    let now = Instant::now();
    assert!(watch.take_changed(now).is_empty());
    let deadline = watch.deadline().expect("a deadline");
    let changes = watch.take_changed(deadline);
    assert!(changes.folders.is_empty());
    let mut expected = (0..20)
        .map(|index| folder.join(index.to_string()))
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(changes.entries, expected);
    assert_eq!(watch.deadline(), None);
    assert!(watch.take_changed(deadline).is_empty());
}

fn settled(watch: &mut FolderWatch) -> FolderChanges {
    let timeout = Timespec {
        tv_sec: 5,
        tv_nsec: 0,
    };
    let fd = watch.fd();
    poll(&mut [PollFd::new(&fd, PollFlags::IN)], Some(&timeout)).expect("poll");
    watch.read();
    let deadline = watch.deadline().expect("a deadline");
    watch.take_changed(deadline)
}

#[test]
fn a_change_to_the_watched_folder_itself_asks_for_a_full_read() {
    let place = Place::new();
    let folder = place.path("watched");
    fs::create_dir(&folder).expect("folder");
    place.file("watched/inside", "x");
    let mut watch = FolderWatch::new().expect("inotify");
    watch.watch(std::slice::from_ref(&folder));

    fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).expect("chmod");
    fs::remove_file(folder.join("inside")).expect("remove");

    let changes = settled(&mut watch);
    assert_eq!(changes.folders, [folder]);
    assert!(changes.entries.is_empty());
}

#[test]
fn a_rename_inside_a_watched_folder_reports_both_names() {
    let place = Place::new();
    let folder = place.path("watched");
    let old = place.file("watched/old", "x");
    let mut watch = FolderWatch::new().expect("inotify");
    watch.watch(std::slice::from_ref(&folder));

    fs::rename(&old, folder.join("new")).expect("rename");

    let changes = settled(&mut watch);
    assert!(changes.folders.is_empty());
    assert_eq!(changes.entries, [folder.join("new"), old]);
}

#[test]
fn folders_come_first_then_names_in_finder_order() {
    let root = TempDir::new().expect("temporary folder");
    for name in ["file10", "File2", "beta", "Alpha", ".hidden", "file1"] {
        fs::write(root.path().join(name), "").expect("file");
    }
    fs::create_dir(root.path().join("zoo")).expect("folder");

    assert_eq!(
        names(root.path(), false),
        ["zoo", "Alpha", "beta", "file1", "File2", "file10"]
    );
    assert_eq!(names(root.path(), true)[1], ".hidden");
}

#[test]
fn a_sorted_chunk_merges_into_the_listing_and_reports_where_rows_moved() {
    let root = TempDir::new().expect("temporary folder");
    let entry = |name: &str| {
        let path = root.path().join(name);
        fs::write(&path, "").expect("file");
        Entry::read(path, name.to_owned())
    };
    let mut entries = vec![entry("b"), entry("d"), entry("f")];
    let chunk = vec![entry("a"), entry("c"), entry("g")];

    let moved = merge(&mut entries, chunk, Sort::default());

    let shown = entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(shown, ["a", "b", "c", "d", "f", "g"]);
    assert_eq!(moved, [1, 3, 4]);
}

#[test]
fn list_dates_read_today_yesterday_this_year_and_older() {
    let at = |text: &str| SystemTime::from(text.parse::<jiff::Timestamp>().expect("timestamp"));
    let now = at("2026-10-30T12:00:00Z");
    let utc = jiff::tz::TimeZone::UTC;

    assert_eq!(
        list_date(at("2026-10-30T09:12:00Z"), now, &utc),
        "Today, 09:12"
    );
    assert_eq!(
        list_date(at("2026-10-29T21:03:00Z"), now, &utc),
        "Yesterday, 21:03"
    );
    assert_eq!(
        list_date(at("2026-10-28T16:20:00Z"), now, &utc),
        "28 Oct, 16:20"
    );
    assert_eq!(
        list_date(at("2025-12-31T23:59:00Z"), now, &utc),
        "31 Dec 2025, 23:59"
    );
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
    assert_eq!(format_size(2_340_000_000), "2.34 GB");
}

#[test]
fn deleting_never_reaches_into_another_mount() {
    let place = Place::new();
    let outcome = place.run(Task::Delete {
        sources: vec![PathBuf::from("/proc")],
    });
    let kinds: Vec<_> = outcome
        .failures
        .iter()
        .map(|failure| failure.error.kind())
        .collect();
    assert_eq!(kinds, [std::io::ErrorKind::CrossesDevices]);
}

#[test]
fn deleting_a_folder_removes_links_but_not_what_they_point_to() {
    let place = Place::new();
    let kept = place.file("elsewhere/kept.txt", "kept");
    place.file("doomed/inner/deep/file.txt", "gone");
    symlink(place.path("elsewhere"), place.path("doomed/inner/link")).expect("link");
    let outcome = place.run(Task::Delete {
        sources: vec![place.path("doomed")],
    });
    assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
    assert!(place.path("doomed").symlink_metadata().is_err());
    assert_eq!(read(&kept), "kept");
}

#[test]
fn allowing_a_script_to_run_sets_execute_for_whoever_can_read() {
    let place = TempDir::new().unwrap();
    let script = place.path().join("run.sh");
    fs::write(&script, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o640)).unwrap();
    let mode = fs::metadata(&script).unwrap().permissions().mode();
    assert!(!unbound_fs::runnable(mode));

    unbound_fs::set_mode(&script, unbound_fs::with_runnable(mode, true)).unwrap();

    let mode = fs::metadata(&script).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o750);
    assert!(unbound_fs::runnable(mode));
    assert_eq!(unbound_fs::with_runnable(mode, false) & 0o777, 0o640);
}

#[test]
fn access_changes_only_the_chosen_class() {
    use unbound_fs::{Access, Class, access, with_access};
    let mode = 0o100_764;
    assert_eq!(access(mode, Class::Owner), Access::ReadWrite);
    assert_eq!(access(mode, Class::Group), Access::ReadWrite);
    assert_eq!(access(mode, Class::Others), Access::ReadOnly);

    assert_eq!(
        with_access(mode, Class::Group, Access::ReadOnly, false),
        0o100_744
    );
    assert_eq!(
        with_access(mode, Class::Owner, Access::ReadOnly, false),
        0o100_564
    );
    assert_eq!(
        with_access(mode, Class::Others, Access::NoAccess, false),
        0o100_760
    );
    assert_eq!(
        with_access(0o40_700, Class::Others, Access::ReadOnly, true),
        0o40_705
    );
}
