#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use tempfile::TempDir;
use unbound_files::{
    Apps, Browser, FilesView, Kind, Location, NameEditor, Preview, ROW_HEIGHT, Request, Row,
    SIDEBAR_WIDTH, Sheet, Sort, SortBy, format_size, list, permissions, preview, rows,
};
use unbound_fs::Trash;
use unbound_toolkit::ui::{Event, Key, Modifiers, Pointer};
use unbound_toolkit::{Item, Palette, Point, Scheme, TextSystem};

fn names(folder: &Path, show_hidden: bool) -> Vec<String> {
    list(folder, show_hidden, Sort::default())
        .expect("listing")
        .into_iter()
        .map(|entry| entry.name)
        .collect()
}

struct Place {
    root: TempDir,
    trash: Trash,
}

impl Place {
    fn new() -> Self {
        let root = TempDir::new().expect("temporary folder");
        fs::create_dir_all(root.path().join("home/Documents/Reports")).expect("folders");
        fs::write(root.path().join("home/Documents/a.txt"), "first\n").expect("file");
        fs::create_dir(root.path().join("home/Empty")).expect("empty folder");
        fs::write(root.path().join("home/notes.txt"), "notes\n").expect("file");
        fs::write(root.path().join("home/.profile"), "hidden\n").expect("hidden file");
        let trash = Trash::new(
            root.path().join("Trash"),
            rustix::process::getuid().as_raw(),
        );
        Self { root, trash }
    }

    fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    fn browser(&self) -> Browser {
        Browser::open(Location::Folder(self.home()), self.trash.clone())
    }

    fn apps(&self) -> Apps {
        let data = self.root.path().join("data");
        fs::create_dir_all(data.join("mime")).expect("mime");
        fs::write(
            data.join("mime/globs2"),
            "50:text/plain:*.txt\n50:image/png:*.png\n",
        )
        .expect("globs");
        fs::create_dir_all(data.join("applications")).expect("applications");
        fs::write(
            data.join("applications/editor.desktop"),
            "[Desktop Entry]\nType=Application\nName=Editor\nExec=editor %F\nMimeType=text/plain;\n",
        )
        .expect("entry");
        Apps::load(vec![data], &[self.root.path().join("config")])
    }

    fn view(&self) -> FilesView {
        let mut view = FilesView::new(
            self.browser(),
            Vec::new(),
            self.apps(),
            Palette::for_scheme(Scheme::Light),
            TextSystem::new(),
        )
        .expect("view");
        view.scene(1000, 600);
        view
    }
}

fn selected_name(browser: &Browser) -> Option<String> {
    browser.selected().map(|entry| entry.name.clone())
}

fn press(key: Key) -> Event {
    Event::Key {
        key,
        modifiers: Modifiers::default(),
    }
}

fn command(key: Key, shift: bool) -> Event {
    Event::Key {
        key,
        modifiers: Modifiers {
            logo: true,
            shift,
            ..Modifiers::default()
        },
    }
}

