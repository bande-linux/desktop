#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use unbound_file_chooser::{Chooser, ChooserView, Finished, Icons};
use unbound_files::rows;
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{Appearance, CpuRenderer, Scheme, TextSystem};
use unbound_xdg::MimeDatabase;
use xdg_desktop_portal_unbound::file_chooser::{Dialog, Filter, Mode, Pattern};

const SIZE: (u32, u32) = (820, 520);
const MIN_SSIM: f64 = 0.99;

fn check_golden(name: &str, rgba: Vec<u8>) {
    let actual = image::RgbaImage::from_raw(SIZE.0, SIZE.1, rgba).expect("frame");
    let actual = image::DynamicImage::ImageRgba8(actual).to_rgb8();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
    if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().expect("golden directory")).expect("directory");
        actual.save(&path).expect("write golden");
    }
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    actual.save(&target).expect("save the frame");
    let expected = image::open(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .to_rgb8();
    let similarity = image_compare::rgb_hybrid_compare(&expected, &actual).expect("same size");
    assert!(
        similarity.score >= MIN_SSIM,
        "{name} differs from its golden: {:.4} (frame in {})",
        similarity.score,
        target.display()
    );
}

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

fn view(home: &Path, mode: Mode, scheme: Scheme) -> ChooserView {
    let chooser = Chooser::new(
        dialog(mode),
        MimeDatabase::load(&[]),
        home.join("Documents"),
    );
    ChooserView::new(
        chooser,
        rows(home, None, None),
        home.to_path_buf(),
        Icons::new(Vec::new()),
        Appearance {
            scheme,
            ..Appearance::default()
        },
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
    for (scheme, suffix) in [(Scheme::Light, "light"), (Scheme::Dark, "dark")] {
        let mut open = view(
            &home.home,
            Mode::Open {
                multiple: true,
                directory: false,
            },
            scheme,
        );
        check_golden(&format!("open-{suffix}"), render(&mut open));
        let mut save = view(
            &home.home,
            Mode::Save {
                name: Some("Untitled.txt".to_owned()),
                file: None,
            },
            scheme,
        );
        check_golden(&format!("save-{suffix}"), render(&mut save));
    }
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
        Scheme::Light,
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
        Scheme::Light,
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
