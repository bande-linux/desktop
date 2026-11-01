#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bande_files::{
    BookmarkEdit, Browser, FilesView, ListColumn, Location, PATH_BAR_HEIGHT, ROW_HEIGHT, Request,
    Row, SIDEBAR_WIDTH, Sheet, SidebarEdit, TAB_BAR_HEIGHT, TOOLBAR_HEIGHT, ViewMemory, ViewMode,
    hidden_places, permissions, rows, with_bookmarks, with_volumes, without_bookmark,
};
use bande_fs::{FolderChanges, Sort, SortBy, Trash, list};
use bande_preview::Preview;
use bande_thumbnails::{Cache, Thumbnailers, Thumbnails};
use bande_toolkit::ui::{
    ClipboardRequest, Event, Key, Menu, Modifiers, Pointer, PointerButton, ROW_HEIGHT as LIST_ROW,
    Table,
};
use bande_toolkit::{
    CpuRenderer, DragAction, FrameRegions, Image, Item, Palette, Point, SHEET_INSET, Scheme,
    TextSystem, framed_scene,
};
use bande_xdg::{Apps, file_uri};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use tempfile::TempDir;

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
            "50:text/plain:*.txt\n50:image/png:*.png\n50:application/pdf:*.pdf\n",
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

    let rows = rows(&home, Some(user_dirs), Some(&bookmarks), None);

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
            ("Recents", Location::Recents),
            ("home", Location::Folder(home.clone())),
            ("Documents", Location::Folder(home.join("Documents"))),
            ("Bilder", Location::Folder(home.join("Bilder"))),
            ("Reports", Location::Folder(home.join("Documents/Reports"))),
            ("Share", Location::Folder(PathBuf::from("/srv/My Share"))),
            ("Trash", Location::Trash),
        ]
    );
}

fn place_names(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| match row {
            Row::Place(place) => Some(place.name.clone()),
            Row::Heading(_) => None,
        })
        .collect()
}

#[test]
fn a_sidebar_layout_orders_and_hides_any_place_and_keeps_new_bookmarks() {
    let place = Place::new();
    let home = place.home();
    let reports = home.join("Documents/Reports");
    let empty = home.join("Empty");
    let bookmarks = format!("file://{}\nfile://{}\n", reports.display(), empty.display());
    let layout = format!(
        "file://{}\nfile://{}\nrecents\n-file://{}\n-trash\nfile:///gone\n",
        reports.display(),
        home.join("Documents").display(),
        home.display()
    );

    let rows = rows(&home, None, Some(&bookmarks), Some(&layout));

    assert_eq!(
        place_names(&rows),
        ["Reports", "Documents", "Recents", "Empty"]
    );
    assert_eq!(
        hidden_places(Some(&layout)),
        [Location::Folder(home.clone()), Location::Trash]
    );
}

#[test]
fn permissions_read_as_in_the_finder() {
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
    fn picture_rect(items: &[Item]) -> Option<bande_toolkit::Rect> {
        items.iter().find_map(|item| match item {
            Item::Image { rect, .. } if rect.width == 40.0 => Some(*rect),
            Item::Clip { items, .. } => picture_rect(items),
            _ => None,
        })
    }
    let image = picture_rect(&scene.items).expect("the picture is drawn");
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
    assert!(view.renaming().is_some());
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
        [Request::CopyFiles { paths, cut: false }] => {
            paths.iter().map(|path| file_uri(path)).collect::<Vec<_>>()
        }
        other => panic!("{other:?}"),
    };
    fs::write(place.home().join("a.txt"), "other").expect("file");
    let mut view = place.view();

    view.handle(&command(Key::Character('v'), false));
    assert_eq!(view.take_requests(), [Request::PasteFiles]);
    assert!(view.pasted_files(&copied, "", false));
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
fn cut_files_paste_as_a_move_and_paste_waits_for_files_on_the_clipboard() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&press(Key::Up));
    view.handle(&command(Key::Character('x'), false));
    let cut = match view.take_requests().as_slice() {
        [Request::CopyFiles { paths, cut: true }] => {
            paths.iter().map(|path| file_uri(path)).collect::<Vec<_>>()
        }
        other => panic!("{other:?}"),
    };
    let paste_enabled = |view: &mut FilesView| {
        let menu = right_click(view, Point::new(SIDEBAR_WIDTH + 60.0, 400.0));
        menu.items
            .iter()
            .any(|item| item.title == "Paste" && item.enabled)
    };
    assert!(!paste_enabled(&mut view));
    view.set_clipboard_has_files(true);
    assert!(paste_enabled(&mut view));

    view.handle(&command(Key::Character('G'), true));
    view.scene(1000, 600);
    view.handle(&Event::Commit("Documents".to_owned()));
    view.handle(&press(Key::Enter));
    assert!(view.pasted_files(&cut, "", true));
    run_until(&mut view, idle);
    assert!(!place.home().join("notes.txt").exists());
    assert!(place.home().join("Documents/notes.txt").exists());
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
    let at = Point::new(
        SIDEBAR_WIDTH + 60.0,
        TOOLBAR_HEIGHT + 6.0 + ROW_HEIGHT * (row as f32 + 0.5),
    );
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

fn type_text(view: &mut FilesView, text: &str) {
    for character in text.chars() {
        view.handle(&press(Key::Character(character)));
    }
}

fn shown(view: &FilesView) -> Vec<String> {
    view.browser().columns()[0]
        .entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect()
}

#[test]
fn searching_finds_names_below_the_folder_and_escape_returns() {
    let place = Place::new();
    fs::write(place.home().join("Documents/Reports/q3 report.txt"), "q3").expect("file");
    let mut view = place.view();
    view.handle(&press(Key::Down));

    view.handle(&command(Key::Character('f'), false));
    type_text(&mut view, "report");
    run_until(&mut view, |view| shown(view).len() == 2);
    assert_eq!(shown(&view), ["Reports", "q3 report.txt"]);
    assert_eq!(view.searching(), Some("report"));
    assert_eq!(view.mode(), ViewMode::List);
    assert_eq!(view.title(), "Searching “home”");

    view.handle(&press(Key::Down));
    view.handle(&press(Key::Down));
    assert_eq!(
        selected_name(view.browser()).as_deref(),
        Some("q3 report.txt")
    );
    assert!(view.menu_action("show-in-enclosing-folder"));
    assert_eq!(view.searching(), None);
    assert_eq!(
        view.browser().active_folder(),
        Some(place.home().join("Documents/Reports").as_path())
    );
    assert_eq!(
        selected_name(view.browser()).as_deref(),
        Some("q3 report.txt")
    );

    view.handle(&command(Key::Character('f'), false));
    type_text(&mut view, "q3");
    run_until(&mut view, |view| shown(view) == ["q3 report.txt"]);
    view.handle(&press(Key::Escape));
    assert_eq!(view.searching(), None);
    assert_eq!(view.mode(), ViewMode::Columns);
    assert_eq!(
        view.browser().active_folder(),
        Some(place.home().join("Documents/Reports").as_path())
    );
}

#[test]
fn recents_list_recently_used_files_newest_first() {
    let place = Place::new();
    let notes = place.home().join("notes.txt");
    let first = place.home().join("Documents/a.txt");
    let xbel = place.root.path().join("recently-used.xbel");
    fs::write(
        &xbel,
        format!(
            "<xbel><bookmark href=\"{}\" visited=\"2026-02-01T08:00:00Z\"/><bookmark href=\"{}\" visited=\"2026-03-01T08:00:00Z\"/></xbel>",
            file_uri(&notes),
            file_uri(&first)
        ),
    )
    .expect("recents");
    let mut view = place.view();
    view.set_recents_sources(xbel, place.root.path().join("usage"));
    view.handle(&command(Key::Character('f'), true));
    assert_eq!(view.browser().root(), &Location::Recents);
    assert_eq!(view.title(), "Recents");
    assert_eq!(shown(&view), ["a.txt", "notes.txt"]);
}

fn right_click(view: &mut FilesView, at: Point) -> Menu {
    view.handle(&Event::PointerDown(
        Pointer::at(at).with_button(PointerButton::Secondary),
    ));
    match view.take_requests().as_slice() {
        [Request::ShowMenu { menu, .. }] => menu.clone(),
        other => panic!("{other:?}"),
    }
}

fn titles(menu: &Menu) -> Vec<String> {
    menu.items
        .iter()
        .filter(|item| !item.separator)
        .map(|item| item.title.replace(['\u{2068}', '\u{2069}'], ""))
        .collect()
}

#[test]
fn right_clicking_an_item_selects_it_and_its_menu_acts_on_it() {
    let place = Place::new();
    let mut view = place.view();

    let menu = right_click(
        &mut view,
        Point::new(
            SIDEBAR_WIDTH + 60.0,
            TOOLBAR_HEIGHT + 6.0 + ROW_HEIGHT * 2.5,
        ),
    );

    assert_eq!(selected_name(view.browser()).as_deref(), Some("notes.txt"));
    assert_eq!(
        titles(&menu),
        [
            "Open",
            "Open With",
            "Quick Look",
            "Get Info",
            "Rename",
            "Duplicate",
            "Compress “notes.txt”",
            "Cut",
            "Copy",
            "Move to Trash"
        ]
    );
    let delete = menu.items.last().and_then(|item| item.alternate.as_deref());
    assert_eq!(
        delete.map(|item| (item.title.as_str(), item.action.as_str())),
        Some(("Delete Immediately…", "delete-immediately"))
    );
    let open_with = menu
        .items
        .iter()
        .find_map(|item| item.submenu.as_ref())
        .expect("open with");
    assert_eq!(titles(open_with), ["Editor", "Other…"]);
    assert!(view.menu_action(&open_with.items[0].action));
    let requests = view.take_requests();
    let [Request::Open { app, files }] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    assert_eq!(
        (app.id.as_str(), files.as_slice()),
        (
            "editor.desktop",
            [place.home().join("notes.txt")].as_slice()
        )
    );

    right_click(
        &mut view,
        Point::new(
            SIDEBAR_WIDTH + 60.0,
            TOOLBAR_HEIGHT + 6.0 + ROW_HEIGHT * 2.5,
        ),
    );
    assert!(view.menu_action("info"));
    assert_eq!(
        view.info().map(|info| info.entries[0].name.as_str()),
        Some("notes.txt")
    );
    view.handle(&command(Key::Character('i'), false));
    assert!(view.info().is_none());
    assert!(view.menu_action("trash"));
    run_until(&mut view, idle);
    assert!(!place.home().join("notes.txt").exists());
}

