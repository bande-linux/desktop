#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use tempfile::TempDir;
use unbound_files::{
    Browser, FilesView, Location, ROW_HEIGHT, Request, Row, SIDEBAR_WIDTH, Sheet, permissions,
    rows, with_volumes, without_bookmark,
};
use unbound_fs::{Sort, SortBy, Trash, list};
use unbound_toolkit::ui::{Event, Key, Menu, Modifiers, Pointer, PointerButton};
use unbound_toolkit::{Item, Palette, Point, Scheme, TextSystem};
use unbound_xdg::Apps;

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

#[test]
fn filtering_narrows_the_folder_and_escape_restores_it() {
    let place = Place::new();
    let mut view = place.view();
    let shown = |view: &FilesView| {
        view.browser().columns()[0]
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>()
    };

    view.handle(&command(Key::Character('f'), false));
    view.handle(&press(Key::Character('N')));
    view.handle(&press(Key::Character('o')));

    assert_eq!(shown(&view), ["notes.txt"]);
    assert_eq!(
        view.browser().selected().map(|entry| entry.name.as_str()),
        Some("notes.txt")
    );
    view.handle(&press(Key::Escape));
    assert!(view.search().is_none());
    assert_eq!(shown(&view), ["Documents", "Empty", "notes.txt"]);
    assert_eq!(
        view.browser().selected().map(|entry| entry.name.as_str()),
        Some("notes.txt")
    );
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

fn titles(menu: &Menu) -> Vec<&str> {
    menu.items
        .iter()
        .filter(|item| !item.separator)
        .map(|item| item.title.as_str())
        .collect()
}

#[test]
fn right_clicking_an_item_selects_it_and_its_menu_acts_on_it() {
    let place = Place::new();
    let mut view = place.view();

    let menu = right_click(
        &mut view,
        Point::new(SIDEBAR_WIDTH + 60.0, 6.0 + ROW_HEIGHT * 2.5),
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
            "Copy",
            "Move to Trash"
        ]
    );
    let open_with = menu
        .items
        .iter()
        .find_map(|item| item.submenu.as_ref())
        .expect("open with");
    assert_eq!(titles(open_with), ["Editor"]);
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
        Point::new(SIDEBAR_WIDTH + 60.0, 6.0 + ROW_HEIGHT * 2.5),
    );
    assert!(view.menu_action("info"));
    assert_eq!(
        view.info().map(|info| info.entries[0].name.as_str()),
        Some("notes.txt")
    );
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
        Point::new(SIDEBAR_WIDTH + 60.0, 6.0 + ROW_HEIGHT * 0.5),
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
            "Sort By",
            "as Columns",
            "as List",
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
fn a_bookmark_is_removed_from_the_sidebar_but_not_from_the_disk() {
    let place = Place::new();
    let reports = place.home().join("Documents/Reports");
    let bookmarks = format!("file://{}\nfile:///srv/share Share\n", reports.display());
    let mut view = place.view();
    view.set_sidebar(rows(&place.home(), None, Some(&bookmarks)));
    view.scene(1000, 600);

    let menu = right_click(&mut view, Point::new(40.0, 38.0 + 26.0 * 2.5));
    assert_eq!(titles(&menu), ["Open", "Remove from Sidebar"]);
    assert!(view.menu_action("remove-from-sidebar"));
    assert_eq!(
        view.take_requests(),
        [Request::RemoveBookmark(reports.clone())]
    );
    assert_eq!(
        without_bookmark(&bookmarks, &reports),
        "file:///srv/share Share\n"
    );
    assert!(reports.is_dir());
}

#[test]
fn mounted_volumes_are_listed_under_locations_before_the_trash() {
    let home = PathBuf::from("/home/ada");
    let favourites = rows(&home, None, None);
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
        ["Favourites", "ada", "Locations", "Stick", "Trash"]
    );
    assert_eq!(with_volumes(&shown, &[]), favourites);
}
