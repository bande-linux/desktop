#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use bande_fs::{Answer, Job, Report, Resolution, Sort, Task, Trash, list, rename};
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
