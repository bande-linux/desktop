#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use unbound_desktop_surface::{
    Bounds, Desktop, DesktopView, Direction, Grid, Item, Kind, Layer, Layouts, Luma, MenuApp,
    Positions, Request, Selection, Slot, SortBy, WallpaperKey, command, desktop_menu, fitted_label,
    format_layouts, items_menu, parse_layouts,
};
use unbound_toolkit::ui::{Event, Key, Menu, Modifiers, Pointer, PointerButton};
use unbound_toolkit::{
    Appearance, Color, CpuRenderer, Image, Point, Scheme, TextAlign, TextStyle, TextSystem,
};
use unbound_wallpaper::{Fit, render};

const SCREEN: (f32, f32) = (720.0, 480.0);
const MIN_SSIM: f64 = 0.99;

fn item(name: &str, kind: Kind) -> Item {
    Item {
        path: PathBuf::from("/home/ada/Desktop").join(name),
        name: name.to_owned(),
        kind,
        kind_name: match kind {
            Kind::Folder => "Folder".to_owned(),
            Kind::Volume => "Mount point".to_owned(),
            Kind::File => "Text document".to_owned(),
        },
        size: name.len() as u64,
        added: None,
    }
}

fn path(name: &str) -> PathBuf {
    PathBuf::from("/home/ada/Desktop").join(name)
}

fn grid() -> Grid {
    Grid::new(Bounds::new(64.0, 24.0, 1216.0, 776.0))
}

fn desktop(names: &[&str]) -> Desktop {
    let mut desktop = Desktop::new(grid());
    desktop.set_items(
        names.iter().map(|name| item(name, Kind::File)).collect(),
        &Positions::new(),
    );
    desktop
}

fn slot(column: u32, row: u32) -> Slot {
    Slot { column, row }
}

fn slot_of(desktop: &Desktop, name: &str) -> Slot {
    desktop.slot_of(&path(name)).unwrap()
}

