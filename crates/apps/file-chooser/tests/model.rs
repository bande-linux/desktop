#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use unbound_file_chooser::{
    Accept, Chooser, glob_matches, initial_folder, initial_name, remember, remembered_folder,
    typed_location,
};
use unbound_xdg::MimeDatabase;
use xdg_desktop_portal_unbound::file_chooser::{Dialog, Filter, Mode, Pattern};

fn dialog(mode: Mode, filters: Vec<Filter>) -> Dialog {
    Dialog {
        app_id: "org.example.Editor".to_owned(),
        parent_window: String::new(),
        title: String::new(),
        accept_label: None,
        modal: true,
        mode,
        filters,
        current_filter: None,
        choices: Vec::new(),
        folder: None,
    }
}

fn open(multiple: bool) -> Mode {
    Mode::Open {
        multiple,
        directory: false,
    }
}

fn folder() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("temp dir");
    for name in ["notes.txt", "Photo.PNG", "song.ogg", ".hidden.png"] {
        std::fs::write(folder.path().join(name), name).expect("file");
    }
    std::fs::create_dir(folder.path().join("Archive")).expect("folder");
    folder
}

fn names(chooser: &Chooser) -> Vec<String> {
    chooser
        .entries()
        .iter()
        .map(|entry| entry.name.clone())
        .collect()
}

#[test]
fn globs_match_names_without_regard_to_case() {
    assert!(glob_matches("*.png", "Photo.PNG"));
    assert!(glob_matches("*.[jJ][pP]g", "trip.JPG"));
    assert!(glob_matches("report-??.pdf", "report-07.pdf"));
    assert!(glob_matches("[!a]*", "b.txt"));
    assert!(glob_matches("*.[0-9]", "log.3"));
    assert!(!glob_matches("*.png", "notes.txt"));
    assert!(!glob_matches("report-??.pdf", "report-7.pdf"));
}

#[test]
fn a_filter_shows_matching_files_and_every_folder_but_never_hidden_files() {
    let folder = folder();
    let images = Filter {
        name: "Images".to_owned(),
        patterns: vec![Pattern::Glob("*.png".to_owned())],
    };
    let all = Filter {
        name: "All".to_owned(),
        patterns: vec![Pattern::Glob("*".to_owned())],
    };
    let mut chooser = Chooser::new(
        dialog(open(false), vec![images, all]),
        MimeDatabase::load(&[]),
        folder.path().to_path_buf(),
    );
    assert_eq!(names(&chooser), ["Archive", "Photo.PNG"]);
    chooser.set_filter(1);
    assert_eq!(
        names(&chooser),
        ["Archive", "notes.txt", "Photo.PNG", "song.ogg"]
    );
}

#[test]
fn choosing_folders_hides_files() {
    let folder = folder();
    let chooser = Chooser::new(
        dialog(
            Mode::Open {
                multiple: false,
                directory: true,
            },
            Vec::new(),
        ),
        MimeDatabase::load(&[]),
        folder.path().to_path_buf(),
    );
    assert_eq!(names(&chooser), ["Archive"]);
    let Accept::Done(selection) = chooser.accept(&[]) else {
        panic!("the current folder is chosen");
    };
    assert_eq!(selection.paths, [folder.path().to_path_buf()]);
}

#[test]
fn opening_a_single_folder_enters_it_and_files_are_chosen() {
    let folder = folder();
    let path = |name: &str| folder.path().join(name);
    let chooser = Chooser::new(
        dialog(open(false), Vec::new()),
        MimeDatabase::load(&[]),
        folder.path().to_path_buf(),
    );
    assert_eq!(
        chooser.accept(&[path("Archive")]),
        Accept::Enter(path("Archive"))
    );
    assert_eq!(chooser.accept(&[]), Accept::Nothing);
    let Accept::Done(selection) = chooser.accept(&[path("notes.txt"), path("song.ogg")]) else {
        panic!("files are chosen");
    };
    assert_eq!(
        selection.paths,
        [path("notes.txt")],
        "one file unless multiple"
    );

    let chooser = Chooser::new(
        dialog(open(true), Vec::new()),
        MimeDatabase::load(&[]),
        folder.path().to_path_buf(),
    );
    let Accept::Done(selection) = chooser.accept(&[path("notes.txt"), path("song.ogg")]) else {
        panic!("files are chosen");
    };
    assert_eq!(selection.paths, [path("notes.txt"), path("song.ogg")]);
}