#[test]
fn the_trash_menus_put_back_and_ask_before_emptying() {
    let place = Place::new();
    place
        .trash
        .put(&place.home().join("notes.txt"))
        .expect("trash");
    let mut view = FilesView::new(
        Browser::open(Location::Trash, place.trash.clone()),
        Vec::new(),
        place.apps(),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    )
    .expect("view");
    view.scene(1000, 600);

    let empty_space = right_click(&mut view, Point::new(SIDEBAR_WIDTH + 60.0, 400.0));
    assert_eq!(titles(&empty_space)[0], "Empty Trash…");
    assert!(view.menu_action("empty-trash"));
    assert_eq!(view.sheet(), Some(&Sheet::EmptyTrash));
    view.handle(&press(Key::Escape));

    let item = right_click(
        &mut view,
        Point::new(
            SIDEBAR_WIDTH + 60.0,
            TOOLBAR_HEIGHT + 6.0 + ROW_HEIGHT * 0.5,
        ),
    );
    assert_eq!(titles(&item)[..2], ["Put Back", "Delete Immediately…"]);
    assert!(view.menu_action("put-back"));
    run_until(&mut view, idle);
    assert!(place.home().join("notes.txt").exists());
}

#[test]
fn the_empty_space_menu_sorts_and_shows_hidden_files() {
    let place = Place::new();
    let mut view = place.view();
    fs::write(place.home().join("big.bin"), [0u8; 4096]).expect("file");
    view.refresh(&[place.home()]);

    let menu = right_click(&mut view, Point::new(SIDEBAR_WIDTH + 60.0, 400.0));
    assert_eq!(
        titles(&menu),
        [
            "New Folder",
            "Paste",
            "Get Info",
            "Open in Terminal",
            "Sort By",
            "as Icons",
            "as List",
            "as Columns",
            "Show Hidden Files"
        ]
    );
    assert!(view.menu_action("sort:size"));
    assert_eq!(view.browser().sort().by, SortBy::Size);
    assert!(view.menu_action("toggle-hidden"));
    assert!(view.browser().shows_hidden());
    let names = view.browser().columns()[0]
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names[..2], ["Documents", "Empty"]);
    assert_eq!(names[2], "big.bin");
}

#[test]
fn the_space_beside_the_columns_and_the_info_pane_have_menus() {
    let place = Place::new();
    let mut view = place.view();
    let documents = Point::new(
        SIDEBAR_WIDTH + 60.0,
        TOOLBAR_HEIGHT + 6.0 + ROW_HEIGHT * 0.5,
    );
    view.handle(&Event::PointerDown(Pointer::at(documents)));
    view.handle(&Event::PointerUp(Pointer::at(documents)));
    view.scene(1000, 600);
    view.take_requests();

    let beside = right_click(&mut view, Point::new(950.0, 300.0));
    assert_eq!(titles(&beside)[..3], ["New Folder", "Paste", "Get Info"]);
    assert_eq!(
        view.browser().active_folder(),
        Some(place.home().join("Documents").as_path())
    );
    assert!(view.menu_action("info"));
    view.scene(1000, 600);
    view.take_requests();

    let info = right_click(&mut view, Point::new(900.0, 100.0));
    assert_eq!(
        titles(&info)[..4],
        ["New Folder", "Paste", "Get Info", "Open in Terminal"]
    );

    let notes = Point::new(
        SIDEBAR_WIDTH + 60.0,
        TOOLBAR_HEIGHT + 6.0 + ROW_HEIGHT * 2.5,
    );
    view.handle(&Event::PointerDown(Pointer::at(notes)));
    view.handle(&Event::PointerUp(Pointer::at(notes)));
    view.scene(1000, 600);
    view.take_requests();
    let info = right_click(&mut view, Point::new(900.0, 100.0));
    assert_eq!(titles(&info)[..2], ["Open", "Open With"]);
}

#[test]
fn any_place_is_removed_from_the_sidebar_and_comes_back_with_the_defaults() {
    let place = Place::new();
    let home = place.home();
    let reports = home.join("Documents/Reports");
    let bookmarks = format!("file://{}\nfile:///srv/share Share\n", reports.display());
    let mut view = place.view();
    view.set_sidebar(rows(&home, None, Some(&bookmarks), None));
    view.scene(1000, 600);

    let menu = right_click(&mut view, Point::new(40.0, 84.0 + 30.0 * 3.5));
    assert_eq!(
        titles(&menu),
        ["Open", "Open in New Tab", "Remove from Sidebar"]
    );
    assert!(view.menu_action("remove-from-sidebar"));
    let shown = [
        "recents".to_owned(),
        file_uri(&home),
        file_uri(&home.join("Documents")),
        "file:///srv/share".to_owned(),
    ];
    assert_eq!(
        view.take_requests(),
        [Request::EditSidebar(SidebarEdit {
            layout: Some(shown.map(|line| format!("{line}\n")).concat()),
            bookmarks: Some(BookmarkEdit::Remove(reports.clone())),
        })]
    );
    assert!(reports.is_dir());

    right_click(&mut view, Point::new(40.0, 84.0 + 30.0 * 2.5));
    assert!(view.menu_action("remove-from-sidebar"));
    let requests: [Request; 1] = view.take_requests().try_into().expect("one edit");
    let [Request::EditSidebar(edit)] = requests else {
        panic!("not a sidebar edit");
    };
    let layout = edit.layout.expect("a layout");
    assert_eq!(edit.bookmarks, None);
    let remaining = rows(&home, None, Some(&bookmarks), Some(&layout));
    assert_eq!(
        place_names(&remaining),
        ["Recents", "home", "Reports", "Share", "Trash"]
    );

    let menu = right_click(&mut view, Point::new(40.0, 500.0));
    assert_eq!(titles(&menu), ["Restore Default Sidebar"]);
    assert!(view.menu_action("restore-sidebar"));
    assert_eq!(
        view.take_requests(),
        [Request::EditSidebar(SidebarEdit {
            layout: None,
            bookmarks: None,
        })]
    );
}

#[test]
fn mounted_volumes_are_listed_under_locations_before_the_trash() {
    let home = PathBuf::from("/home/ada");
    let favourites = rows(&home, None, None, None);
    let stick = PathBuf::from("/run/media/ada/Stick");

    let shown = with_volumes(&favourites, std::slice::from_ref(&stick));
    let names = |rows: &[Row]| {
        rows.iter()
            .map(|row| match row {
                Row::Place(place) => place.name.clone(),
                Row::Heading(title) => title.clone(),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(&shown),
        [
            "Favourites",
            "Recents",
            "ada",
            "Locations",
            "Stick",
            "Trash"
        ]
    );
    assert_eq!(with_volumes(&shown, &[]), favourites);
}

#[test]
fn the_list_view_selects_by_click_sorts_by_header_and_opens_on_double_click() {
    let place = Place::new();
    fs::write(place.home().join("big.txt"), vec![b'x'; 5000]).expect("file");
    let mut view = place.view();
    view.handle(&command(Key::Character('2'), false));
    view.scene(1000, 600);
    let row = |index: f32| {
        Point::new(
            SIDEBAR_WIDTH + 60.0,
            TOOLBAR_HEIGHT + Table::HEADER_HEIGHT + LIST_ROW * (index + 0.5),
        )
    };
    let click = |view: &mut FilesView, at: Point, count: u8| {
        view.handle(&Event::PointerDown(Pointer::at(at).with_count(count)));
        view.handle(&Event::PointerUp(Pointer::at(at)));
        view.scene(1000, 600);
    };

    click(&mut view, row(2.0), 1);
    assert_eq!(selected_name(view.browser()).as_deref(), Some("big.txt"));

    click(
        &mut view,
        Point::new(SIDEBAR_WIDTH + 470.0, TOOLBAR_HEIGHT + 10.0),
        1,
    );
    assert_eq!(
        view.browser().sort(),
        Sort {
            by: SortBy::Size,
            descending: true
        }
    );
    assert_eq!(selected_name(view.browser()).as_deref(), Some("big.txt"));

    click(&mut view, row(0.0), 1);
    let first = selected_name(view.browser()).expect("first row");
    click(&mut view, row(0.0), 2);
    assert_eq!(
        view.browser().root(),
        &Location::Folder(place.home().join(first))
    );
}

#[test]
fn dragging_a_selection_starts_a_drag_and_dropping_on_a_folder_moves_it_there() {
    let place = Place::new();
    let mut view = place.view();
    let row = |index: f32| {
        Point::new(
            SIDEBAR_WIDTH + 60.0,
            TOOLBAR_HEIGHT + 6.0 + ROW_HEIGHT * (index + 0.5),
        )
    };
    click_row(&mut view, 1, 1, Modifiers::default());
    view.handle(&Event::PointerUp(Pointer::at(row(1.0))));
    click_row(
        &mut view,
        2,
        1,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    view.handle(&Event::PointerUp(Pointer::at(row(2.0))));
    view.scene(1000, 600);

    view.handle(&Event::PointerDown(Pointer::at(row(2.0))));
    view.handle(&Event::PointerMove(Pointer::at(Point::new(
        SIDEBAR_WIDTH + 70.0,
        row(2.0).y + 12.0,
    ))));
    let dragged = vec![place.home().join("Empty"), place.home().join("notes.txt")];
    assert!(matches!(
        view.take_requests().as_slice(),
        [Request::StartDrag { paths, .. }] if *paths == dragged
    ));
    let uris = dragged
        .iter()
        .map(|path| file_uri(path))
        .collect::<Vec<_>>();
    assert_eq!(view.drag_entered(row(1.0), &uris), None);
    assert_eq!(view.drag_moved(row(0.0)), Some(DragAction::Move));
    view.drag_finished();

    let uris = vec![file_uri(&place.home().join("notes.txt"))];
    assert!(view.dropped(row(0.0), &uris, Some(DragAction::Move)));
    run_until(&mut view, idle);
    assert!(place.home().join("Documents/notes.txt").exists());
    assert!(!place.home().join("notes.txt").exists());
}

#[test]
fn dragging_a_group_of_rows_in_the_list_view_drags_all_of_them() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&command(Key::Character('2'), false));
    view.scene(1000, 600);
    let row = |index: f32| {
        Point::new(
            SIDEBAR_WIDTH + 60.0,
            TOOLBAR_HEIGHT + Table::HEADER_HEIGHT + LIST_ROW * (index + 0.5),
        )
    };
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    for (index, modifiers) in [(1.0, Modifiers::default()), (2.0, shift)] {
        let pointer = Pointer::at(row(index)).with_modifiers(modifiers);
        view.handle(&Event::PointerDown(pointer));
        view.handle(&Event::PointerUp(pointer));
    }
    view.scene(1000, 600);

    view.handle(&Event::PointerDown(Pointer::at(row(1.0))));
    view.handle(&Event::PointerMove(Pointer::at(Point::new(
        SIDEBAR_WIDTH + 90.0,
        row(1.0).y + 30.0,
    ))));

    let dragged = vec![place.home().join("Empty"), place.home().join("notes.txt")];
    let requests = view.take_requests();
    let [Request::StartDrag { paths, grab }] = requests.as_slice() else {
        panic!("one drag starts: {requests:?}");
    };
    assert_eq!(*paths, dragged);
    let icon = view.drag_icon(*grab);
    assert!(
        icon.hotspot.x > 20.0 && icon.hotspot.x <= 40.0,
        "grabbed by the name, the icon sits left of the pointer: {:?}",
        icon.hotspot
    );
}

fn names_of(entries: Vec<&bande_fs::Entry>) -> Vec<String> {
    entries
        .into_iter()
        .map(|entry| entry.name.clone())
        .collect()
}

fn shift(key: Key) -> Event {
    Event::Key {
        key,
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    }
}

#[test]
fn the_icon_view_moves_in_two_dimensions_and_is_remembered_per_folder() {
    let place = Place::new();
    for index in 1..=10 {
        fs::write(place.home().join(format!("f{index:02}.txt")), "x").expect("file");
    }
    let memory = place.root.path().join("state/files-views");
    let mut view = place.view();
    view.set_view_memory(ViewMemory::load(memory.clone()));
    view.handle(&command(Key::Character('1'), false));
    assert_eq!(view.mode(), ViewMode::Icons);
    view.scene(1000, 600);

    let selected = |view: &FilesView| names_of(view.browser().selection());
    view.handle(&press(Key::Right));
    assert_eq!(selected(&view), ["Documents"]);
    view.handle(&press(Key::Down));
    assert_eq!(selected(&view), ["f06.txt"]);
    view.handle(&press(Key::Right));
    view.handle(&press(Key::Down));
    assert_eq!(selected(&view), ["f07.txt"]);
    view.handle(&press(Key::Up));
    assert_eq!(selected(&view), ["Empty"]);
    view.handle(&shift(Key::Right));
    assert_eq!(selected(&view), ["Empty", "f01.txt"]);

    view.handle(&press(Key::Home));
    view.handle(&press(Key::Enter));
    assert_eq!(
        view.browser().root(),
        &Location::Folder(place.home().join("Documents"))
    );
    assert_eq!(view.mode(), ViewMode::Icons);
    view.handle(&command(Key::Character('3'), false));
    view.handle(&command(Key::Character('['), false));
    assert_eq!(view.browser().root(), &Location::Folder(place.home()));
    assert_eq!(view.mode(), ViewMode::Icons);
    view.handle(&command(Key::Character('='), false));
    assert_eq!(view.icon_size(), 80);

    let remembered = ViewMemory::load(memory);
    let home = remembered.get(&place.home()).expect("home is remembered");
    assert_eq!((home.mode, home.icon_size), (ViewMode::Icons, 80));
    let documents = remembered
        .get(&place.home().join("Documents"))
        .expect("documents are remembered");
    assert_eq!(documents.mode, ViewMode::Columns);
}

#[test]
fn dragging_on_empty_space_in_the_icon_view_selects_the_icons_it_touches() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&command(Key::Character('1'), false));
    view.scene(1000, 600);

    view.handle(&Event::PointerDown(Pointer::at(Point::new(
        SIDEBAR_WIDTH + 4.0,
        TOOLBAR_HEIGHT + 4.0,
    ))));
    view.handle(&Event::PointerMove(Pointer::at(Point::new(
        SIDEBAR_WIDTH + 200.0,
        TOOLBAR_HEIGHT + 60.0,
    ))));
    assert_eq!(names_of(view.browser().selection()), ["Documents", "Empty"]);
    view.handle(&Event::PointerMove(Pointer::at(Point::new(
        SIDEBAR_WIDTH + 100.0,
        TOOLBAR_HEIGHT + 60.0,
    ))));
    assert_eq!(names_of(view.browser().selection()), ["Documents"]);
    view.handle(&Event::PointerUp(Pointer::at(Point::new(
        SIDEBAR_WIDTH + 100.0,
        TOOLBAR_HEIGHT + 60.0,
    ))));

    view.handle(&Event::PointerDown(
        Pointer::at(Point::new(SIDEBAR_WIDTH + 400.0, 400.0)).with_button(PointerButton::Secondary),
    ));
    assert!(matches!(
        view.take_requests().as_slice(),
        [Request::ShowMenu { .. }]
    ));
}

