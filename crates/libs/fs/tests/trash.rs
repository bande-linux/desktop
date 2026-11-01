#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::os::unix::fs::symlink;

use bande_fs::TrashDir;
use tempfile::TempDir;

#[test]
fn a_volume_trash_whose_files_folder_is_a_link_is_not_used() {
    let place = TempDir::new().expect("dir");
    let uid = rustix::process::getuid().as_raw();
    let volume = place.path().join("volume");
    let documents = place.path().join("documents");
    fs::create_dir_all(&documents).expect("documents");
    fs::write(documents.join("report.txt"), "keep").expect("report");
    let trash = volume.join(format!(".Trash-{uid}"));
    fs::create_dir_all(trash.join("info")).expect("info");
    symlink(&documents, trash.join("files")).expect("link");

    assert!(TrashDir::for_volume(&volume, uid).is_err());
    assert_eq!(
        fs::read_to_string(documents.join("report.txt")).expect("report"),
        "keep"
    );
}
