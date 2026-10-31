#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use bande_file_chooser::{Chooser, ChooserView, Finished, Icons};
use bande_files::rows;
use bande_toolkit::ui::{Event, Key, Modifiers};
use bande_toolkit::{Appearance, CpuRenderer, TextSystem};
use bande_xdg::MimeDatabase;
use xdg_desktop_portal_bande::file_chooser::{Dialog, Filter, Mode, Pattern};

const SIZE: (u32, u32) = (820, 520);

struct Home {
    _root: tempfile::TempDir,
    home: PathBuf,
}

fn home() -> Home {
    let root = tempfile::tempdir().expect("temp dir");
    let home = root.path().join("anna");
    let documents = home.join("Documents");
    std::fs::create_dir_all(documents.join("Receipts")).expect("folders");
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_741_950_000);
    for (name, size) in [
        ("Budget.ods", 18_400),
        ("Holiday.png", 2_400_000),
        ("Letter.txt", 1_200),
        ("song.ogg", 3_100_000),
    ] {
        let file = std::fs::File::create(documents.join(name)).expect("file");
        file.set_len(size).expect("size");
        file.set_modified(modified).expect("time");
    }
    std::fs::File::open(documents.join("Receipts"))
        .and_then(|folder| folder.set_modified(modified))
        .expect("folder time");
    Home { _root: root, home }
}

fn dialog(mode: Mode) -> Dialog {
    Dialog {
        app_id: "org.example.Editor".to_owned(),
        parent_window: String::new(),
        title: String::new(),
        accept_label: None,
        modal: true,
        mode,
        filters: vec![
            Filter {
                name: "All files".to_owned(),
                patterns: vec![Pattern::Glob("*".to_owned())],
            },
            Filter {
                name: "Images".to_owned(),
                patterns: vec![Pattern::Glob("*.png".to_owned())],
            },
        ],
        current_filter: None,
        choices: Vec::new(),
        folder: None,
    }
}

fn view(home: &Path, mode: Mode) -> ChooserView {
    let chooser = Chooser::new(
        dialog(mode),
        MimeDatabase::load(&[]),
        home.join("Documents"),
    );
    ChooserView::new(
        chooser,
        rows(home, None, None, None),
        home.to_path_buf(),
        Icons::new(Vec::new()),
        Appearance::default(),
        TextSystem::new(),
    )
}

fn render(view: &mut ChooserView) -> Vec<u8> {
    let scene = view.scene(SIZE.0, SIZE.1);
    CpuRenderer::new().render(&scene).expect("frame").rgba
}

fn key(view: &mut ChooserView, key: Key) {
    view.handle(&Event::Key {
        key,
        modifiers: Modifiers::default(),
    });
}

#[test]
fn the_open_and_save_dialogs_match_their_goldens() {
    let home = home();
    let mut open = view(
        &home.home,
        Mode::Open {
            multiple: true,
            directory: false,
        },
    );
    bande_golden::assert_golden_rgba!("open", SIZE, render(&mut open));
    let mut save = view(
        &home.home,
        Mode::Save {
            name: Some("Untitled.txt".to_owned()),
            file: None,
        },
    );
    bande_golden::assert_golden_rgba!("save", SIZE, render(&mut save));
}

#[test]
fn typing_jumps_to_a_file_and_return_opens_it() {
    let home = home();
    let mut view = view(
        &home.home,
        Mode::Open {
            multiple: false,
            directory: false,
        },
    );
    render(&mut view);
    key(&mut view, Key::Character('s'));
    key(&mut view, Key::Enter);
    let Some(Finished::Chosen(selection)) = view.take_finished() else {
        panic!("Return opens the file typed to");
    };
    assert_eq!(selection.paths, [home.home.join("Documents/song.ogg")]);
}

#[test]
fn return_on_a_folder_enters_it_and_escape_cancels() {
    let home = home();
    let mut view = view(
        &home.home,
        Mode::Open {
            multiple: false,
            directory: false,
        },
    );
    render(&mut view);
    key(&mut view, Key::Character('r'));
    key(&mut view, Key::Enter);
    assert!(view.take_finished().is_none());
    assert_eq!(view.folder(), home.home.join("Documents/Receipts"));
    render(&mut view);
    key(&mut view, Key::Escape);
    assert!(matches!(view.take_finished(), Some(Finished::Cancelled)));
}

fn type_text(view: &mut ChooserView, text: &str) {
    for character in text.chars() {
        key(view, Key::Character(character));
    }
}

fn super_shift(view: &mut ChooserView, letter: char) {
    view.handle(&Event::Key {
        key: Key::Character(letter),
        modifiers: Modifiers {
            shift: true,
            logo: true,
            ..Modifiers::default()
        },
    });
}

#[test]
fn typing_a_path_goes_to_its_folder_and_selects_a_file_there() {
    let home = home();
    let mut view = view(
        &home.home,
        Mode::Open {
            multiple: false,
            directory: false,
        },
    );
    super_shift(&mut view, 'h');
    assert_eq!(view.folder(), home.home);
    render(&mut view);
    type_text(&mut view, "~/Documents/Receipts");
    key(&mut view, Key::Enter);
    render(&mut view);
    assert_eq!(view.folder(), home.home.join("Documents/Receipts"));
    type_text(&mut view, "/");
    let deadline = Instant::now() + Duration::from_secs(5);
    while view.animating() && Instant::now() < deadline {
        render(&mut view);
        std::thread::sleep(Duration::from_millis(16));
    }
    bande_golden::assert_golden_rgba!("go-to", SIZE, render(&mut view));
    key(&mut view, Key::Backspace);
    type_text(&mut view, "~/Documents/Letter.txt");
    key(&mut view, Key::Enter);
    render(&mut view);
    assert_eq!(view.folder(), home.home.join("Documents"));
    key(&mut view, Key::Enter);
    let Some(Finished::Chosen(selection)) = view.take_finished() else {
        panic!("Return opens the file gone to");
    };
    assert_eq!(selection.paths, [home.home.join("Documents/Letter.txt")]);
}

fn super_key(view: &mut ChooserView, key: Key) {
    view.handle(&Event::Key {
        key,
        modifiers: Modifiers {
            logo: true,
            ..Modifiers::default()
        },
    });
}

#[test]
fn super_brackets_go_back_and_forward_through_the_folders_visited() {
    let home = home();
    let mut view = view(
        &home.home,
        Mode::Open {
            multiple: false,
            directory: false,
        },
    );
    let documents = home.home.join("Documents");
    let receipts = documents.join("Receipts");
    render(&mut view);
    key(&mut view, Key::Character('r'));
    key(&mut view, Key::Enter);
    super_shift(&mut view, 'h');
    assert_eq!(view.folder(), home.home);

    super_key(&mut view, Key::Character('['));
    assert_eq!(view.folder(), receipts);
    super_key(&mut view, Key::Character('['));
    assert_eq!(view.folder(), documents);
    render(&mut view);
    key(&mut view, Key::Enter);
    assert_eq!(view.folder(), receipts);
    super_key(&mut view, Key::Character(']'));
    assert_eq!(view.folder(), receipts);
    super_key(&mut view, Key::Character('['));
    super_key(&mut view, Key::Character(']'));
    assert_eq!(view.folder(), receipts);
}