const GOLDEN_SIZE: (u32, u32) = (900, 560);
const LONG_NAME: &str = "quarterly-report-final-version-2-for-the-board.pdf";

fn check_golden(name: &str, rgba: Vec<u8>) {
    let whole = bande_golden::from_rgba(GOLDEN_SIZE.0, GOLDEN_SIZE.1, rgba);
    let steady = GOLDEN_SIZE.1 - (PATH_BAR_HEIGHT + SHEET_INSET as f32).round() as u32;
    let steady = image::imageops::crop_imm(&whole, 0, 0, GOLDEN_SIZE.0, steady).to_image();
    bande_golden::assert_golden!(name, &steady);
}

impl Place {
    fn themed_view(&self) -> FilesView {
        let apps = self.apps();
        let mime = self.root.path().join("data/mime");
        for (mime_type, comment) in [
            ("inode/directory", "Folder"),
            ("text/plain", "Plain text document"),
            ("image/png", "PNG image"),
        ] {
            let file = mime.join(format!("{mime_type}.xml"));
            fs::create_dir_all(file.parent().expect("type folder")).expect("mime folder");
            fs::write(
                file,
                format!("<mime-type type=\"{mime_type}\"><comment>{comment}</comment></mime-type>"),
            )
            .expect("mime description");
        }
        let icons = self.root.path().join("data/icons");
        fs::create_dir_all(&icons).expect("icons");
        let theme = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data/icons/Bande");
        if !icons.join("Bande").exists() {
            std::os::unix::fs::symlink(theme, icons.join("Bande")).expect("icon theme");
        }
        let mut view = FilesView::new(
            self.browser(),
            rows(&self.home(), None, None, None),
            apps,
            Palette::for_scheme(Scheme::Light),
            TextSystem::new(),
        )
        .expect("view");
        view.set_thumbnails(
            Thumbnails::spawn(
                Cache::new(self.root.path().join("cache")),
                Thumbnailers::default(),
            )
            .expect("thumbnails"),
        );
        view
    }
}

fn images(items: &[Item], found: &mut Vec<Image>) {
    for item in items {
        match item {
            Item::Image { image, .. } => found.push(image.clone()),
            Item::Clip { items, .. } | Item::Opacity { items, .. } => images(items, found),
            _ => {}
        }
    }
}

fn settle_thumbnails(view: &mut FilesView, size: (u32, u32), thumbnails: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let scene = view.scene(size.0, size.1);
        let mut found = Vec::new();
        images(&scene.items, &mut found);
        if found
            .iter()
            .filter(|image| image.width() != image.height())
            .count()
            >= thumbnails
        {
            return;
        }
        assert!(Instant::now() < deadline, "the thumbnails did not arrive");
        std::thread::sleep(Duration::from_millis(20));
        for index in (0..view.fds().len()).rev() {
            view.source_ready(index);
        }
    }
}

fn picture(path: &Path, width: u32, height: u32) {
    image::RgbaImage::from_fn(width, height, |x, y| match y < height * 3 / 5 {
        true => image::Rgba([90, 150 + (y * 60 / height) as u8, 230, 255]),
        false => image::Rgba([60 + (x * 80 / width) as u8, 150, 70, 255]),
    })
    .save(path)
    .expect("picture");
}

#[test]
fn the_icon_view_matches_its_golden_with_thumbnails_and_long_names() {
    let place = Place::new();
    picture(&place.home().join("Holiday.png"), 640, 400);
    picture(&place.home().join("Portrait.png"), 300, 450);
    fs::write(
        place
            .home()
            .join("A very long document name that needs two lines to show.txt"),
        "long",
    )
    .expect("file");
    let mut view = place.themed_view();
    view.handle(&command(Key::Character('1'), false));
    view.scene(GOLDEN_SIZE.0, GOLDEN_SIZE.1);
    view.handle(&press(Key::Right));
    view.handle(&press(Key::Right));
    view.handle(&press(Key::Right));
    settle_thumbnails(&mut view, GOLDEN_SIZE, 2);
    shoot("icons", &mut view);
    view.set_window_active(false);
    shoot("icons-inactive", &mut view);
}

fn shoot(name: &str, view: &mut FilesView) {
    for (scheme, suffix) in [(Scheme::Light, ""), (Scheme::Dark, "-dark")] {
        let palette = Palette::for_scheme(scheme);
        view.set_palette(palette);
        let scene = view.scene(GOLDEN_SIZE.0, GOLDEN_SIZE.1);
        let scene = framed_scene(
            &scene,
            &view.title(),
            Some(FrameRegions {
                sidebar: SIDEBAR_WIDTH,
                toolbar: 0.0,
            }),
            Some(view.header()),
            &palette,
        );
        let frame = CpuRenderer::new().render(&scene).expect("render");
        check_golden(&format!("{name}{suffix}"), frame.rgba);
    }
}

#[test]
fn the_list_view_with_tabs_matches_its_golden() {
    let place = Place::new();
    fs::write(place.home().join("Budget.txt"), "x".repeat(48_000)).expect("file");
    let noon = std::time::UNIX_EPOCH + Duration::from_secs(1_741_953_600);
    for entry in fs::read_dir(place.home()).expect("home") {
        fs::File::open(entry.expect("entry").path())
            .and_then(|file| file.set_modified(noon))
            .expect("set the date");
    }
    let mut view = place.themed_view();
    view.handle(&command(Key::Character('2'), false));
    view.handle(&command(Key::Character('t'), false));
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Down));
    view.scene(GOLDEN_SIZE.0, GOLDEN_SIZE.1);
    view.handle(&Event::PointerMove(Pointer::at(Point::new(
        SIDEBAR_WIDTH + 100.0,
        TOOLBAR_HEIGHT + TAB_BAR_HEIGHT / 2.0,
    ))));
    shoot("tabs-list", &mut view);
}

#[test]
fn the_column_view_matches_its_golden() {
    let place = Place::new();
    date_files(&[place.home().join("Documents/a.txt")]);
    fs::write(place.home().join("Documents").join(LONG_NAME), "report\n").expect("file");
    let mut view = place.themed_view();
    view.handle(&command(Key::Character('3'), false));
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Right));
    view.handle(&press(Key::Down));
    run_until(&mut view, previewed);
    shoot("columns", &mut view);
}

