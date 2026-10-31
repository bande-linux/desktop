#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use unbound_fs::{Change, Job, Outcome, Report, Task, Trash, archive_path, can_extract};

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
        let job = Job::start(task, self.trash.clone(), false).expect("job");
        loop {
            if let Report::Finished(outcome) = job.wait() {
                return outcome;
            }
        }
    }

    fn extract(&self, archive: &Path) -> PathBuf {
        let outcome = self.run(Task::Extract {
            archives: vec![archive.to_path_buf()],
        });
        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        match outcome.changes.as_slice() {
            [Change::Created(path)] => path.clone(),
            other => panic!("{other:?}"),
        }
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read")
}

fn hidden_leftovers(folder: &Path) -> Vec<String> {
    fs::read_dir(folder)
        .expect("folder")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.starts_with('.'))
        .collect()
}

#[test]
fn compressing_a_selection_makes_a_zip_that_extracts_into_a_folder_of_the_same_name() {
    let place = Place::new();
    let script = place.file("work/Project/run.sh", "#!/bin/sh\necho hi\n");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("mode");
    place.file("work/Project/docs/readme.txt", "read me");
    symlink("docs/readme.txt", place.path("work/Project/link")).expect("link");
    let notes = place.file("work/notes.txt", "notes");
    let sources = vec![place.path("work/Project"), notes];

    let archive = archive_path(&sources).expect("a name");
    assert_eq!(archive, place.path("work/Archive.zip"));
    let outcome = place.run(Task::Compress {
        sources: sources.clone(),
        archive: archive.clone(),
    });
    assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
    assert_eq!(outcome.changes, [Change::Created(archive.clone())]);
    assert_eq!(
        archive_path(&sources),
        Some(place.path("work/Archive 2.zip"))
    );
    assert!(can_extract(&archive));

    fs::create_dir(place.path("elsewhere")).expect("folder");
    let moved = place.path("elsewhere/Archive.zip");
    fs::rename(&archive, &moved).expect("move");
    let folder = place.extract(&moved);
    assert_eq!(folder, place.path("elsewhere/Archive"));
    assert_eq!(read(&folder.join("notes.txt")), "notes");
    assert_eq!(read(&folder.join("Project/docs/readme.txt")), "read me");
    assert_eq!(read(&folder.join("Project/link")), "read me");
    let mode = fs::metadata(folder.join("Project/run.sh"))
        .expect("script")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
    assert!(hidden_leftovers(&place.path("elsewhere")).is_empty());
}

#[test]
fn an_archive_with_one_item_extracts_that_item_beside_it_under_a_free_name() {
    let place = Place::new();
    let folder = place.path("Photos");
    place.file("Photos/beach.txt", "sand");
    let archive = archive_path(std::slice::from_ref(&folder)).expect("a name");
    assert_eq!(archive, place.path("Photos.zip"));
    place.run(Task::Compress {
        sources: vec![folder],
        archive: archive.clone(),
    });
    let extracted = place.extract(&archive);
    assert_eq!(extracted, place.path("Photos 2"));
    assert_eq!(read(&extracted.join("beach.txt")), "sand");
}

fn tar_bytes(place: &Place) -> Vec<u8> {
    place.file("source/album/one.txt", "one");
    place.file("source/two.txt", "two");
    let mut builder = tar::Builder::new(Vec::new());
    builder
        .append_dir_all("album", place.path("source/album"))
        .expect("folder");
    builder
        .append_path_with_name(place.path("source/two.txt"), "two.txt")
        .expect("file");
    builder.into_inner().expect("tar")
}

#[test]
fn compressed_tar_archives_extract_in_every_supported_format() {
    let place = Place::new();
    let tar = tar_bytes(&place);
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar).expect("gzip");
    let mut bzip = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    bzip.write_all(&tar).expect("bzip2");
    let mut xz =
        lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::default()).expect("xz");
    xz.write_all(&tar).expect("xz");
    let formats = [
        ("plain.tar", tar.clone()),
        ("gzip.tar.gz", gzip.finish().expect("gzip")),
        ("bzip.tbz2", bzip.finish().expect("bzip2")),
        ("xz.tar.xz", xz.finish().expect("xz")),
        (
            "zstd.tar.zst",
            ruzstd::encoding::compress_to_vec(
                tar.as_slice(),
                ruzstd::encoding::CompressionLevel::Fastest,
            ),
        ),
    ];
    for (name, bytes) in formats {
        let archive = place.path(name);
        File::create(&archive)
            .and_then(|mut file| file.write_all(&bytes))
            .expect("archive");
        let folder = place.extract(&archive);
        let stem = name.split('.').next().expect("stem");
        assert_eq!(folder, place.path(stem), "{name}");
        assert_eq!(read(&folder.join("album/one.txt")), "one", "{name}");
        assert_eq!(read(&folder.join("two.txt")), "two", "{name}");
    }
}

#[test]
fn entries_that_climb_out_of_the_archive_folder_are_ignored() {
    let place = Place::new();
    let archive = place.path("evil.zip");
    let mut zip = zip::ZipWriter::new(File::create(&archive).expect("zip"));
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("../escaped.txt", options).expect("entry");
    zip.write_all(b"out").expect("write");
    zip.start_file("inside.txt", options).expect("entry");
    zip.write_all(b"in").expect("write");
    zip.finish().expect("finish");

    let extracted = place.extract(&archive);
    assert_eq!(read(&extracted), "in");
    assert!(!place.path("escaped.txt").exists());
    assert!(
        !place
            .dir
            .path()
            .parent()
            .expect("parent")
            .join("escaped.txt")
            .exists()
    );
}

#[test]
fn a_broken_archive_fails_without_leaving_anything_behind() {
    let place = Place::new();
    let archive = place.file("broken.tar.gz", "not gzip at all");
    let outcome = place.run(Task::Extract {
        archives: vec![archive],
    });
    assert_eq!(outcome.failures.len(), 1);
    assert!(outcome.changes.is_empty());
    assert!(hidden_leftovers(place.dir.path()).is_empty());
    assert!(!can_extract(&place.path("notes.txt")));
}