fn run_until(view: &mut FilesView, done: impl Fn(&FilesView) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(view) {
        assert!(Instant::now() < deadline, "the view did not settle");
        let ready = {
            let fds = view.fds();
            let mut polls = fds
                .iter()
                .map(|fd| PollFd::new(fd, PollFlags::IN))
                .collect::<Vec<_>>();
            let timeout = Timespec {
                tv_sec: 0,
                tv_nsec: 20_000_000,
            };
            poll(&mut polls, Some(&timeout)).expect("poll");
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

fn idle(view: &FilesView) -> bool {
    view.operations().running().is_empty()
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
fn down_selects_and_shows_the_selected_folder_in_the_next_column() {
    let place = Place::new();
    let mut browser = place.browser();
    assert_eq!(browser.selected(), None);

    browser.down();

    assert_eq!(selected_name(&browser).as_deref(), Some("Documents"));
    assert_eq!(browser.columns().len(), 2);
    let child = browser.columns()[1].folder().expect("folder");
    assert_eq!(names(child, false), ["Reports", "a.txt"]);
    assert_eq!(browser.active(), 0);
}

#[test]
fn right_drills_into_the_folder_and_left_retreats_keeping_it_selected() {
    let place = Place::new();
    let mut browser = place.browser();
    browser.down();

    assert!(browser.right());
    assert_eq!(browser.active(), 1);
    assert_eq!(selected_name(&browser).as_deref(), Some("Reports"));
    browser.down();
    assert_eq!(selected_name(&browser).as_deref(), Some("a.txt"));
    assert_eq!(browser.columns().len(), 2);

    assert!(browser.left());
    assert_eq!(browser.active(), 0);
    assert_eq!(selected_name(&browser).as_deref(), Some("Documents"));
    assert_eq!(browser.columns()[1].selected, None);
    assert!(!browser.left());
}

#[test]
fn a_selected_file_or_an_empty_folder_cannot_be_entered() {
    let place = Place::new();
    let mut browser = place.browser();
    browser.down();
    browser.down();
    assert_eq!(selected_name(&browser).as_deref(), Some("Empty"));
    assert_eq!(browser.columns().len(), 2);
    assert!(!browser.right());

    browser.down();
    assert_eq!(selected_name(&browser).as_deref(), Some("notes.txt"));
    assert_eq!(browser.columns().len(), 1);
    assert!(!browser.right());
}

#[test]
fn the_enclosing_folder_opens_with_the_previous_folder_selected() {
    let place = Place::new();
    let mut browser = Browser::open(
        Location::Folder(place.home().join("Documents")),
        place.trash.clone(),
    );

    assert!(browser.enclosing_folder());

    assert_eq!(browser.columns()[0].folder(), Some(place.home().as_path()));
    assert_eq!(selected_name(&browser).as_deref(), Some("Documents"));
}

#[test]
fn hidden_files_can_be_shown_without_losing_the_selection() {
    let place = Place::new();
    let mut browser = place.browser();
    browser.down();
    browser.right();
    browser.down();

    browser.toggle_hidden();

    assert!(browser.shows_hidden());
    assert!(
        browser.columns()[0]
            .entries
            .iter()
            .any(|entry| entry.name == ".profile")
    );
    assert_eq!(browser.active(), 1);
    assert_eq!(selected_name(&browser).as_deref(), Some("a.txt"));
}

#[test]
fn shift_extends_the_selection_and_hides_the_next_column() {
    let place = Place::new();
    let mut browser = place.browser();
    browser.down();

    browser.extend(true);
    browser.extend(true);

    let selected = browser
        .selection()
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(selected, ["Documents", "Empty", "notes.txt"]);
    assert_eq!(browser.selected(), None);
    assert_eq!(browser.columns().len(), 1);
    browser.extend(false);
    assert_eq!(browser.selection().len(), 2);
    browser.select_all();
    assert_eq!(browser.selection().len(), 3);
}

#[test]
fn refreshing_keeps_the_selection_and_follows_removals() {
    let place = Place::new();
    let mut browser = place.browser();
    browser.up();
    assert_eq!(selected_name(&browser).as_deref(), Some("notes.txt"));

    fs::write(place.home().join("Apples.txt"), "").expect("file");
    browser.refresh(&[place.home()]);
    assert_eq!(selected_name(&browser).as_deref(), Some("notes.txt"));

    fs::remove_file(place.home().join("notes.txt")).expect("remove");
    browser.refresh(&[place.home()]);
    assert_eq!(selected_name(&browser).as_deref(), Some("Apples.txt"));
}

#[test]
fn revealing_selects_items_and_history_goes_back_and_forward() {
    let place = Place::new();
    let mut browser = place.browser();
    let report = place.home().join("Documents/a.txt");

    browser.reveal(&report);
    assert_eq!(selected_name(&browser).as_deref(), Some("a.txt"));
    assert_eq!(
        browser.root(),
        &Location::Folder(place.home().join("Documents"))
    );

    assert!(browser.go_back());
    assert_eq!(browser.root(), &Location::Folder(place.home()));
    assert_eq!(selected_name(&browser).as_deref(), Some("Documents"));
    assert!(browser.go_forward());
    assert_eq!(
        browser.root(),
        &Location::Folder(place.home().join("Documents"))
    );
    assert!(!browser.go_forward());
}

#[test]
fn the_sidebar_lists_existing_user_folders_bookmarks_and_the_trash() {
    let place = Place::new();
    let home = place.home();
    fs::create_dir(home.join("Bilder")).expect("pictures");
    let user_dirs = "XDG_PICTURES_DIR=\"$HOME/Bilder\"\nXDG_DOCUMENTS_DIR=\"$HOME/Documents\"\n";
    let bookmarks = format!(
        "file://{}/Documents/Reports\nfile:///srv/My%20Share Share\nsftp://host/x\n",
        home.display()
    );

    let rows = rows(&home, Some(user_dirs), Some(&bookmarks));

    let places = rows
        .iter()
        .filter_map(|row| match row {
            Row::Place(place) => Some((place.name.as_str(), place.location.clone())),
            Row::Heading(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        places,
        [
            ("home", Location::Folder(home.clone())),
            ("Documents", Location::Folder(home.join("Documents"))),
            ("Bilder", Location::Folder(home.join("Bilder"))),
            ("Reports", Location::Folder(home.join("Documents/Reports"))),
            ("Share", Location::Folder(PathBuf::from("/srv/My Share"))),
            ("Trash", Location::Trash),
        ]
    );
}

#[test]
fn renaming_starts_with_the_name_selected_without_its_extension() {
    let mut editor = NameEditor::new("report.final.pdf", false);
    assert_eq!(editor.selection(), 0..12);

    editor.insert("summary");
    assert_eq!(editor.text(), "summary.pdf");
    editor.key(Key::Backspace, Modifiers::default());
    editor.key(Key::Right, Modifiers::default());
    editor.key(
        Key::Left,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    editor.key(Key::Character('é'), Modifiers::default());
    assert_eq!(editor.text(), "summarépdf");
    assert_eq!(NameEditor::new("photos.2024", true).selection(), 0..11);
    assert_eq!(NameEditor::new(".bashrc", false).selection(), 0..7);
}

#[test]
fn quick_look_previews_images_text_folders_and_other_files() {
    let place = Place::new();
    let home = place.home();
    image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 255]))
        .save(home.join("picture.png"))
        .expect("png");
    fs::write(home.join("data.bin"), [0u8, 1, 2, 3, 4]).expect("binary");
    let entries = list(&home, false, Sort::default()).expect("listing");
    let entry = |name: &str| {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .expect("entry")
    };

    match preview(entry("picture.png")) {
        Preview::Image(image) => assert_eq!((image.width(), image.height()), (3, 2)),
        other => panic!("{other:?}"),
    }
    match preview(entry("notes.txt")) {
        Preview::Text(text) => assert_eq!(text, "notes\n"),
        other => panic!("{other:?}"),
    }
    assert_eq!(entry("Documents").kind, Kind::Folder);
    assert!(matches!(
        preview(entry("Documents")),
        Preview::Folder { items: 2 }
    ));
    assert!(matches!(
        preview(entry("data.bin")),
        Preview::Other { size: 5 }
    ));
}

#[test]
fn sizes_and_permissions_read_as_in_the_finder() {
    assert_eq!(format_size(0), "Zero bytes");
    assert_eq!(format_size(1), "1 byte");
    assert_eq!(format_size(999), "999 bytes");
    assert_eq!(format_size(1_000), "1 KB");
    assert_eq!(format_size(12_400), "12 KB");
    assert_eq!(format_size(999_600), "1 MB");
    assert_eq!(format_size(1_500_000), "1.5 MB");
    assert_eq!(format_size(2_340_000_000), "2.34 GB");
    assert_eq!(permissions(0o754), "rwxr-xr--");
}

#[test]
fn space_shows_the_selected_image_in_quick_look_and_closes_it_again() {
    let place = Place::new();
    image::RgbaImage::from_pixel(40, 30, image::Rgba([0, 0, 255, 255]))
        .save(place.home().join("picture.png"))
        .expect("png");
    let mut view = place.view();
    view.handle(&press(Key::Up));
    assert_eq!(
        view.browser().selected().map(|entry| entry.name.as_str()),
        Some("picture.png")
    );

    assert!(view.handle(&press(Key::Space)));
    run_until(&mut view, |view| {
        view.quick_look()
            .is_some_and(|shown| shown.preview.is_some())
    });
    let scene = view.scene(1000, 600);
    let panel = view.quick_look_frame();
    let image = scene.items.iter().find_map(|item| match item {
        Item::Clip { items, .. } => items.iter().find_map(|item| match item {
            Item::Image { rect, .. } if rect.width == 40.0 => Some(*rect),
            _ => None,
        }),
        _ => None,
    });
    let image = image.expect("the picture is drawn");
    assert_eq!((image.width, image.height), (40.0, 30.0));
    assert!(image.x > panel.x && image.x + image.width < panel.x + panel.width);

    view.handle(&press(Key::Up));
    assert_eq!(
        view.quick_look().map(|shown| shown.entry.name.as_str()),
        Some("notes.txt")
    );
    assert!(view.handle(&press(Key::Space)));
    assert!(view.quick_look().is_none());
}

#[test]
fn opening_a_file_asks_for_its_default_app() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&press(Key::Up));

    view.handle(&command(Key::Character('o'), false));

    let requests = view.take_requests();
    let [Request::Open { app, files }] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    assert_eq!(app.id, "editor.desktop");
    assert_eq!(files, &[place.home().join("notes.txt")]);
    view.handle(&press(Key::Up));
    view.handle(&press(Key::Up));
    view.handle(&press(Key::Enter));
    assert_eq!(
        view.browser().root(),
        &Location::Folder(place.home().join("Documents"))
    );
}

#[test]
fn a_new_folder_is_renamed_in_place_and_undone() {
    let place = Place::new();
    let mut view = place.view();

    view.handle(&command(Key::Character('N'), true));
    assert!(place.home().join("untitled folder").is_dir());
    assert!(view.rename_editor().is_some());
    for character in "Plans".chars() {
        view.handle(&press(Key::Character(character)));
    }
    view.handle(&press(Key::Enter));

    assert!(place.home().join("Plans").is_dir());
    assert_eq!(
        view.browser().selected().map(|entry| entry.name.as_str()),
        Some("Plans")
    );
    view.handle(&command(Key::Character('z'), false));
    run_until(&mut view, idle);
    assert!(place.home().join("untitled folder").is_dir());
    view.handle(&command(Key::Character('z'), false));
    run_until(&mut view, idle);
    assert!(!place.home().join("untitled folder").exists());
    view.handle(&command(Key::Character('Z'), true));
    run_until(&mut view, idle);
    assert!(place.home().join("untitled folder").is_dir());
}

#[test]
fn moving_to_the_trash_and_putting_back() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&press(Key::Up));

    view.handle(&command(Key::Backspace, false));
    run_until(&mut view, idle);
    assert!(!place.home().join("notes.txt").exists());
    assert_eq!(
        view.browser().selected().map(|entry| entry.name.as_str()),
        Some("Empty")
    );

    let mut browser = Browser::open(Location::Trash, place.trash.clone());
    browser.down();
    assert_eq!(selected_name(&browser).as_deref(), Some("notes.txt"));
    let mut view = FilesView::new(
        browser,
        Vec::new(),
        place.apps(),
        Palette::for_scheme(Scheme::Dark),
        TextSystem::new(),
    )
    .expect("view");
    view.scene(1000, 600);
    view.handle(&command(Key::Backspace, false));
    run_until(&mut view, idle);
    assert_eq!(
        fs::read_to_string(place.home().join("notes.txt")).expect("back"),
        "notes\n"
    );
    assert!(place.trash.is_empty());
    assert!(view.browser().columns()[0].entries.is_empty());
}