#[test]
fn tabs_keep_their_own_folders_and_close_back_to_one() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&command(Key::Character('t'), false));
    assert_eq!((view.tab_count(), view.active_tab()), (2, 1));
    view.handle(&press(Key::Down));
    view.handle(&command(Key::Down, false));
    assert_eq!(
        view.browser().root(),
        &Location::Folder(place.home().join("Documents"))
    );
    assert_eq!(view.tab_titles(), ["home", "Documents"]);
    assert_eq!(view.title(), "Documents");

    view.handle(&command(Key::Character('['), true));
    assert_eq!(view.active_tab(), 0);
    assert_eq!(view.browser().root(), &Location::Folder(place.home()));
    view.handle(&command(Key::Character(']'), true));
    assert_eq!(view.active_tab(), 1);

    view.scene(1000, 600);
    view.handle(&Event::PointerDown(
        Pointer::at(Point::new(
            SIDEBAR_WIDTH + 100.0,
            TAB_BAR_HEIGHT + TOOLBAR_HEIGHT + 6.0 + ROW_HEIGHT * 0.5,
        ))
        .with_button(PointerButton::Middle),
    ));
    assert_eq!(view.tab_count(), 3);
    assert_eq!(view.tab_titles(), ["home", "Documents", "Reports"]);

    assert!(view.close_tab(view.active_tab()));
    assert_eq!(view.tab_titles(), ["home", "Documents"]);
    assert_eq!(view.title(), "Documents");
    assert!(view.close_tab(0));
    assert_eq!(view.tab_count(), 1);
    assert!(!view.close_tab(0));
    assert_eq!(
        view.browser().root(),
        &Location::Folder(place.home().join("Documents"))
    );
}

#[test]
fn files_dropped_on_a_tab_move_into_its_folder_and_hovering_springs_it_open() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&press(Key::Down));
    assert_eq!(
        titles(&right_click(
            &mut view,
            Point::new(
                SIDEBAR_WIDTH + 60.0,
                TOOLBAR_HEIGHT + 6.0 + ROW_HEIGHT * 0.5
            )
        ))[..2],
        ["Open", "Open in New Tab"]
    );
    assert!(view.menu_action("open-in-new-tab"));
    assert_eq!(view.tab_titles(), ["home", "Documents"]);
    view.scene(1000, 600);

    let notes = place.home().join("notes.txt");
    let uris = [file_uri(&notes)];
    let over_documents = Point::new(SIDEBAR_WIDTH + 600.0, TOOLBAR_HEIGHT + TAB_BAR_HEIGHT / 2.0);
    let over_home = Point::new(SIDEBAR_WIDTH + 200.0, TOOLBAR_HEIGHT + TAB_BAR_HEIGHT / 2.0);
    assert_eq!(
        view.drag_entered(over_documents, &uris),
        Some(DragAction::Move)
    );
    view.handle(&command(Key::Character('['), true));
    view.scene(1000, 600);
    assert_eq!(view.drag_moved(over_documents), Some(DragAction::Move));
    assert!(view.dropped(over_documents, &uris, Some(DragAction::Move)));
    run_until(&mut view, idle);
    assert!(place.home().join("Documents/notes.txt").exists());

    let uris = [file_uri(&place.home().join("Empty"))];
    view.drag_entered(over_home, &uris);
    assert_eq!(view.active_tab(), 0);
    view.drag_moved(over_documents);
    view.tick(Instant::now() + Duration::from_secs(2));
    assert_eq!(view.active_tab(), 1);
}

fn date_everything(folder: &Path) {
    let noon = std::time::UNIX_EPOCH + Duration::from_secs(1_741_953_600);
    for entry in fs::read_dir(folder).expect("folder") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            date_everything(&path);
        }
        fs::File::open(&path)
            .and_then(|file| file.set_modified(noon))
            .expect("set the date");
    }
}

#[test]
fn search_results_match_their_golden() {
    let place = Place::new();
    fs::write(place.home().join("Documents/Reports/q3 report.txt"), "q3").expect("file");
    fs::write(place.home().join("Report card.txt"), "card").expect("file");
    date_everything(&place.home());
    let mut view = place.themed_view();
    view.scene(GOLDEN_SIZE.0, GOLDEN_SIZE.1);
    view.handle(&command(Key::Character('f'), false));
    type_text(&mut view, "report");
    run_until(&mut view, |view| shown(view).len() == 3);
    view.handle(&press(Key::Down));
    settle(&mut view);
    shoot("search", &mut view);
}

#[test]
fn compressing_makes_an_undoable_zip_and_opening_it_extracts_beside_it() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&command(Key::Character('2'), false));
    view.scene(1000, 600);
    view.handle(&press(Key::End));
    assert_eq!(selected_name(view.browser()).as_deref(), Some("notes.txt"));
    let menu = right_click(
        &mut view,
        Point::new(
            SIDEBAR_WIDTH + 60.0,
            TOOLBAR_HEIGHT + Table::HEADER_HEIGHT + LIST_ROW * 2.5,
        ),
    );
    assert!(titles(&menu).contains(&"Compress “notes.txt”".to_owned()));
    assert!(view.menu_action("compress"));
    run_until(&mut view, idle);
    let archive = place.home().join("notes.txt.zip");
    assert!(archive.exists());
    assert_eq!(
        selected_name(view.browser()).as_deref(),
        Some("notes.txt.zip")
    );

    view.handle(&command(Key::Down, false));
    run_until(&mut view, idle);
    let extracted = place.home().join("notes 2.txt");
    assert_eq!(
        fs::read_to_string(&extracted).expect("extracted"),
        "notes\n"
    );
    assert_eq!(
        selected_name(view.browser()).as_deref(),
        Some("notes 2.txt")
    );
    assert!(view.take_requests().is_empty());

    view.handle(&command(Key::Character('z'), false));
    run_until(&mut view, idle);
    assert!(!extracted.exists());
    view.handle(&command(Key::Character('z'), false));
    run_until(&mut view, idle);
    assert!(!archive.exists());
}

#[test]
fn quick_look_shows_documents_through_installed_thumbnailers() {
    let place = Place::new();
    let sample = place.root.path().join("page.png");
    picture(&sample, 600, 800);
    let script = place.root.path().join("pdf-thumbnailer");
    fs::write(
        &script,
        format!("#!/bin/sh\ncp {} \"$2\"\n", sample.display()),
    )
    .expect("script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("executable");
    let thumbnailers = place.root.path().join("thumbnailers-data");
    fs::create_dir_all(thumbnailers.join("thumbnailers")).expect("folder");
    fs::write(
        thumbnailers.join("thumbnailers/pdf.thumbnailer"),
        format!(
            "[Thumbnailer Entry]\nExec={} %i %o\nMimeType=application/pdf;\n",
            script.display()
        ),
    )
    .expect("thumbnailer");
    fs::write(place.home().join("paper.pdf"), b"%PDF-1.7\n\0binary").expect("pdf");
    let mut view = place.view();
    view.set_thumbnails(
        Thumbnails::spawn(
            Cache::new(place.root.path().join("cache")),
            Thumbnailers::load(&[thumbnailers]),
        )
        .expect("thumbnails"),
    );
    view.handle(&press(Key::Up));
    assert_eq!(selected_name(view.browser()).as_deref(), Some("paper.pdf"));

    view.handle(&press(Key::Space));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let scene = view.scene(1000, 600);
        let mut found = Vec::new();
        images(&scene.items, &mut found);
        if found
            .iter()
            .any(|image| (image.width(), image.height()) == (384, 512))
        {
            break;
        }
        assert!(Instant::now() < deadline, "the page did not arrive");
        std::thread::sleep(Duration::from_millis(20));
        for index in (0..view.fds().len()).rev() {
            view.source_ready(index);
        }
    }

    view.handle(&press(Key::Up));
    assert_eq!(
        view.quick_look().map(|shown| shown.entry.name.as_str()),
        Some("notes.txt")
    );
}

#[test]
fn bookmarks_are_added_before_a_favourite_or_moved_keeping_their_label() {
    let text = "file:///srv/a A\nfile:///srv/b\nsftp://host/x\n";
    assert_eq!(
        with_bookmarks(text, &[PathBuf::from("/srv/new")], None),
        "file:///srv/a A\nfile:///srv/b\nsftp://host/x\nfile:///srv/new\n"
    );
    assert_eq!(
        with_bookmarks(text, &[PathBuf::from("/srv/b")], Some(Path::new("/srv/a"))),
        "file:///srv/b\nfile:///srv/a A\nsftp://host/x\n"
    );
    assert_eq!(
        with_bookmarks(text, &[PathBuf::from("/srv/a")], None),
        "file:///srv/b\nsftp://host/x\nfile:///srv/a A\n"
    );
    assert_eq!(
        without_bookmark(text, Path::new("/srv/a")),
        "file:///srv/b\nsftp://host/x\n"
    );
}

#[test]
fn folders_dropped_between_favourites_and_dragged_places_join_the_sidebar() {
    let place = Place::new();
    let home = place.home();
    let reports = home.join("Documents/Reports");
    let empty = home.join("Empty");
    let bookmarks = format!("file://{}\n", reports.display());
    let mut view = place.view();
    view.set_sidebar(rows(&home, None, Some(&bookmarks), None));
    view.scene(1000, 600);
    let row_top = |row: f32| 84.0 + 30.0 * row;
    let lines = |lines: &[String]| lines.iter().map(|line| format!("{line}\n")).collect();

    let uris = [file_uri(&empty)];
    let before_documents = Point::new(60.0, row_top(2.0));
    assert_eq!(
        view.drag_entered(before_documents, &uris),
        Some(DragAction::Copy)
    );
    assert!(view.dropped(before_documents, &uris, Some(DragAction::Copy)));
    assert_eq!(
        view.take_requests(),
        [Request::EditSidebar(SidebarEdit {
            layout: Some(lines(&[
                "recents".to_owned(),
                file_uri(&home),
                file_uri(&empty),
                file_uri(&home.join("Documents")),
                file_uri(&reports),
            ])),
            bookmarks: Some(BookmarkEdit::Add(vec![empty.clone()])),
        })]
    );
    let notes = [file_uri(&home.join("notes.txt"))];
    view.drag_entered(before_documents, &notes);
    assert!(view.dropped(before_documents, &notes, Some(DragAction::Move)));
    run_until(&mut view, idle);
    assert!(home.join("Documents/notes.txt").exists());
    view.take_requests();

    let on_recents = Point::new(60.0, row_top(0.5));
    let below_the_last = Point::new(60.0, row_top(4.0) + 4.0);
    view.handle(&Event::PointerDown(Pointer::at(on_recents)));
    view.handle(&Event::PointerMove(Pointer::at(below_the_last)));
    view.handle(&Event::PointerUp(Pointer::at(below_the_last)));
    assert_eq!(
        view.take_requests(),
        [Request::EditSidebar(SidebarEdit {
            layout: Some(lines(&[
                file_uri(&home),
                file_uri(&home.join("Documents")),
                file_uri(&reports),
                "recents".to_owned(),
            ])),
            bookmarks: None,
        })]
    );
}

