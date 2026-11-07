#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use bande_fs::{Answer, Job, Report, Resolution, Sort, Task, Trash, list, rename, same_volume};
use tempfile::TempDir;

fn trash(place: &TempDir) -> Trash {
    Trash::isolated(place.path().join("trash"), 0)
}

fn answer(job: &Job, resolution: Resolution) {
    job.answer(Answer {
        resolution,
        apply_to_all: false,
    });
}

fn unreadable(path: &Path) -> bool {
    fs::write(path, "new").expect("source");
    fs::set_permissions(path, fs::Permissions::from_mode(0o000)).expect("mode");
    fs::File::open(path).is_err()
}

#[test]
fn replacing_keeps_the_old_item_until_the_new_one_is_complete() {
    let place = TempDir::new().expect("dir");
    let (from, to) = (place.path().join("from"), place.path().join("to"));
    fs::create_dir_all(&from).expect("from");
    fs::create_dir_all(&to).expect("to");
    fs::write(to.join("report.txt"), "old").expect("old");
    if !unreadable(&from.join("report.txt")) {
        return;
    }

    let job = Job::start(
        Task::Copy {
            sources: vec![from.join("report.txt")],
            destination: to.clone(),
        },
        trash(&place),
        true,
    )
    .expect("job");
    assert!(matches!(job.wait(), Report::Conflict(_)));
    answer(&job, Resolution::Replace);
    assert!(matches!(job.wait(), Report::Failure(_)));
    answer(&job, Resolution::Skip);
    assert!(matches!(job.wait(), Report::Finished(_)));

    assert_eq!(
        fs::read_to_string(to.join("report.txt")).expect("old"),
        "old"
    );
    assert_eq!(fs::read_dir(&to).expect("to").count(), 1);
}

#[test]
fn names_that_are_not_utf8_survive_listing_copy_and_keep_both() {
    let place = TempDir::new().expect("dir");
    let (from, to) = (place.path().join("from"), place.path().join("to"));
    fs::create_dir_all(&from).expect("from");
    fs::create_dir_all(&to).expect("to");
    let name = OsStr::from_bytes(b"caf\xe9.txt");
    fs::write(from.join(name), "text").expect("source");

    let listed = list(&from, false, Sort::default()).expect("list");
    assert_eq!(fs::read_to_string(&listed[0].path).expect("open"), "text");
    assert!(matches!(rename(&listed[0].path, &listed[0].name), Ok(None)));

    for resolution in [None, Some(Resolution::KeepBoth)] {
        let job = Job::start(
            Task::Copy {
                sources: vec![listed[0].path.clone()],
                destination: to.clone(),
            },
            trash(&place),
            true,
        )
        .expect("job");
        if let Some(resolution) = resolution {
            assert!(matches!(job.wait(), Report::Conflict(_)));
            answer(&job, resolution);
        }
        assert!(matches!(job.wait(), Report::Finished(_)));
    }

    assert_eq!(fs::read_to_string(to.join(name)).expect("copy"), "text");
    let second = to.join(OsStr::from_bytes(b"caf\xe9 2.txt"));
    assert_eq!(fs::read_to_string(second).expect("second"), "text");
}

#[test]
fn a_move_between_disks_finishes_after_try_again() {
    let Ok(other) = TempDir::new_in("/dev/shm") else {
        return;
    };
    let place = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("dir");
    if same_volume(other.path(), place.path()) {
        return;
    }
    let source = place.path().join("album");
    fs::create_dir_all(&source).expect("source");
    fs::write(source.join("a.txt"), "a").expect("a");
    if !unreadable(&source.join("b.txt")) {
        return;
    }

    let job = Job::start(
        Task::Move {
            sources: vec![source.clone()],
            destination: other.path().to_path_buf(),
        },
        trash(&place),
        true,
    )
    .expect("job");
    assert!(matches!(job.wait(), Report::Failure(_)));
    fs::set_permissions(source.join("b.txt"), fs::Permissions::from_mode(0o600)).expect("mode");
    answer(&job, Resolution::Retry);
    assert!(matches!(job.wait(), Report::Finished(_)));

    assert!(!source.exists());
    let moved = other.path().join("album");
    assert_eq!(fs::read_to_string(moved.join("b.txt")).expect("b"), "new");
    assert_eq!(fs::read_dir(other.path()).expect("other").count(), 1);
}

#[test]
fn a_stale_partial_is_never_listed() {
    let place = TempDir::new().expect("dir");
    fs::write(place.path().join(".bande-partial-1-1"), "half").expect("partial");
    assert!(
        list(place.path(), true, Sort::default())
            .expect("list")
            .is_empty()
    );
}