#[test]
fn a_name_conflict_asks_and_keeps_both() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Right));
    view.handle(&press(Key::Down));
    assert_eq!(
        view.browser().selected().map(|entry| entry.name.as_str()),
        Some("a.txt")
    );
    view.handle(&command(Key::Character('c'), false));
    let copied = match view.take_requests().as_slice() {
        [Request::CopyText(text)] => text.clone(),
        other => panic!("{other:?}"),
    };
    fs::write(place.home().join("a.txt"), "other").expect("file");
    let mut view = place.view();

    view.handle(&command(Key::Character('v'), false));
    assert_eq!(view.take_requests(), [Request::Paste]);
    assert!(view.pasted(&copied));
    run_until(&mut view, |view| view.sheet().is_some());
    assert!(matches!(view.sheet(), Some(Sheet::Conflict { .. })));
    view.handle(&press(Key::Enter));
    run_until(&mut view, idle);

    assert_eq!(
        fs::read_to_string(place.home().join("a.txt")).expect("old"),
        "other"
    );
    assert_eq!(
        fs::read_to_string(place.home().join("a 2.txt")).expect("new"),
        "first\n"
    );
    assert_eq!(
        view.browser().selected().map(|entry| entry.name.as_str()),
        Some("a 2.txt")
    );
}