#[test]
fn templates_make_new_documents_and_folders_open_in_the_terminal() {
    let place = Place::new();
    let templates = place.root.path().join("Templates");
    fs::create_dir(&templates).expect("templates");
    fs::write(templates.join("Letter.txt"), "Dear\n").expect("template");
    let mut view = place.view();
    view.set_templates(templates);

    let menu = right_click(&mut view, Point::new(SIDEBAR_WIDTH + 60.0, 400.0));
    let titles = titles(&menu);
    assert_eq!(titles[..2], ["New Folder", "New Document"]);
    assert!(titles.contains(&"Open in Terminal".to_owned()));
    assert!(view.menu_action("open-in-terminal"));
    assert_eq!(view.take_requests(), [Request::OpenTerminal(place.home())]);

    assert!(view.menu_action("new-document:Letter.txt"));
    let letter = place.home().join("Letter.txt");
    assert_eq!(fs::read_to_string(&letter).expect("letter"), "Dear\n");
    assert_eq!(view.renaming(), Some(letter.as_path()));
    view.handle(&press(Key::Escape));
    view.handle(&command(Key::Character('z'), false));
    run_until(&mut view, idle);
    assert!(!letter.exists());
}

#[test]
fn hiding_the_status_bar_is_remembered_for_the_next_window() {
    let place = Place::new();
    let file = place.root.path().join("state/bande/files-status-bar");
    let mut view = place.view();
    view.set_status_bar_file(file.clone());
    let with_bar = view.content_frame().height;

    view.handle(&command(Key::Character('/'), false));
    let mut next = place.view();
    next.set_status_bar_file(file.clone());
    assert_eq!(next.content_frame().height, with_bar + PATH_BAR_HEIGHT);

    next.handle(&command(Key::Character('/'), false));
    let mut third = place.view();
    third.set_status_bar_file(file);
    assert_eq!(third.content_frame().height, with_bar);
}

const BIG: usize = 20_000;

fn big_folder(place: &Place) -> PathBuf {
    let folder = place.home().join("Big");
    fs::create_dir(&folder).expect("big folder");
    for index in 0..BIG {
        fs::File::create(folder.join(format!("File {index:05}"))).expect("file");
    }
    folder
}

fn pump(browser: &mut Browser) -> bool {
    let fd = browser.listing_fd().expect("a lister");
    let timeout = Timespec {
        tv_sec: 0,
        tv_nsec: 200_000_000,
    };
    let ready = poll(&mut [PollFd::new(&fd, PollFlags::IN)], Some(&timeout)).expect("poll") > 0;
    ready && browser.take_listed().0
}

fn listed(browser: &mut Browser) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while browser.columns().iter().any(|column| column.is_listing()) {
        assert!(Instant::now() < deadline, "the listing did not finish");
        pump(browser);
    }
}

#[test]
fn a_big_folder_shows_its_first_items_at_once_and_fills_in_sorted() {
    let place = Place::new();
    let folder = big_folder(&place);
    let mut browser = place.browser();

    browser.go(Location::Folder(folder.clone()));
    let first = browser.columns()[0].entries.len();
    assert!(first > 0 && first < BIG, "{first} items at once");
    assert!(browser.columns()[0].is_listing());

    listed(&mut browser);
    let shown = browser.columns()[0]
        .entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(shown, names(&folder, false));
}

#[test]
fn going_back_while_a_big_folder_lists_drops_its_late_items() {
    let place = Place::new();
    let folder = big_folder(&place);
    let mut browser = place.browser();
    browser.go(Location::Folder(folder));

    assert!(browser.go_back());
    while pump(&mut browser) {}

    assert_eq!(browser.root(), &Location::Folder(place.home()));
    let shown = browser.columns()[0]
        .entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(shown, names(&place.home(), false));
    assert_eq!(selected_name(&browser).as_deref(), Some("Big"));
}

#[test]
fn a_file_revealed_in_a_big_folder_is_selected_once_it_arrives() {
    let place = Place::new();
    let folder = big_folder(&place);
    let mut browser = place.browser();
    let last = folder.join(format!("File {:05}", BIG - 1));

    browser.reveal(&last);
    listed(&mut browser);

    assert_eq!(browser.selected().map(|entry| &entry.path), Some(&last));
    assert_eq!(browser.columns()[0].marked.len(), 1);
    assert_eq!(browser.columns()[0].selected, Some(BIG - 1));
}

#[test]
fn go_to_folder_on_a_file_in_a_big_folder_selects_it() {
    let place = Place::new();
    let folder = big_folder(&place);
    let mut view = place.view();
    let wanted = folder.join("File 12345");

    view.handle(&command(Key::Character('G'), true));
    view.handle(&Event::Commit(wanted.display().to_string()));
    view.handle(&press(Key::Enter));
    run_until(&mut view, |view| {
        view.browser().selected().map(|entry| &entry.path) == Some(&wanted)
    });

    run_until(&mut view, |view| !view.browser().columns()[0].is_listing());
    assert_eq!(
        view.browser().selected().map(|entry| &entry.path),
        Some(&wanted)
    );
    assert_eq!(view.browser().selection().len(), 1);
}

fn shown_names(browser: &Browser) -> Vec<String> {
    browser.columns()[0]
        .entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect()
}

#[test]
fn changes_in_a_big_folder_move_only_their_rows_and_keep_the_selection() {
    let place = Place::new();
    let folder = big_folder(&place);
    let mut browser = place.browser();
    browser.go(Location::Folder(folder.clone()));
    listed(&mut browser);
    browser.select(0, 100);
    let change = |browser: &mut Browser, paths: &[&str]| {
        browser.apply(&FolderChanges {
            folders: Vec::new(),
            entries: paths.iter().map(|name| folder.join(name)).collect(),
        });
    };

    fs::File::create(folder.join("File 00050a")).expect("file");
    fs::File::create(folder.join("Unreported")).expect("file");
    change(&mut browser, &["File 00050a"]);
    assert_eq!(shown_names(&browser)[51], "File 00050a");
    assert_eq!(browser.columns()[0].entries.len(), BIG + 1);
    assert_eq!(selected_name(&browser).as_deref(), Some("File 00100"));
    assert_eq!(browser.columns()[0].selected, Some(101));

    fs::remove_file(folder.join("File 00010")).expect("remove");
    change(&mut browser, &["File 00010"]);
    assert_eq!(browser.columns()[0].selected, Some(100));

    fs::rename(folder.join("File 00200"), folder.join("Aardvark")).expect("rename");
    change(&mut browser, &["File 00200", "Aardvark"]);
    let names = shown_names(&browser);
    assert_eq!(names[0], "Aardvark");
    assert!(!names.contains(&"File 00200".to_owned()));
    assert!(!names.contains(&"Unreported".to_owned()));
    assert_eq!(selected_name(&browser).as_deref(), Some("File 00100"));
    assert_eq!(browser.columns()[0].marked, [101]);
}

#[test]
fn a_change_to_the_folder_itself_reads_it_again() {
    let place = Place::new();
    let mut browser = place.browser();
    fs::write(place.home().join("unreported.txt"), "x").expect("file");

    browser.apply(&FolderChanges {
        folders: vec![place.home()],
        entries: Vec::new(),
    });

    assert!(shown_names(&browser).contains(&"unreported.txt".to_owned()));
}

#[test]
fn a_file_written_into_a_shown_big_folder_appears_in_place() {
    let place = Place::new();
    let folder = big_folder(&place);
    let mut view = place.view();
    view.handle(&command(Key::Character('2'), false));
    view.handle(&command(Key::Character('G'), true));
    view.handle(&Event::Commit(folder.display().to_string()));
    view.handle(&press(Key::Enter));
    run_until(&mut view, |view| !view.browser().columns()[0].is_listing());
    view.handle(&command(Key::Character('a'), false));
    assert_eq!(view.browser().selection().len(), BIG);

    fs::write(folder.join("File 00000a"), "new").expect("file");
    run_until(&mut view, |view| {
        view.browser().columns()[0].entries.len() == BIG + 1
    });

    assert_eq!(view.browser().columns()[0].entries[1].name, "File 00000a");
    assert_eq!(view.browser().selection().len(), BIG);
    assert!(!view.browser().columns()[0].is_marked(1));
}

#[test]
fn select_all_then_shift_click_in_a_big_folder_keeps_the_right_marks() {
    let place = Place::new();
    let folder = big_folder(&place);
    let mut browser = place.browser();
    browser.go(Location::Folder(folder));
    listed(&mut browser);

    browser.select_all();
    assert_eq!(browser.columns()[0].marked.len(), BIG);
    browser.extend_to(0, 5);

    assert_eq!(browser.columns()[0].marked, (0..=5).collect::<Vec<_>>());
    assert!(browser.columns()[0].is_marked(5));
    assert!(!browser.columns()[0].is_marked(6));
}

#[test]
fn sorting_a_big_folder_by_size_after_select_all_keeps_every_mark() {
    let place = Place::new();
    let folder = big_folder(&place);
    fs::write(folder.join("File 00007"), "the largest file").expect("file");
    let mut browser = place.browser();
    browser.go(Location::Folder(folder.clone()));
    listed(&mut browser);
    browser.select_all();

    browser.set_sort(Sort {
        by: SortBy::Size,
        descending: false,
    });

    let column = &browser.columns()[0];
    assert_eq!(column.marked.len(), BIG);
    assert_eq!(column.entries[0].name, "File 00007");
    assert!(!column.is_listing());
}

#[test]
fn select_all_in_the_list_view_of_a_big_folder_marks_every_row_once() {
    let place = Place::new();
    let folder = big_folder(&place);
    let mut view = place.view();
    view.handle(&command(Key::Character('2'), false));
    view.handle(&command(Key::Character('G'), true));
    view.handle(&Event::Commit(folder.display().to_string()));
    view.handle(&press(Key::Enter));
    run_until(&mut view, |view| !view.browser().columns()[0].is_listing());

    view.handle(&command(Key::Character('a'), false));
    assert_eq!(view.browser().selection().len(), BIG);

    view.handle(&press(Key::Up));
    assert_eq!(view.browser().selection().len(), 1);
}

fn date_files(paths: &[PathBuf]) {
    let noon = std::time::UNIX_EPOCH + Duration::from_secs(1_741_953_600);
    for path in paths {
        fs::File::open(path)
            .and_then(|file| file.set_modified(noon))
            .expect("set the date");
    }
}

fn previewed(view: &FilesView) -> bool {
    view.column_preview()
        .is_some_and(|shown| shown.preview.is_some())
}

#[test]
fn a_selected_file_gets_a_preview_column_with_its_kind_and_size() {
    let place = Place::new();
    let mut view = place.view();

    view.handle(&press(Key::Down));
    assert!(view.column_preview().is_none());
    assert_eq!(view.browser().columns().len(), 2);

    view.handle(&press(Key::Down));
    view.handle(&press(Key::Down));
    run_until(&mut view, previewed);
    let shown = view.column_preview().expect("a preview column");
    let entry = shown.entry.as_ref().expect("one file");
    assert_eq!(entry.name, "notes.txt");
    assert_eq!(entry.size, 6);
    assert!(!shown.kind.is_empty());
    assert!(matches!(&shown.preview, Some(Preview::Text(text)) if text == "notes\n"));
    assert_eq!(view.browser().columns().len(), 1);
}