#[test]
fn items_fill_the_grid_from_the_top_right_down_then_left() {
    let grid = grid();
    assert_eq!((grid.columns, grid.rows), (11, 6));
    let first = grid.cell(slot(0, 0));
    assert_eq!(first.x + first.width, 64.0 + 1216.0 - 12.0);
    assert_eq!(first.y, 24.0 + 12.0);
    let names: Vec<String> = (0..8).map(|index| format!("note {index}.txt")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let desktop = desktop(&names);
    assert_eq!(slot_of(&desktop, "note 0.txt"), slot(0, 0));
    assert_eq!(slot_of(&desktop, "note 5.txt"), slot(0, 5));
    assert_eq!(slot_of(&desktop, "note 6.txt"), slot(1, 0));
    assert_eq!(grid.slot_at(first.x + 10.0, first.y + 10.0), slot(0, 0));
}

#[test]
fn remembered_places_win_and_new_items_take_the_first_free_slot() {
    let mut desktop = Desktop::new(grid());
    let remembered: Positions = [("b.txt".to_owned(), slot(0, 0))].into_iter().collect();
    desktop.set_items(
        vec![item("a.txt", Kind::File), item("b.txt", Kind::File)],
        &remembered,
    );
    assert_eq!(slot_of(&desktop, "b.txt"), slot(0, 0));
    assert_eq!(slot_of(&desktop, "a.txt"), slot(0, 1));
    desktop.move_items(&[path("a.txt")], (3, 2));
    assert_eq!(slot_of(&desktop, "a.txt"), slot(3, 3));
    desktop.set_items(
        vec![
            item("a.txt", Kind::File),
            item("b.txt", Kind::File),
            item("c.txt", Kind::File),
        ],
        &Positions::new(),
    );
    assert_eq!(slot_of(&desktop, "a.txt"), slot(3, 3));
    assert_eq!(slot_of(&desktop, "c.txt"), slot(0, 1));
    assert_eq!(desktop.positions().get("a.txt"), Some(&slot(3, 3)));
}

#[test]
fn moving_onto_a_taken_slot_lands_next_to_it_and_clean_up_packs_in_order() {
    let mut desktop = desktop(&["a", "b", "c"]);
    desktop.move_items(&[path("c")], (0, -2));
    assert_ne!(slot_of(&desktop, "c"), slot_of(&desktop, "a"));
    let c = slot_of(&desktop, "c");
    assert!(c.column <= 1 && c.row <= 1, "{c:?}");
    desktop.move_items(&[path("a")], (5, 4));
    desktop.clean_up();
    let mut slots: Vec<Slot> = ["a", "b", "c"]
        .iter()
        .map(|name| slot_of(&desktop, name))
        .collect();
    slots.sort();
    assert_eq!(slots, vec![slot(0, 0), slot(0, 1), slot(0, 2)]);
    assert_eq!(slot_of(&desktop, "a"), slot(0, 2));
}

#[test]
fn sorting_keeps_volumes_first_and_stays_sorted_as_files_arrive() {
    let mut desktop = Desktop::new(grid());
    desktop.set_items(
        vec![
            item("notes 10.txt", Kind::File),
            item("notes 9.txt", Kind::File),
            item("Stick", Kind::Volume),
            item("Archive", Kind::Folder),
        ],
        &Positions::new(),
    );
    desktop.arrange(SortBy::Name);
    assert_eq!(slot_of(&desktop, "Stick"), slot(0, 0));
    assert_eq!(slot_of(&desktop, "Archive"), slot(0, 1));
    assert_eq!(slot_of(&desktop, "notes 9.txt"), slot(0, 2));
    assert_eq!(slot_of(&desktop, "notes 10.txt"), slot(0, 3));
    desktop.set_items(
        vec![
            item("notes 10.txt", Kind::File),
            item("notes 9.txt", Kind::File),
            item("Stick", Kind::Volume),
            item("Archive", Kind::Folder),
            item("apple.txt", Kind::File),
        ],
        &Positions::new(),
    );
    assert_eq!(slot_of(&desktop, "apple.txt"), slot(0, 1));
    assert_eq!(slot_of(&desktop, "Archive"), slot(0, 2));
    desktop.arrange(SortBy::Kind);
    assert_eq!(slot_of(&desktop, "Archive"), slot(0, 1));
    desktop.move_items(&[path("Archive")], (2, 0));
    assert_eq!(desktop.sort(), SortBy::None);
}

#[test]
fn arrows_move_the_selection_across_the_grid() {
    let names: Vec<String> = (0..8).map(|index| format!("{index}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut desktop = desktop(&names);
    desktop.navigate(Direction::Down, false);
    assert_eq!(desktop.selected(), vec![path("0")]);
    desktop.navigate(Direction::Down, false);
    assert_eq!(desktop.selected(), vec![path("1")]);
    desktop.navigate(Direction::Left, false);
    assert_eq!(desktop.selected(), vec![path("7")]);
    desktop.navigate(Direction::Left, false);
    assert_eq!(desktop.selected(), vec![path("7")]);
    desktop.navigate(Direction::Right, true);
    assert_eq!(desktop.selected(), vec![path("1"), path("7")]);
    desktop.navigate(Direction::Up, false);
    assert_eq!(desktop.selected(), vec![path("0")]);
}

#[test]
fn the_rubber_band_selects_what_it_touches_and_modifiers_toggle() {
    let mut desktop = desktop(&["a", "b", "c"]);
    let a = grid().cell(slot(0, 0));
    let b = grid().cell(slot(0, 1));
    let band = Bounds::spanning((a.x + 50.0, a.y + 20.0), (b.x + 52.0, b.y + 20.0));
    desktop.select_in(band, &BTreeSet::new());
    assert_eq!(desktop.selected(), vec![path("a"), path("b")]);
    let keep: BTreeSet<PathBuf> = [path("a"), path("c")].into_iter().collect();
    desktop.select_in(band, &keep);
    assert_eq!(desktop.selected(), vec![path("b"), path("c")]);
}

#[test]
fn typing_selects_the_first_name_at_or_after_the_letters() {
    let mut desktop = desktop(&["Budget.ods", "apple.txt", "Carrot", "bank 2.pdf"]);
    assert!(desktop.select_prefix("b"));
    assert_eq!(desktop.selected(), vec![path("bank 2.pdf")]);
    assert!(desktop.select_prefix("bu"));
    assert_eq!(desktop.selected(), vec![path("Budget.ods")]);
    assert!(desktop.select_prefix("ca"));
    assert_eq!(desktop.selected(), vec![path("Carrot")]);
    assert!(!desktop.select_prefix("zz"));
}

#[test]
fn layouts_survive_a_round_trip_per_grid() {
    let mut layouts = Layouts::new();
    layouts.insert(
        "11x6".to_owned(),
        [
            ("a b.txt".to_owned(), slot(2, 3)),
            ("c".to_owned(), slot(0, 0)),
        ]
        .into_iter()
        .collect(),
    );
    layouts.insert(
        "8x5".to_owned(),
        [("a b.txt".to_owned(), slot(1, 1))].into_iter().collect(),
    );
    let text = format_layouts(&layouts);
    assert_eq!(parse_layouts(&text), layouts);
    assert_eq!(parse_layouts("garbage\n8x5\tx\t1\tname\n"), Layouts::new());
}

#[test]
fn long_names_are_cut_in_the_middle_to_two_lines_keeping_the_extension() {
    let mut text = TextSystem::new();
    let style = TextStyle::new(12.0, Color::hex(0));
    let short = fitted_label(&mut text, "Notes.txt", style, 96.0);
    assert_eq!(short.line_count(), 1);
    let long = "A very long report about the quarterly numbers of the north region final.pdf";
    let cut = fitted_label(&mut text, long, style, 96.0);
    assert_eq!(cut.line_count(), 2);
    assert!(cut.width() <= 96.5);
    let full = text.paragraph(long, style, Some(96.0), TextAlign::Center);
    assert!(full.line_count() > 2);
}

fn view_with_items(scheme: Scheme) -> DesktopView {
    let now = Instant::now();
    let mut view = DesktopView::new(
        Appearance {
            scheme,
            ..Appearance::default()
        },
        Layouts::new(),
    );
    view.set_viewport(
        SCREEN,
        1.0,
        Bounds::new(64.0, 24.0, SCREEN.0 - 64.0, SCREEN.1 - 24.0),
        now,
    );
    let folder = Image::from_rgba(64, 64, [0x5B, 0x8D, 0xEF, 0xFF].repeat(64 * 64));
    let document = Image::from_rgba(64, 64, [0xF4, 0xF5, 0xF7, 0xFF].repeat(64 * 64));
    view.set_items(
        vec![
            (item("Projects", Kind::Folder), folder.clone()),
            (
                item("Screenshot 2026-10-30 at 09.41.12.png", Kind::File),
                document.clone(),
            ),
            (item("Notes.txt", Kind::File), document),
            (item("Stick", Kind::Volume), folder),
        ],
        now,
    );
    view
}

fn wallpaper(view: &mut DesktopView, scheme: Scheme) {
    let name = match scheme {
        Scheme::Light => "drift-light.svg",
        Scheme::Dark => "drift-dark.svg",
    };
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../data/wallpapers")
        .join(name);
    let picture = render(&path, Fit::Fill, SCREEN.0 as u32, SCREEN.1 as u32).unwrap();
    let luma = Luma::of(&picture);
    let image = Image::from_rgba(picture.width, picture.height, picture.rgba).unwrap();
    view.show_wallpaper(
        Layer {
            key: WallpaperKey {
                path,
                fit: Fit::Fill,
                width: SCREEN.0 as u32,
                height: SCREEN.1 as u32,
            },
            image,
            luma,
        },
        Instant::now() - Duration::from_secs(5),
    );
}

fn check_golden(name: &str, view: &mut DesktopView) {
    let scene = view.scene(Instant::now() + Duration::from_secs(5));
    let frame = CpuRenderer::new().render(&scene).unwrap();
    let actual = image::RgbaImage::from_raw(scene.width, scene.height, frame.rgba).unwrap();
    let actual = image::DynamicImage::ImageRgba8(actual).to_rgb8();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
    if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        actual.save(&path).unwrap();
    }
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    actual.save(&target).unwrap();
    let expected = image::open(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .to_rgb8();
    let similarity = image_compare::rgb_hybrid_compare(&expected, &actual).unwrap();
    assert!(
        similarity.score >= MIN_SSIM,
        "{name} differs from its golden: {:.4} (frame in {})",
        similarity.score,
        target.display()
    );
}

fn at(view: &DesktopView, name: &str) -> Point {
    let cell = view
        .desktop()
        .grid()
        .cell(view.desktop().slot_of(&path(name)).unwrap());
    Point::new(cell.x + cell.width / 2.0, cell.y + 30.0)
}

fn click(view: &mut DesktopView, point: Point, count: u8, modifiers: Modifiers) {
    let pointer = Pointer::at(point)
        .with_count(count)
        .with_modifiers(modifiers);
    view.handle(&Event::PointerDown(pointer), Instant::now());
    view.handle(&Event::PointerUp(pointer), Instant::now());
}

fn key(view: &mut DesktopView, key: Key, modifiers: Modifiers) {
    view.handle(&Event::Key { key, modifiers }, Instant::now());
}

const SUPER: Modifiers = Modifiers {
    shift: false,
    alt: false,
    control: false,
    logo: true,
};

#[test]
fn the_desktop_matches_its_golden_in_light_and_dark() {
    for (scheme, name) in [
        (Scheme::Light, "desktop-light"),
        (Scheme::Dark, "desktop-dark"),
    ] {
        let mut view = view_with_items(scheme);
        wallpaper(&mut view, scheme);
        view.set_focused(true);
        let notes = at(&view, "Notes.txt");
        click(&mut view, notes, 1, Modifiers::default());
        check_golden(name, &mut view);
    }
}

#[test]
fn clicks_and_keys_ask_for_the_right_file_actions() {
    let mut view = view_with_items(Scheme::Dark);
    let notes = at(&view, "Notes.txt");
    click(&mut view, notes, 1, Modifiers::default());
    assert_eq!(view.desktop().selected(), vec![path("Notes.txt")]);
    click(&mut view, notes, 2, Modifiers::default());
    assert_eq!(
        view.take_requests(),
        vec![Request::Open(vec![path("Notes.txt")])]
    );
    let projects = at(&view, "Projects");
    click(&mut view, projects, 1, SUPER);
    key(&mut view, Key::Backspace, SUPER);
    assert_eq!(
        view.take_requests(),
        vec![Request::Trash(vec![path("Projects"), path("Notes.txt")])]
    );
    key(&mut view, Key::Escape, Modifiers::default());
    assert!(view.desktop().selected().is_empty());
    key(
        &mut view,
        Key::Character('n'),
        Modifiers {
            shift: true,
            ..SUPER
        },
    );
    key(&mut view, Key::Character('z'), SUPER);
    assert_eq!(
        view.take_requests(),
        vec![Request::NewFolder, Request::Undo]
    );
}

#[test]
fn renaming_in_place_confirms_with_return_and_cancels_with_escape() {
    let mut view = view_with_items(Scheme::Light);
    let notes = at(&view, "Notes.txt");
    click(&mut view, notes, 1, Modifiers::default());
    key(&mut view, Key::Function(2), Modifiers::default());
    for letter in "Ideas".chars() {
        key(&mut view, Key::Character(letter), Modifiers::default());
    }
    key(&mut view, Key::Enter, Modifiers::default());
    assert_eq!(
        view.take_requests(),
        vec![Request::Rename {
            path: path("Notes.txt"),
            name: "Ideas.txt".to_owned(),
        }]
    );
    key(&mut view, Key::Function(2), Modifiers::default());
    key(&mut view, Key::Character('x'), Modifiers::default());
    key(&mut view, Key::Escape, Modifiers::default());
    assert!(view.take_requests().is_empty());
    let label = Point::new(notes.x, notes.y + 50.0);
    click(&mut view, label, 1, Modifiers::default());
    key(&mut view, Key::Character('y'), Modifiers::default());
    key(&mut view, Key::Enter, Modifiers::default());
    assert_eq!(
        view.take_requests(),
        vec![Request::Rename {
            path: path("Notes.txt"),
            name: "y.txt".to_owned(),
        }]
    );
}

#[test]
fn dragging_moves_icons_on_the_grid_or_into_a_folder() {
    let mut view = view_with_items(Scheme::Light);
    let notes = at(&view, "Notes.txt");
    let grid = view.desktop().grid();
    let target = grid.cell(slot(3, 2));
    let drop = Point::new(target.x + target.width / 2.0, target.y + 30.0);
    let pointer = Pointer::at(notes);
    let now = Instant::now();
    view.handle(&Event::PointerDown(pointer), now);
    view.handle(
        &Event::PointerMove(Pointer::at(Point::new(notes.x - 20.0, notes.y))),
        now,
    );
    view.handle(&Event::PointerMove(Pointer::at(drop)), now);
    view.handle(&Event::PointerUp(Pointer::at(drop)), now);
    assert_eq!(view.desktop().slot_of(&path("Notes.txt")), Some(slot(3, 2)));
    assert_eq!(
        view.take_changed_layouts()
            .and_then(|layouts| layouts.get(&grid.key()).cloned())
            .and_then(|positions| positions.get("Notes.txt").copied()),
        Some(slot(3, 2))
    );
    let notes = at(&view, "Notes.txt");
    let projects = at(&view, "Projects");
    view.handle(&Event::PointerDown(Pointer::at(notes)), now);
    view.handle(&Event::PointerMove(Pointer::at(projects)), now);
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    view.handle(
        &Event::PointerUp(Pointer::at(projects).with_modifiers(alt)),
        now,
    );
    assert_eq!(
        view.take_requests(),
        vec![Request::Transfer {
            sources: vec![path("Notes.txt")],
            folder: path("Projects"),
            copy: true,
        }]
    );
    let secondary = Pointer::at(projects).with_button(PointerButton::Secondary);
    view.handle(&Event::PointerDown(secondary), now);
    assert_eq!(view.desktop().selected(), vec![path("Projects")]);
    assert_eq!(
        view.take_requests(),
        vec![Request::ContextMenu {
            at: projects,
            on_items: true,
        }]
    );
}

fn titles(menu: &Menu) -> Vec<String> {
    menu.items
        .iter()
        .filter(|item| !item.separator)
        .map(|item| item.title.clone())
        .collect()
}

#[test]
fn the_menus_offer_what_fits_the_selection_and_their_actions_reach_the_desktop() {
    let menu = desktop_menu(SortBy::Name);
    assert_eq!(
        titles(&menu),
        [
            "New Folder",
            "Get Info",
            "Change Wallpaper…",
            "Sort By",
            "Clean Up",
            "Show Desktop Folder in Files"
        ]
    );
    let sorts = menu
        .items
        .iter()
        .find_map(|item| item.submenu.clone())
        .unwrap();
    let checked: Vec<&str> = sorts
        .items
        .iter()
        .filter(|item| item.checked)
        .map(|item| item.title.as_str())
        .collect();
    assert_eq!(checked, ["Name"]);
    assert!(
        !menu
            .items
            .iter()
            .any(|item| item.title == "Clean Up" && item.enabled)
    );

    let files = items_menu(&Selection {
        count: 3,
        kinds: vec![Kind::File, Kind::Folder, Kind::File],
        picture: false,
        apps: vec![MenuApp {
            id: "org.example.Editor.desktop".to_owned(),
            name: "Editor".to_owned(),
        }],
    });
    assert!(titles(&files).contains(&"Move 3 Items to Trash".to_owned()));
    assert!(
        !files
            .items
            .iter()
            .any(|item| item.title == "Rename" && item.enabled)
    );
    let volume = items_menu(&Selection {
        count: 1,
        kinds: vec![Kind::Volume],
        picture: false,
        apps: Vec::new(),
    });
    assert!(titles(&volume).contains(&"Eject".to_owned()));
    assert!(!titles(&volume).iter().any(|title| title.contains("Trash")));
    let picture = items_menu(&Selection {
        count: 1,
        kinds: vec![Kind::File],
        picture: true,
        apps: Vec::new(),
    });
    assert!(titles(&picture).contains(&"Set as Wallpaper".to_owned()));

    let mut view = view_with_items(Scheme::Light);
    let now = Instant::now();
    view.run_command(command("sort:size").unwrap(), now);
    assert_eq!(view.desktop().sort(), SortBy::Size);
    let notes = at(&view, "Notes.txt");
    click(&mut view, notes, 1, Modifiers::default());
    let open_with = files
        .items
        .iter()
        .find_map(|item| item.submenu.clone())
        .unwrap();
    view.run_command(command(&open_with.items[0].action).unwrap(), now);
    view.run_command(command("trash").unwrap(), now);
    assert_eq!(
        view.take_requests(),
        vec![
            Request::OpenWith {
                app: "org.example.Editor.desktop".to_owned(),
                paths: vec![path("Notes.txt")],
            },
            Request::Trash(vec![path("Notes.txt")]),
        ]
    );
    assert_eq!(command("unknown"), None);
}