#[test]
fn the_list_view_shows_one_folder_sorted_by_the_chosen_column() {
    let place = Place::new();
    fs::write(place.home().join("big.txt"), vec![b'x'; 5000]).expect("file");
    let mut view = place.view();
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Right));
    view.handle(&press(Key::Down));

    view.handle(&command(Key::Character('2'), false));

    let browser = view.browser();
    assert!(browser.single());
    assert_eq!(browser.columns().len(), 1);
    assert_eq!(
        browser.root(),
        &Location::Folder(place.home().join("Documents"))
    );
    assert_eq!(selected_name(browser).as_deref(), Some("a.txt"));

    let mut browser = place.browser();
    let files = |browser: &Browser| {
        browser.columns()[0]
            .entries
            .iter()
            .filter(|entry| !entry.is_folder())
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>()
    };
    browser.set_sort(Sort {
        by: SortBy::Size,
        descending: false,
    });
    assert_eq!(files(&browser), ["big.txt", "notes.txt"]);
    browser.set_sort(Sort {
        by: SortBy::Size,
        descending: true,
    });
    assert_eq!(files(&browser), ["notes.txt", "big.txt"]);
    assert!(browser.columns()[0].entries[0].is_folder());
}

#[test]
fn go_to_folder_accepts_relative_paths_and_reveals_files() {
    let place = Place::new();
    let mut view = place.view();
    let go_to = |view: &mut FilesView, typed: &str| {
        view.handle(&command(Key::Character('G'), true));
        assert!(matches!(view.sheet(), Some(Sheet::GoTo(_))));
        view.handle(&Event::Commit(typed.to_owned()));
        view.handle(&press(Key::Enter));
        assert!(view.sheet().is_none());
    };

    go_to(&mut view, "Documents/Reports");
    assert_eq!(
        view.browser().root(),
        &Location::Folder(place.home().join("Documents/Reports"))
    );
    go_to(
        &mut view,
        &place.home().join("notes.txt").display().to_string(),
    );
    assert_eq!(
        view.browser().selected().map(|entry| entry.name.as_str()),
        Some("notes.txt")
    );
    go_to(&mut view, "missing");
    assert!(view.notice().is_some());
}

fn click_row(view: &mut FilesView, row: usize, count: u8, modifiers: Modifiers) {
    let at = Point::new(SIDEBAR_WIDTH + 60.0, 6.0 + ROW_HEIGHT * (row as f32 + 0.5));
    view.handle(&Event::PointerDown(
        Pointer::at(at).with_count(count).with_modifiers(modifiers),
    ));
}

#[test]
fn clicks_select_extend_toggle_and_open() {
    let place = Place::new();
    let mut view = place.view();
    let names = |view: &FilesView| {
        view.browser()
            .selection()
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>()
    };

    click_row(&mut view, 0, 1, Modifiers::default());
    assert_eq!(names(&view), ["Documents"]);
    click_row(
        &mut view,
        2,
        1,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(names(&view), ["Documents", "Empty", "notes.txt"]);
    click_row(
        &mut view,
        1,
        1,
        Modifiers {
            logo: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(names(&view), ["Documents", "notes.txt"]);

    click_row(&mut view, 0, 1, Modifiers::default());
    view.scene(1000, 600);
    click_row(&mut view, 0, 2, Modifiers::default());
    assert_eq!(
        view.browser().root(),
        &Location::Folder(place.home().join("Documents"))
    );
}