#[test]
fn several_selected_files_preview_as_a_count_and_total_size() {
    let place = Place::new();
    fs::write(place.home().join("other.txt"), "four").expect("file");
    let mut view = place.view();
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Down));

    view.handle(&Event::Key {
        key: Key::Down,
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    });

    let shown = view.column_preview().expect("a preview column");
    assert!(shown.entry.is_none());
    assert_eq!((shown.count, shown.bytes), (2, 10));
}

#[test]
fn left_from_a_previewed_file_returns_to_its_folder() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Right));
    view.handle(&press(Key::Down));
    assert!(view.column_preview().is_some());

    view.handle(&press(Key::Right));
    assert_eq!(selected_name(view.browser()).as_deref(), Some("a.txt"));
    assert!(view.column_preview().is_some());

    view.handle(&press(Key::Left));
    assert_eq!(selected_name(view.browser()).as_deref(), Some("Documents"));
    assert!(view.column_preview().is_none());
}

#[test]
fn column_rows_are_thirty_two_points_apart() {
    let place = Place::new();
    for name in ["b.txt", "c.txt", "d.txt"] {
        fs::write(place.home().join(name), "x").expect("file");
    }
    let mut view = place.view();
    let at = Point::new(
        SIDEBAR_WIDTH + 60.0,
        TOOLBAR_HEIGHT + 6.0 + 32.0 * 3.0 + 26.0,
    );

    view.handle(&Event::PointerDown(Pointer::at(at)));
    view.handle(&Event::PointerUp(Pointer::at(at)));

    assert_eq!(selected_name(view.browser()).as_deref(), Some("c.txt"));
}

#[test]
fn the_preview_column_matches_its_golden() {
    let place = Place::new();
    picture(&place.home().join("Holiday.png"), 640, 400);
    date_files(&[place.home().join("Holiday.png")]);
    let mut view = place.themed_view();
    view.handle(&command(Key::Character('3'), false));
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Down));
    run_until(&mut view, previewed);
    assert_eq!(
        selected_name(view.browser()).as_deref(),
        Some("Holiday.png")
    );
    shoot("columns-preview", &mut view);
}

#[test]
fn sorting_by_kind_groups_kinds_then_names() {
    let place = Place::new();
    for name in ["z.png", "a.txt", "b.png"] {
        fs::write(place.home().join(name), "x").expect("file");
    }
    let mut browser = place.browser();

    browser.set_sort(Sort {
        by: SortBy::Kind,
        descending: false,
    });

    assert_eq!(
        shown_names(&browser),
        ["Documents", "Empty", "b.png", "z.png", "a.txt", "notes.txt"]
    );
}

#[test]
fn each_folder_keeps_its_own_sort_across_restarts() {
    let place = Place::new();
    let downloads = place.home().join("Downloads");
    fs::create_dir(&downloads).expect("downloads");
    let memory = place.root.path().join("state/files-views");
    let go = |view: &mut FilesView, folder: &Path| {
        view.handle(&command(Key::Character('G'), true));
        view.handle(&Event::Commit(folder.display().to_string()));
        view.handle(&press(Key::Enter));
    };
    let mut view = place.view();
    view.set_view_memory(ViewMemory::load(memory.clone()));
    view.handle(&command(Key::Character('2'), false));
    go(&mut view, &downloads);
    assert!(view.menu_action("sort:modified"));
    go(&mut view, &place.home().join("Documents"));
    assert!(view.menu_action("sort:name"));

    let mut view = place.view();
    view.set_view_memory(ViewMemory::load(memory));
    go(&mut view, &downloads);
    assert_eq!(view.browser().sort().by, SortBy::Modified);
    go(&mut view, &place.home().join("Documents"));
    assert_eq!(view.browser().sort().by, SortBy::Name);
}

#[test]
fn a_hidden_list_column_stays_hidden_in_a_new_window() {
    let place = Place::new();
    let file = place.root.path().join("state/files-columns");
    let mut view = place.view();
    view.set_list_columns_file(file.clone());
    assert_eq!(view.list_columns(), ListColumn::DEFAULT);

    assert!(view.menu_action("column:size"));
    assert!(view.menu_action("column:created"));

    let mut again = place.view();
    again.set_list_columns_file(file);
    assert_eq!(
        again.list_columns(),
        [ListColumn::Modified, ListColumn::Created, ListColumn::Kind]
    );
}

#[test]
fn right_clicking_the_list_header_offers_the_columns() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&command(Key::Character('2'), false));
    view.scene(1000, 600);

    view.handle(&Event::PointerDown(
        Pointer::at(Point::new(
            SIDEBAR_WIDTH + 100.0,
            TOOLBAR_HEIGHT + Table::HEADER_HEIGHT / 2.0,
        ))
        .with_button(PointerButton::Secondary),
    ));

    let requests = view.take_requests();
    let Some(Request::ShowMenu { menu, .. }) = requests.first() else {
        panic!("no menu: {requests:?}");
    };
    let columns = menu
        .items
        .iter()
        .map(|item| (item.action.as_str(), item.checked))
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        [
            ("column:modified", true),
            ("column:created", false),
            ("column:size", true),
            ("column:kind", true),
        ]
    );
}

fn outline(view: &FilesView) -> Vec<(String, u8)> {
    let column = &view.browser().columns()[0];
    column
        .entries
        .iter()
        .enumerate()
        .map(|(row, entry)| {
            (
                entry.name.clone(),
                column.depths.get(row).copied().unwrap_or_default(),
            )
        })
        .collect()
}

fn list_row(index: f32) -> Point {
    Point::new(
        SIDEBAR_WIDTH + 60.0,
        TOOLBAR_HEIGHT + Table::HEADER_HEIGHT + LIST_ROW * (index + 0.5),
    )
}

fn expanded_documents(place: &Place) -> FilesView {
    let mut view = place.view();
    view.handle(&command(Key::Character('2'), false));
    view.scene(1000, 600);
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Right));
    view.scene(1000, 600);
    view
}

#[test]
fn right_on_a_folder_in_the_list_shows_its_children_and_left_hides_them() {
    let place = Place::new();
    let mut view = expanded_documents(&place);

    assert_eq!(
        outline(&view),
        [
            ("Documents".to_owned(), 0),
            ("Reports".to_owned(), 1),
            ("a.txt".to_owned(), 1),
            ("Empty".to_owned(), 0),
            ("notes.txt".to_owned(), 0),
        ]
    );
    assert_eq!(selected_name(view.browser()).as_deref(), Some("Documents"));

    view.handle(&press(Key::Left));
    assert_eq!(
        outline(&view),
        [
            ("Documents".to_owned(), 0),
            ("Empty".to_owned(), 0),
            ("notes.txt".to_owned(), 0),
        ]
    );
}

#[test]
fn a_new_file_in_an_expanded_folder_appears_without_collapsing_it() {
    let place = Place::new();
    let mut view = expanded_documents(&place);

    fs::write(place.home().join("Documents/b.txt"), "new").expect("file");
    run_until(&mut view, |view| {
        outline(view).contains(&("b.txt".to_owned(), 1))
    });

    assert_eq!(outline(&view)[3], ("b.txt".to_owned(), 1));
    assert_eq!(outline(&view)[4], ("Empty".to_owned(), 0));
}

#[test]
fn a_selection_across_levels_drags_all_its_paths() {
    let place = Place::new();
    let mut view = expanded_documents(&place);

    view.handle(&Event::PointerDown(Pointer::at(list_row(2.0))));
    view.handle(&Event::PointerUp(Pointer::at(list_row(2.0))));
    let logo = Modifiers {
        logo: true,
        ..Modifiers::default()
    };
    view.handle(&Event::PointerDown(
        Pointer::at(list_row(4.0)).with_modifiers(logo),
    ));
    view.handle(&Event::PointerUp(Pointer::at(list_row(4.0))));
    view.scene(1000, 600);
    view.handle(&Event::PointerDown(Pointer::at(list_row(4.0))));
    view.handle(&Event::PointerMove(Pointer::at(Point::new(
        SIDEBAR_WIDTH + 70.0,
        list_row(4.0).y + 12.0,
    ))));

    let dragged = vec![
        place.home().join("Documents/a.txt"),
        place.home().join("notes.txt"),
    ];
    assert!(matches!(
        view.take_requests().as_slice(),
        [Request::StartDrag { paths, .. }] if *paths == dragged
    ));
}

#[test]
fn the_list_with_an_open_folder_matches_its_golden() {
    let place = Place::new();
    fs::write(place.home().join(LONG_NAME), "report\n").expect("file");
    let noon = std::time::UNIX_EPOCH + Duration::from_secs(1_741_953_600);
    for path in [
        place.home().join("Documents"),
        place.home().join("Documents/Reports"),
        place.home().join("Documents/a.txt"),
        place.home().join("Empty"),
        place.home().join("notes.txt"),
        place.home().join(LONG_NAME),
    ] {
        fs::File::open(path)
            .and_then(|file| file.set_modified(noon))
            .expect("set the date");
    }
    let mut view = place.themed_view();
    view.handle(&command(Key::Character('2'), false));
    view.scene(GOLDEN_SIZE.0, GOLDEN_SIZE.1);
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Right));
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Down));
    shoot("list-expanded", &mut view);
}

fn alt_command(key: Key) -> Event {
    Event::Key {
        key,
        modifiers: Modifiers {
            logo: true,
            alt: true,
            ..Modifiers::default()
        },
    }
}

fn quick_look_position(view: &FilesView) -> Option<(String, Option<(usize, usize)>)> {
    view.quick_look()
        .map(|shown| (shown.entry.name.clone(), shown.position()))
}

#[test]
fn quick_look_steps_through_a_selection_without_changing_it() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&press(Key::Down));
    view.handle(&shift(Key::Down));
    view.handle(&shift(Key::Down));
    let selected = names_of(view.browser().selection());
    assert_eq!(selected.len(), 3);

    view.handle(&press(Key::Space));
    assert_eq!(
        quick_look_position(&view),
        Some(("Documents".to_owned(), Some((1, 3))))
    );
    view.handle(&press(Key::Right));
    assert_eq!(
        quick_look_position(&view),
        Some(("Empty".to_owned(), Some((2, 3))))
    );
    view.handle(&press(Key::Right));
    view.handle(&press(Key::Right));
    assert_eq!(
        quick_look_position(&view),
        Some(("notes.txt".to_owned(), Some((3, 3))))
    );
    view.handle(&press(Key::Left));
    assert_eq!(
        quick_look_position(&view),
        Some(("Empty".to_owned(), Some((2, 3))))
    );
    assert_eq!(names_of(view.browser().selection()), selected);
}