#[test]
fn saving_over_an_existing_file_asks_first_and_a_folder_name_enters_it() {
    let folder = folder();
    let save = Mode::Save {
        name: Some("draft.txt".to_owned()),
        file: None,
    };
    let mut chooser = Chooser::new(
        dialog(save, Vec::new()),
        MimeDatabase::load(&[]),
        folder.path().to_path_buf(),
    );
    assert_eq!(chooser.name, "draft.txt");
    let Accept::Done(selection) = chooser.accept(&[]) else {
        panic!("a new name is saved at once");
    };
    assert_eq!(selection.paths, [folder.path().join("draft.txt")]);

    chooser.name = "notes.txt".to_owned();
    assert_eq!(
        chooser.accept(&[]),
        Accept::Replace(folder.path().join("notes.txt"))
    );
    chooser.name = "Archive".to_owned();
    assert_eq!(
        chooser.accept(&[]),
        Accept::Enter(folder.path().join("Archive"))
    );
    for name in ["", "  ", "a/b", ".."] {
        chooser.name = name.to_owned();
        assert_eq!(chooser.accept(&[]), Accept::Nothing, "{name:?}");
    }
}

#[test]
fn the_last_folder_is_remembered_for_each_app() {
    let memory = remember("", "org.example.Editor", Path::new("/home/anna/Notes"));
    let memory = remember(
        &memory,
        "org.example.Viewer",
        Path::new("/home/anna/Pictures"),
    );
    let memory = remember(&memory, "org.example.Editor", Path::new("/home/anna/Work"));
    assert_eq!(
        remembered_folder(&memory, "org.example.Editor"),
        Some(PathBuf::from("/home/anna/Work"))
    );
    assert_eq!(
        remembered_folder(&memory, "org.example.Viewer"),
        Some(PathBuf::from("/home/anna/Pictures"))
    );
    assert_eq!(memory.lines().count(), 2);
    assert_eq!(remembered_folder(&memory, "org.example.Other"), None);
}

#[test]
fn a_remembered_folder_keeps_its_exact_name() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let folder = Path::new(OsStr::from_bytes(b"/home/anna/caf\xe9\n\tnotes"));
    let memory = remember("", "org.example.Editor", folder);
    let memory = remember(&memory, "org.example.Viewer", Path::new("/home/anna"));
    assert_eq!(
        remembered_folder(&memory, "org.example.Editor").as_deref(),
        Some(folder)
    );
    assert_eq!(memory.lines().count(), 2);
}

#[test]
fn the_dialog_starts_in_the_apps_folder_then_the_remembered_one_then_a_default() {
    let folder = folder();
    let home = Path::new("/home/anna");
    let documents = Path::new("/home/anna/Documents");
    let archive = folder.path().join("Archive");

    let mut asked = dialog(open(false), Vec::new());
    assert_eq!(
        initial_folder(&asked, Some(archive.clone()), home, documents),
        archive
    );
    assert_eq!(initial_folder(&asked, None, home, documents), home);
    asked.folder = Some(folder.path().to_path_buf());
    assert_eq!(
        initial_folder(&asked, Some(archive.clone()), home, documents),
        folder.path()
    );

    let save = dialog(
        Mode::Save {
            name: None,
            file: Some(archive.join("old.txt")),
        },
        Vec::new(),
    );
    assert_eq!(initial_folder(&save, None, home, documents), archive);
    assert_eq!(initial_name(&save), "old.txt");
    let save = dialog(
        Mode::Save {
            name: None,
            file: None,
        },
        Vec::new(),
    );
    assert_eq!(initial_folder(&save, None, home, documents), documents);
}

#[test]
fn typed_locations_expand_the_home_folder_and_resolve_against_the_shown_one() {
    let home = Path::new("/home/anna");
    let shown = Path::new("/home/anna/Documents");
    assert_eq!(typed_location(" ~ ", home, shown), home);
    assert_eq!(
        typed_location("~/Pictures/", home, shown),
        home.join("Pictures")
    );
    assert_eq!(typed_location("/etc", home, shown), Path::new("/etc"));
    assert_eq!(
        typed_location("Receipts", home, shown),
        shown.join("Receipts")
    );
    assert_eq!(typed_location("~bob", home, shown), shown.join("~bob"));
}