#[test]
fn quick_look_opens_its_item_with_the_default_app() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&press(Key::Up));
    view.handle(&command(Key::Character('y'), false));
    view.scene(1000, 600);
    let button = view
        .hit_frame(&bande_files::Hit::QuickLookOpen)
        .expect("an Open with button");

    view.handle(&Event::PointerDown(Pointer::at(Point::new(
        button.x + button.width / 2.0,
        button.y + button.height / 2.0,
    ))));

    let requests = view.take_requests();
    let [Request::Open { app, files }] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    assert_eq!(app.id, "editor.desktop");
    assert_eq!(files, &[place.home().join("notes.txt")]);
    assert!(view.quick_look().is_some());
}

#[test]
fn page_down_scrolls_a_long_text_in_quick_look() {
    let place = Place::new();
    let text = (1..=400)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    fs::write(place.home().join("notes.txt"), text).expect("long text");
    let mut view = place.view();
    view.handle(&press(Key::Up));
    view.handle(&press(Key::Space));
    run_until(&mut view, |view| {
        view.quick_look()
            .is_some_and(|shown| matches!(shown.preview, Some(Preview::Text(_))))
    });
    view.scene(1000, 600);

    assert!(view.handle(&press(Key::PageDown)));

    let deadline = Instant::now() + Duration::from_secs(5);
    while view.quick_look_scrolled() < 100.0 {
        assert!(Instant::now() < deadline, "the text did not scroll");
        std::thread::sleep(Duration::from_millis(20));
        view.scene(1000, 600);
    }
    assert_eq!(selected_name(view.browser()).as_deref(), Some("notes.txt"));
}

#[test]
fn quick_look_in_full_screen_covers_the_window_and_escape_returns() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&press(Key::Up));
    view.handle(&press(Key::Space));
    let panel = view.quick_look_frame();
    assert!(panel.width < 1000.0);

    view.handle(&alt_command(Key::Character('y')));
    let full = view.quick_look_frame();
    assert_eq!(
        (full.x, full.y, full.width, full.height),
        (0.0, 0.0, 1000.0, 600.0)
    );

    view.handle(&press(Key::Escape));
    assert_eq!(view.quick_look_frame(), panel);
    assert!(view.quick_look().is_some());
    view.handle(&press(Key::Escape));
    assert!(view.quick_look().is_none());
}

#[test]
fn super_plus_zooms_a_quick_look_picture_instead_of_the_icons() {
    let place = Place::new();
    picture(&place.home().join("picture.png"), 80, 60);
    let mut view = place.view();
    view.handle(&command(Key::Character('1'), false));
    let icon_size = view.icon_size();
    view.handle(&press(Key::Right));
    while selected_name(view.browser()).as_deref() != Some("picture.png") {
        view.handle(&press(Key::Right));
    }
    view.handle(&press(Key::Space));
    run_until(&mut view, |view| {
        view.quick_look()
            .is_some_and(|shown| matches!(shown.preview, Some(Preview::Image(_))))
    });

    view.handle(&command(Key::Character('='), false));
    view.handle(&command(Key::Character('='), false));

    assert!(view.quick_look().is_some_and(|shown| shown.zoom > 1.5));
    assert_eq!(view.icon_size(), icon_size);
    for _ in 0..3 {
        view.handle(&command(Key::Character('-'), false));
    }
    assert!(view.quick_look().is_some_and(|shown| shown.zoom == 1.0));
}

#[test]
fn quick_look_matches_its_goldens() {
    let place = Place::new();
    fs::write(
        place.home().join("notes.txt"),
        "# Notes\n\nQuick Look shows the first lines of a text file\nin a monospaced face, and Page Down scrolls it.\n\n- one\n- two\n- three\n",
    )
    .expect("text");
    picture(&place.home().join("Lake.png"), 640, 400);
    picture(&place.home().join("Mountain.png"), 300, 450);
    let mut view = place.themed_view();
    view.handle(&press(Key::Up));
    view.handle(&press(Key::Space));
    run_until(&mut view, |view| {
        view.quick_look()
            .is_some_and(|shown| shown.preview.is_some())
    });
    shoot("quick-look-text", &mut view);

    view.handle(&press(Key::Escape));
    view.handle(&press(Key::Up));
    view.handle(&shift(Key::Up));
    view.handle(&press(Key::Space));
    run_until(&mut view, |view| {
        view.quick_look()
            .is_some_and(|shown| shown.preview.is_some() && shown.position().is_some())
    });
    shoot("quick-look-several", &mut view);
}

fn control_node(view: &FilesView, role: accesskit::Role, label: &str) -> accesskit::NodeId {
    view.accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| node.role() == role && node.label() == Some(label))
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no {role:?} named {label}"))
}

fn click_node(view: &mut FilesView, node: accesskit::NodeId) {
    view.perform(&accesskit::ActionRequest {
        action: accesskit::Action::Click,
        target_tree: accesskit::TreeId::ROOT,
        target_node: node,
        data: None,
    });
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).expect("metadata").permissions().mode() & 0o777
}

fn inspect(place: &Place, name: &str) -> FilesView {
    let mut view = place.view();
    view.handle(&command(Key::Character('2'), false));
    while selected_name(view.browser()).as_deref() != Some(name) {
        view.handle(&press(Key::Down));
    }
    view.handle(&command(Key::Character('i'), false));
    view.scene(1000, 600);
    view
}

#[test]
fn the_inspector_switch_lets_a_script_run() {
    let place = Place::new();
    let script = place.home().join("run.sh");
    fs::write(&script, "#!/bin/sh\necho hi\n").expect("script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o644)).expect("mode");
    let mut view = inspect(&place, "run.sh");

    let switch = control_node(&view, accesskit::Role::Switch, "Allow running as a program");
    click_node(&mut view, switch);

    assert_eq!(mode_of(&script), 0o755);
    assert_eq!(
        view.info()
            .and_then(|info| info.mode)
            .map(|mode| mode & 0o777),
        Some(0o755)
    );
    view.scene(1000, 600);
    let node = control_node(&view, accesskit::Role::Switch, "Allow running as a program");
    click_node(&mut view, node);
    assert_eq!(mode_of(&script), 0o644);
}

#[test]
fn read_only_access_removes_the_write_bit_of_that_class_only() {
    let place = Place::new();
    let file = place.home().join("notes.txt");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o664)).expect("mode");
    let mut view = inspect(&place, "notes.txt");

    let node = control_node(&view, accesskit::Role::ComboBox, "Group");
    click_node(&mut view, node);
    assert!(
        view.take_requests()
            .iter()
            .any(|request| matches!(request, Request::ShowMenu { .. }))
    );
    view.menu_action("1");

    assert_eq!(mode_of(&file), 0o644);
    view.scene(1000, 600);
    let node = control_node(&view, accesskit::Role::ComboBox, "Everyone");
    click_node(&mut view, node);
    view.menu_action("2");
    assert_eq!(mode_of(&file), 0o640);
}

#[test]
fn the_inspector_sums_several_items_and_names_their_kinds() {
    let place = Place::new();
    fs::write(place.home().join("more.txt"), "12345").expect("file");
    let mut view = place.view();
    view.handle(&command(Key::Character('2'), false));
    view.handle(&command(Key::Character('a'), false));
    view.handle(&command(Key::Character('i'), false));

    let info = view.info().expect("info");
    assert_eq!(info.entries.len(), 4);
    assert_eq!(info.kind, "inode/directory, text/plain");
}

#[test]
fn the_inspector_matches_its_goldens() {
    let place = Place::new();
    picture(&place.home().join("Lake.png"), 640, 400);
    fs::set_permissions(
        place.home().join("Lake.png"),
        fs::Permissions::from_mode(0o644),
    )
    .expect("mode");
    date_everything(&place.home());
    let mut view = place.themed_view();
    view.handle(&command(Key::Character('2'), false));
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Down));
    view.handle(&press(Key::Down));
    view.handle(&command(Key::Character('i'), false));
    run_until(&mut view, |view| {
        view.info()
            .is_some_and(|info| info.pixels == Some((640, 400)))
    });
    settle_thumbnails(&mut view, GOLDEN_SIZE, 0);
    shoot("inspector-file", &mut view);

    view.handle(&command(Key::Character('a'), false));
    shoot("inspector-several", &mut view);
}

impl Place {
    fn add_viewer(&self) {
        fs::create_dir_all(self.root.path().join("data/applications")).expect("applications");
        fs::write(
            self.root.path().join("data/applications/viewer.desktop"),
            "[Desktop Entry]\nType=Application\nName=Viewer\nExec=viewer %F\n",
        )
        .expect("viewer");
        fs::write(
            self.root.path().join("data/applications/settings.desktop"),
            "[Desktop Entry]\nType=Application\nName=Settings\nExec=settings\n",
        )
        .expect("settings");
    }
}

fn open_with_rows(view: &FilesView) -> Vec<String> {
    view.accessibility()
        .nodes
        .into_iter()
        .filter(|(_, node)| node.role() == accesskit::Role::Row)
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect()
}

#[test]
fn other_apps_open_the_file_and_can_become_the_default() {
    let place = Place::new();
    place.add_viewer();
    let mut view = place.view();
    view.handle(&press(Key::Up));
    view.menu_action("open-with-other");
    assert!(matches!(view.sheet(), Some(Sheet::OpenWith(_))));
    view.scene(1000, 600);
    assert_eq!(open_with_rows(&view), ["Editor", "Viewer"]);

    view.handle(&Event::Commit("view".to_owned()));
    view.scene(1000, 600);
    assert_eq!(open_with_rows(&view), ["Viewer"]);
    let always = control_node(&view, accesskit::Role::CheckBox, "Always Open With");
    click_node(&mut view, always);
    view.handle(&press(Key::Enter));

    assert!(view.sheet().is_none());
    let requests = view.take_requests();
    let [Request::Open { app, files }] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    assert_eq!(app.id, "viewer.desktop");
    assert_eq!(files, &[place.home().join("notes.txt")]);
    let list =
        fs::read_to_string(place.root.path().join("config/mimeapps.list")).expect("mimeapps.list");
    assert!(list.contains("text/plain=viewer.desktop"), "{list}");
}

#[test]
fn a_file_without_an_app_asks_which_app_to_use() {
    let place = Place::new();
    place.add_viewer();
    fs::write(place.home().join("unknown.xyz"), [0u8, 1, 2]).expect("file");
    let mut view = place.view();
    while selected_name(view.browser()).as_deref() != Some("unknown.xyz") {
        view.handle(&press(Key::Down));
    }

    view.handle(&command(Key::Character('o'), false));

    assert!(view.take_requests().is_empty());
    let Some(Sheet::OpenWith(open_with)) = view.sheet() else {
        panic!("{:?}", view.sheet());
    };
    assert_eq!(open_with.files, [place.home().join("unknown.xyz")]);
    view.handle(&press(Key::Enter));
    let requests = view.take_requests();
    let [Request::Open { app, .. }] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    assert_eq!(app.id, "editor.desktop");
}

#[test]
fn the_open_with_sheet_matches_its_golden() {
    let place = Place::new();
    place.add_viewer();
    date_everything(&place.home());
    let mut view = place.themed_view();
    view.handle(&press(Key::Up));
    view.menu_action("open-with-other");
    settle(&mut view);
    shoot("open-with-other", &mut view);
}

fn plain(text: Option<String>) -> Option<String> {
    text.map(|text| text.replace(['\u{2068}', '\u{2069}'], ""))
}

#[test]
fn copying_as_pathname_puts_one_absolute_path_per_line() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&press(Key::Down));
    view.handle(&shift(Key::Down));
    view.take_requests();

    view.handle(&Event::Key {
        key: Key::Character('C'),
        modifiers: Modifiers {
            logo: true,
            alt: true,
            shift: true,
            ..Modifiers::default()
        },
    });

    let expected = format!(
        "{}\n{}",
        place.home().join("Documents").display(),
        place.home().join("Empty").display()
    );
    assert_eq!(
        view.take_requests(),
        [Request::Clipboard(ClipboardRequest::Copy(expected))]
    );
}

#[test]
fn add_to_sidebar_bookmarks_the_selected_folder() {
    let place = Place::new();
    let mut view = place.view();
    view.handle(&press(Key::Down));

    view.handle(&Event::Key {
        key: Key::Character('t'),
        modifiers: Modifiers {
            logo: true,
            control: true,
            ..Modifiers::default()
        },
    });

    let requests = view.take_requests();
    let [Request::EditSidebar(SidebarEdit { bookmarks, .. })] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    assert_eq!(
        bookmarks,
        &Some(BookmarkEdit::Add(vec![place.home().join("Documents")]))
    );
    let menu = right_click_selected(&mut view);
    assert!(titles(&menu).contains(&"Add to Sidebar".to_owned()));
}

fn right_click_selected(view: &mut FilesView) -> Menu {
    view.handle(&Event::Key {
        key: Key::Function(10),
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    });
    match view.take_requests().as_slice() {
        [Request::ShowMenu { menu, .. }] => menu.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_edit_menu_names_the_action_to_undo_and_redo() {
    let place = Place::new();
    let mut view = place.view();
    assert_eq!(view.undo_title(), None);
    view.handle(&command(Key::Character('n'), true));
    view.handle(&press(Key::Escape));
    assert_eq!(plain(view.undo_title()).as_deref(), Some("Undo New Folder"));

    view.handle(&press(Key::Down));
    view.handle(&shift(Key::Down));
    view.handle(&command(Key::Backspace, false));
    run_until(&mut view, idle);
    assert_eq!(
        plain(view.undo_title()).as_deref(),
        Some("Undo Move to Trash")
    );

    view.handle(&command(Key::Character('z'), false));
    run_until(&mut view, idle);
    assert_eq!(
        plain(view.redo_title()).as_deref(),
        Some("Redo Move to Trash")
    );
    assert_eq!(plain(view.undo_title()).as_deref(), Some("Undo New Folder"));
}

#[test]
fn recents_offer_show_in_enclosing_folder() {
    let place = Place::new();
    let notes = place.home().join("notes.txt");
    let xbel = place.root.path().join("recently-used.xbel");
    fs::write(
        &xbel,
        format!(
            "<xbel><bookmark href=\"{}\" visited=\"2026-02-01T08:00:00Z\"/></xbel>",
            file_uri(&notes)
        ),
    )
    .expect("recents");
    let mut view = place.view();
    view.set_recents_sources(xbel, place.root.path().join("usage"));
    view.handle(&command(Key::Character('f'), true));
    view.handle(&press(Key::Down));

    let menu = right_click_selected(&mut view);
    assert!(titles(&menu).contains(&"Show in Enclosing Folder".to_owned()));
    view.menu_action("show-in-enclosing-folder");
    assert_eq!(view.browser().root(), &Location::Folder(place.home()));
    assert_eq!(selected_name(view.browser()).as_deref(), Some("notes.txt"));
}

fn photos(place: &Place) -> FilesView {
    for name in ["IMG_1.jpg", "IMG_2.jpg"] {
        fs::write(place.home().join(name), name).expect("photo");
    }
    let mut view = place.view();
    while selected_name(view.browser()).as_deref() != Some("IMG_1.jpg") {
        view.handle(&press(Key::Down));
    }
    view.handle(&shift(Key::Down));
    view
}

fn batch_state(view: &FilesView) -> bande_files::BatchRename {
    match view.sheet() {
        Some(Sheet::Rename(state)) => state.clone(),
        other => panic!("{other:?}"),
    }
}

fn rename_button_disabled(view: &FilesView) -> bool {
    view.accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| node.role() == accesskit::Role::Button && node.label() == Some("Rename"))
        .map(|(_, node)| node.is_disabled())
        .expect("a Rename button")
}

#[test]
fn renaming_several_items_previews_and_applies_the_new_names() {
    let place = Place::new();
    let mut view = photos(&place);

    view.handle(&press(Key::Function(2)));
    assert_eq!(batch_state(&view).items.len(), 2);
    view.scene(1000, 600);

    view.handle(&Event::Commit("IMG_".to_owned()));
    view.handle(&press(Key::Tab));
    view.handle(&Event::Commit("Photo ".to_owned()));
    let state = batch_state(&view);
    assert_eq!(
        (state.find.as_str(), state.replace.as_str()),
        ("IMG_", "Photo ")
    );
    assert_eq!(
        state
            .preview()
            .into_iter()
            .map(|line| line.replace(['\u{2068}', '\u{2069}'], ""))
            .collect::<Vec<_>>(),
        ["IMG_1.jpg → Photo 1.jpg", "IMG_2.jpg → Photo 2.jpg"]
    );
    assert!(!rename_button_disabled(&view));

    view.handle(&press(Key::Enter));

    assert!(view.sheet().is_none());
    assert!(place.home().join("Photo 1.jpg").exists());
    assert!(place.home().join("Photo 2.jpg").exists());
    assert_eq!(
        names_of(view.browser().selection()),
        ["Photo 1.jpg", "Photo 2.jpg"]
    );
    view.handle(&command(Key::Character('z'), false));
    run_until(&mut view, idle);
    assert!(place.home().join("IMG_1.jpg").exists());
    assert!(place.home().join("IMG_2.jpg").exists());
}

#[test]
fn a_clash_disables_rename_and_says_why() {
    let place = Place::new();
    let mut view = photos(&place);
    view.handle(&press(Key::Function(2)));
    view.scene(1000, 600);

    view.handle(&Event::Commit("1".to_owned()));
    view.handle(&press(Key::Tab));
    view.handle(&Event::Commit("2".to_owned()));

    assert!(rename_button_disabled(&view));
    view.handle(&press(Key::Enter));
    assert!(view.sheet().is_some());
    assert!(place.home().join("IMG_1.jpg").exists());
}

#[test]
fn the_batch_rename_sheet_matches_its_golden() {
    let place = Place::new();
    date_everything(&place.home());
    for name in ["IMG_1.jpg", "IMG_2.jpg", "IMG_3.jpg"] {
        fs::write(place.home().join(name), name).expect("photo");
    }
    let mut view = place.themed_view();
    view.handle(&command(Key::Character('2'), false));
    while selected_name(view.browser()).as_deref() != Some("IMG_1.jpg") {
        view.handle(&press(Key::Down));
    }
    view.handle(&shift(Key::Down));
    view.handle(&shift(Key::Down));
    view.handle(&press(Key::Function(2)));
    view.handle(&Event::Commit("IMG_".to_owned()));
    view.handle(&press(Key::Tab));
    view.handle(&Event::Commit("Lake ".to_owned()));
    settle(&mut view);
    shoot("batch-rename", &mut view);
}

fn choose_in_pop_up(view: &mut FilesView, label: &str, index: usize) {
    view.scene(1000, 600);
    let node = control_node(view, accesskit::Role::ComboBox, label);
    click_node(view, node);
    view.take_requests();
    view.menu_action(&index.to_string());
}

fn view_switch_disabled(view: &FilesView) -> bool {
    view.accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| {
            node.role() == accesskit::Role::RadioGroup && node.label() == Some("View")
        })
        .map(|(_, node)| node.is_disabled())
        .expect("the view switch")
}

#[test]
fn search_filters_keep_the_chosen_kind_and_recent_dates() {
    let place = Place::new();
    picture(&place.home().join("trip.png"), 8, 8);
    fs::write(place.home().join("trip.txt"), "notes").expect("file");
    fs::write(place.home().join("old trip.txt"), "old").expect("file");
    fs::create_dir(place.home().join("trip")).expect("folder");
    let month_ago = std::time::SystemTime::now() - Duration::from_secs(30 * 24 * 3600);
    fs::File::open(place.home().join("old trip.txt"))
        .and_then(|file| file.set_modified(month_ago))
        .expect("date");
    let mut view = place.view();
    assert!(!view_switch_disabled(&view));

    view.handle(&command(Key::Character('f'), false));
    type_text(&mut view, "trip");
    run_until(&mut view, |view| shown(view).len() == 4);
    assert!(view_switch_disabled(&view));
    view.handle(&command(Key::Character('3'), false));
    assert_eq!(view.mode(), ViewMode::List);

    choose_in_pop_up(&mut view, "Kind", 3);
    assert_eq!(shown(&view), ["trip.png"]);
    choose_in_pop_up(&mut view, "Kind", 2);
    let mut documents = shown(&view);
    documents.sort();
    assert_eq!(documents, ["old trip.txt", "trip.txt"]);
    choose_in_pop_up(&mut view, "Date Modified", 2);
    assert_eq!(shown(&view), ["trip.txt"]);

    view.handle(&press(Key::Escape));
    assert_eq!(view.searching(), None);
    assert_eq!(view.mode(), ViewMode::Columns);
    assert!(!view_switch_disabled(&view));
}

fn settle(view: &mut FilesView) {
    let deadline = Instant::now() + Duration::from_secs(5);
    view.scene(GOLDEN_SIZE.0, GOLDEN_SIZE.1);
    while view.animating() {
        assert!(Instant::now() < deadline, "the view did not settle");
        std::thread::sleep(Duration::from_millis(20));
        view.scene(GOLDEN_SIZE.0, GOLDEN_SIZE.1);
    }
}

#[test]
fn files_follows_the_text_size() {
    fn widest_text(items: &[Item]) -> f32 {
        items
            .iter()
            .map(|item| match item {
                Item::Text { paragraph, .. } => paragraph.width(),
                Item::Clip { items, .. } | Item::Opacity { items, .. } => widest_text(items),
                _ => 0.0,
            })
            .fold(0.0, f32::max)
    }
    let place = Place::new();
    let mut view = place.view();
    view.handle(&command(Key::Character('2'), false));
    let regular = widest_text(&view.scene(1000, 600).items);

    view.set_appearance(bande_toolkit::Appearance {
        text_size: bande_toolkit::TextSize::LARGEST,
        ..bande_toolkit::Appearance::default()
    });

    assert!(widest_text(&view.scene(1000, 600).items) > regular * 1.5);
}
