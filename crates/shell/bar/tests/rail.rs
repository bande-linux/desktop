#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bande_bar::dock::{DockAction, DockApp, DockState, MenuTarget, Shared, dock_ui, scene};
use bande_bar::menu_bar::{Slot, StatusView};
use bande_bar::menus::{AppMenus, bluetooth_menu, cascade, wifi_menu};
use bande_bar::rail::{RailView, bar_width, surface_size};
use bande_config::BarEdge;
use bande_datetime::{DateFormats, posix_locale};
use bande_dock_model::LauncherUpdate;
use bande_status::dbusmenu::Node;
use bande_status::status::{
    Battery, Bluetooth, Capture, Gadget, InputSource, InputSources, Power, Sense, Sound, Wifi,
};
use bande_toolkit::ui::{
    Event, Key, MenuOutcome, MenuPanel, MenuTracker, Modifiers, Pointer, PointerButton, Ui,
};
use bande_toolkit::{
    Appearance, Color, CpuRenderer, Elevation, Image, Item, MenuPlacement, Palette, Point, Rect,
    Scene, Scheme, Shape, TextSystem,
};

use support::check_golden;

const ICON: f32 = 40.0;
const HEIGHT: u32 = 720;

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data")
}

fn app_icon(name: &str, scale: f64) -> Option<Image> {
    let pixels = (f64::from(ICON) * scale).round() as u32;
    Image::load(
        &data().join(format!("icons/Bande/scalable/apps/{name}.svg")),
        pixels,
    )
}

fn app(app_id: &str, name: &str, kept: bool, running: bool, scale: f64) -> DockApp {
    DockApp {
        app_id: app_id.to_owned(),
        name: name.to_owned(),
        icon: app_icon(app_id, scale),
        kept,
        running,
    }
}

fn apps(scale: f64) -> Vec<DockApp> {
    vec![
        app("org.bande.Files", "Files", true, true, scale),
        app("org.bande.Terminal", "Terminal", true, true, scale),
        app("org.bande.Settings", "Settings", true, false, scale),
        app("org.gnome.TextEditor", "Text Editor", false, true, scale),
    ]
}

fn numbered(count: usize) -> Vec<DockApp> {
    (0..count)
        .map(|index| {
            app(
                &format!("org.example.App{index}"),
                &format!("App {index}"),
                true,
                false,
                1.0,
            )
        })
        .collect()
}

fn tray_icon() -> Option<Image> {
    let size = 15;
    let pixels = (0..size * size)
        .flat_map(|index| {
            let (x, y) = (index % size, index / size);
            let inside = (x as i32 - 7).pow(2) + (y as i32 - 7).pow(2) <= 36;
            [0x30, 0x80, 0xD0, if inside { 0xFF } else { 0 }]
        })
        .collect();
    Image::from_rgba(size, size, pixels)
}

struct Rail {
    view: RailView,
    state: Shared,
    ui: Ui,
    clock: Rc<Cell<Instant>>,
    height: u32,
}

impl Rail {
    fn new(scheme: Scheme, height: u32, apps: Vec<DockApp>) -> Self {
        Self::at_scale(scheme, height, apps, 1.0)
    }

    fn at_scale(scheme: Scheme, height: u32, apps: Vec<DockApp>, scale: f64) -> Self {
        let palette = Palette::for_scheme(scheme);
        let mut view = RailView::new(palette, TextSystem::new(), vec![data()], ICON);
        let status = view.status_mut();
        status.wifi = Some(Wifi {
            enabled: true,
            networks: Vec::new(),
            sign_in: false,
        });
        status.sound = Some(Sound {
            volume: 0.6,
            muted: false,
            outputs: Vec::new(),
            default_output: None,
        });
        status.bluetooth = Some(Bluetooth {
            powered: true,
            devices: vec![Gadget {
                address: "00:11".to_owned(),
                name: "Headphones".to_owned(),
                connected: true,
                battery: None,
            }],
        });
        status.battery = Some(Battery {
            percent: 84.0,
            power: Power::Battery { remaining: None },
        });
        status.input_sources = InputSources {
            sources: vec![
                InputSource {
                    name: "English".to_owned(),
                    short: "EN".to_owned(),
                },
                InputSource {
                    name: "Swedish".to_owned(),
                    short: "SV".to_owned(),
                },
            ],
            active: 0,
        };
        view.set_tray(vec![
            ("Sync".to_owned(), tray_icon()),
            ("Weather".to_owned(), tray_icon()),
        ]);
        view.set_unread(true);
        view.set_app("Files");
        view.set_app_icon(Image::load(
            &data().join("icons/Bande/scalable/apps/org.bande.Files.svg"),
            (24.0 * scale).round() as u32,
        ));
        view.set_workspaces(3, 0);
        let at = "2026-10-29T14:32:00[Europe/Stockholm]"
            .parse()
            .expect("time");
        view.set_time(&at, &DateFormats::for_locale(&posix_locale("en_GB")));
        let state = DockState::new(BarEdge::Left, ICON);
        state.borrow_mut().set_appearance(Appearance {
            scheme,
            ..Appearance::default()
        });
        state.borrow_mut().set_rail(view.apps_region(height as f32));
        let pixels = (f64::from(ICON) * scale).round() as u32;
        let trash = data().join("icons/Bande/scalable/places/user-trash.svg");
        state
            .borrow_mut()
            .set_trash_icons(Image::load(&trash, pixels), None);
        let clock = Rc::new(Cell::new(Instant::now()));
        state.borrow_mut().set_apps(apps, clock.get());
        state.borrow_mut().activated("org.bande.Files");
        let mut ui = dock_ui(state.clone(), palette, TextSystem::new());
        let time = clock.clone();
        ui.set_clock(move || time.get());
        Self {
            view,
            state,
            ui,
            clock,
            height,
        }
    }

    fn size(&self) -> (u32, u32) {
        (surface_size(ICON).0, self.height)
    }

    fn frame(&mut self, scale: f64) -> Vec<u8> {
        let (width, height) = self.size();
        let mut composed = self.view.scene(width, height, scale);
        let apps = scene(&mut self.ui, (width, height));
        composed.items.extend(apps.items);
        let pixels = (
            (f64::from(width) * scale).round() as usize,
            (f64::from(height) * scale).round() as usize,
        );
        let mut rgba = vec![0; pixels.0 * pixels.1 * 4];
        CpuRenderer::new()
            .render_into(&composed, scale, &mut rgba)
            .expect("frame");
        rgba
    }

    fn advance(&mut self, by: Duration) {
        self.clock.set(self.clock.get() + by);
        self.state.borrow_mut().tick(self.clock.get());
    }
}

#[test]
fn the_combined_bar_matches_its_goldens_in_light_and_dark_and_at_one_and_a_half() {
    for (scheme, scale, name) in [
        (Scheme::Light, 1.0, "bar-light"),
        (Scheme::Dark, 1.0, "bar-dark"),
        (Scheme::Light, 1.5, "bar-light-1.5x"),
    ] {
        let mut rail = Rail::at_scale(scheme, HEIGHT, apps(scale), scale);
        rail.state.borrow_mut().update_launcher(
            "org.bande.Terminal",
            LauncherUpdate {
                count: Some(3),
                count_visible: Some(true),
                ..LauncherUpdate::default()
            },
            rail.clock.get(),
        );
        rail.frame(scale);
        rail.advance(Duration::from_secs(2));
        let frame = rail.frame(scale);
        let (width, height) = rail.size();
        let pixels = (
            (f64::from(width) * scale).round() as u32,
            (f64::from(height) * scale).round() as u32,
        );
        check_golden(name, pixels, frame);
    }
}

fn center(rect: Rect) -> Point {
    Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
}

#[test]
fn the_mark_opens_the_launcher_and_its_menu_and_status_glyphs_share_one_button() {
    let mut rail = Rail::new(Scheme::Light, HEIGHT, apps(1.0));
    rail.frame(1.0);
    let view = &rail.view;
    let mark = center(view.slot_rect(Slot::System).expect("mark"));
    let wifi = center(view.slot_rect(Slot::Wifi).expect("wifi"));
    let clock = center(view.slot_rect(Slot::Clock).expect("clock"));
    assert_eq!(
        view.slot_at(mark, PointerButton::Primary),
        Some(Slot::Launcher)
    );
    assert_eq!(
        view.slot_at(mark, PointerButton::Secondary),
        Some(Slot::System)
    );
    assert_eq!(
        view.slot_at(wifi, PointerButton::Primary),
        Some(Slot::ControlCenter)
    );
    assert_eq!(
        view.slot_at(wifi, PointerButton::Secondary),
        Some(Slot::Wifi)
    );
    assert_eq!(
        view.slot_at(clock, PointerButton::Primary),
        Some(Slot::Clock)
    );
    let edge = Point::new(0.0, clock.y);
    assert_eq!(
        view.slot_at(edge, PointerButton::Primary),
        Some(Slot::Clock)
    );
}

#[test]
fn the_tray_shows_two_per_row_and_folds_beyond_three_items_into_more() {
    let mut rail = Rail::new(Scheme::Light, HEIGHT, apps(1.0));
    rail.view.set_tray(
        ["A", "B", "C", "D"]
            .map(|title| (title.to_owned(), tray_icon()))
            .to_vec(),
    );
    rail.frame(1.0);
    let view = &rail.view;
    let first = view.slot_rect(Slot::Tray(0)).expect("first");
    let second = view.slot_rect(Slot::Tray(1)).expect("second");
    let third = view.slot_rect(Slot::Tray(2)).expect("third");
    assert_eq!(first.y, second.y);
    assert!(third.y > first.y);
    assert!(view.slot_rect(Slot::Tray(3)).is_none());
    assert!(view.slot_rect(Slot::InputSource).is_none());
    assert!(view.slot_rect(Slot::TrayMore).is_some());
    assert_eq!(view.hidden_tray(), [Slot::Tray(3), Slot::InputSource]);
    rail.view.set_expanded(true);
    rail.frame(1.0);
    let view = &rail.view;
    assert!(view.hidden_tray().is_empty());
    assert!(view.slot_rect(Slot::TrayMore).is_none());
    assert!(view.slot_rect(Slot::InputSource).is_some());
}

#[test]
fn apps_shrink_once_and_then_fold_the_rest_into_a_more_cell_above_the_trash() {
    let mut rail = Rail::new(Scheme::Light, 600, numbered(14));
    rail.frame(1.0);
    let rects = rail.state.borrow().icon_rects();
    let region = rail.view.apps_region(600.0);
    assert!(rects.iter().all(|(_, rect)| rect.width == ICON - 4.0));
    assert!(rects.len() < 14);
    assert!(
        rects
            .iter()
            .all(|(_, rect)| rect.y >= region.top && rect.y + rect.height <= region.bottom)
    );
    let shown = rects.len();
    let last = rects.last().map(|(_, rect)| *rect).expect("an app");
    let more = Point::new(bar_width(ICON) / 2.0, last.y + last.height + 24.0);
    rail.ui.handle(&Event::PointerDown(Pointer::at(more)));
    let actions = rail.state.borrow_mut().take_actions();
    let Some(DockAction::Menu {
        target: MenuTarget::More(hidden),
        ..
    }) = actions.first()
    else {
        panic!("the more cell opens a list, got {actions:?}");
    };
    assert_eq!(hidden.len(), 14 - shown);
    assert_eq!(
        hidden.first().map(String::as_str),
        Some(format!("org.example.App{shown}").as_str())
    );
}

#[test]
fn every_status_cell_is_a_named_button() {
    let mut rail = Rail::new(Scheme::Light, HEIGHT, apps(1.0));
    rail.frame(1.0);
    let tree = rail.view.accessibility();
    let labels: Vec<String> = tree
        .nodes
        .iter()
        .skip(1)
        .map(|(_, node)| node.label().unwrap_or_default().to_owned())
        .collect();
    assert!(labels.contains(&"Open the launcher".to_owned()));
    assert!(labels.contains(&"Control Center".to_owned()));
    assert!(labels.contains(&"14:32 Thu 29, unread notifications".to_owned()));
    assert!(labels.contains(&"Desktop 2".to_owned()));
}

#[test]
fn the_battery_percentage_is_drawn_inside_the_battery_cell_only_when_asked() {
    let mut rail = Rail::new(Scheme::Light, HEIGHT, apps(1.0));
    let plain = rail.frame(1.0);
    assert!(!rail.view.battery_percentage());
    assert!(rail.view.set_battery_percentage(true));
    let with_percent = rail.frame(1.0);
    let cell = rail.view.slot_rect(Slot::Battery).expect("battery cell");
    let width = rail.size().0 as usize;
    let changed: Vec<Point> = plain
        .as_chunks::<4>()
        .0
        .iter()
        .zip(with_percent.as_chunks::<4>().0)
        .enumerate()
        .filter(|(_, (before, after))| before != after)
        .map(|(index, _)| Point::new((index % width) as f32, (index / width) as f32))
        .collect();
    assert!(!changed.is_empty());
    assert!(changed.iter().all(|point| {
        point.x >= cell.x
            && point.x < cell.x + cell.width
            && point.y >= cell.y
            && point.y < cell.y + cell.height
    }));
    assert!(!rail.view.set_battery_percentage(true));
}

#[test]
fn the_header_names_the_active_app_under_the_mark_and_dims_on_other_displays() {
    let mut rail = Rail::new(Scheme::Light, HEIGHT, apps(1.0));
    rail.frame(1.0);
    let mark = rail.view.slot_rect(Slot::System).expect("mark");
    let header = rail.view.slot_rect(Slot::App).expect("header");
    assert_eq!(header.y, mark.y + mark.height);
    assert!(rail.view.apps_region(HEIGHT as f32).top > header.y + header.height);
    let point = Point::new(
        header.x + header.width / 2.0,
        header.y + header.height / 2.0,
    );
    assert_eq!(
        rail.view.slot_at(point, PointerButton::Primary),
        Some(Slot::App)
    );
    assert!(rail.view.cascades());

    let lit = rail.frame(1.0);
    assert!(rail.view.set_key_display(false));
    let dimmed = rail.frame(1.0);
    assert_ne!(lit, dimmed);
    rail.view.set_app("");
    rail.frame(1.0);
    assert!(rail.view.slot_rect(Slot::App).is_none());
}

#[test]
fn an_auto_hidden_bar_reveals_on_an_edge_push_and_waits_longer_over_full_screen() {
    let mut rail = Rail::new(Scheme::Light, HEIGHT, apps(1.0));
    rail.frame(1.0);
    let now = rail.clock.get();
    rail.state.borrow_mut().set_auto_hide(true, now);
    rail.advance(Duration::from_secs(1));
    rail.frame(1.0);
    assert_eq!(rail.state.borrow().exclusive_zone(), 0);
    assert_eq!(rail.state.borrow().input_region().width, 2.0);
    let edge = Point::new(0.0, 300.0);
    rail.ui.handle(&Event::PointerMove(Pointer::at(edge)));
    rail.advance(Duration::from_millis(150));
    rail.advance(Duration::from_secs(1));
    assert!(rail.state.borrow().is_shown());
    assert_eq!(rail.state.borrow().input_region().width, bar_width(ICON));
    rail.ui.handle(&Event::PointerLeave);
    rail.advance(Duration::from_millis(500));
    rail.advance(Duration::from_secs(1));
    assert!(!rail.state.borrow().is_shown());

    let now = rail.clock.get();
    rail.state.borrow_mut().set_auto_hide(false, now);
    rail.state.borrow_mut().set_full_screen(true, now);
    rail.advance(Duration::from_secs(1));
    rail.ui.handle(&Event::PointerMove(Pointer::at(edge)));
    rail.advance(Duration::from_millis(150));
    assert!(!rail.state.borrow().is_shown());
    rail.advance(Duration::from_millis(200));
    assert!(rail.state.borrow().is_shown());
}

#[test]
fn the_expanded_bar_matches_its_golden() {
    let mut rail = Rail::new(Scheme::Light, HEIGHT, apps(1.0));
    rail.view.set_expanded(true);
    rail.view.set_app_menus(
        ["File", "Edit", "View", "Go", "Window", "Help"]
            .map(str::to_owned)
            .to_vec(),
    );
    rail.state
        .borrow_mut()
        .set_rail(rail.view.apps_region(HEIGHT as f32));
    rail.state.borrow_mut().update_launcher(
        "org.bande.Terminal",
        LauncherUpdate {
            count: Some(3),
            count_visible: Some(true),
            ..LauncherUpdate::default()
        },
        rail.clock.get(),
    );
    rail.frame(1.0);
    rail.advance(Duration::from_secs(2));
    let frame = rail.frame(1.0);
    check_golden("bar-expanded-light", rail.size(), frame);
}

#[test]
fn the_expanded_bar_lists_the_menus_as_rows_that_open_their_menu() {
    let mut rail = Rail::new(Scheme::Light, HEIGHT, apps(1.0));
    rail.view.set_expanded(true);
    rail.view
        .set_app_menus(["File", "Edit"].map(str::to_owned).to_vec());
    rail.frame(1.0);
    let edit = rail.view.slot_rect(Slot::AppMenu(1)).expect("the Edit row");
    let header = rail.view.slot_rect(Slot::App).expect("the header");
    assert!(edit.y > header.y + header.height);
    assert_eq!(
        rail.view.slot_at(center(edit), PointerButton::Primary),
        Some(Slot::AppMenu(1))
    );
    assert!(rail.view.apps_region(HEIGHT as f32).top > edit.y + edit.height);
    assert_eq!(rail.view.width(), 248.0);
    let search = rail
        .view
        .slot_rect(Slot::Launcher)
        .expect("the search field");
    assert_eq!(
        rail.view.slot_at(center(search), PointerButton::Primary),
        Some(Slot::Launcher)
    );
}

fn menu_node(id: i32, label: &str, children: Vec<Node>) -> Node {
    Node {
        id,
        label: label.to_owned(),
        enabled: true,
        visible: true,
        children,
        ..Node::default()
    }
}

fn files_menus() -> Node {
    let leaf = |id, label: &str| menu_node(id, label, Vec::new());
    let shortcut = |node: Node, keys: &str| Node {
        shortcut: Some(keys.to_owned()),
        ..node
    };
    menu_node(
        0,
        "",
        vec![
            menu_node(
                1,
                "Files",
                vec![
                    leaf(10, "About Files"),
                    shortcut(leaf(11, "Quit Files"), "Ctrl+Q"),
                ],
            ),
            menu_node(2, "File", vec![shortcut(leaf(20, "New Window"), "Ctrl+N")]),
            menu_node(
                3,
                "Edit",
                vec![
                    shortcut(leaf(30, "Copy"), "Ctrl+C"),
                    shortcut(leaf(31, "Find"), "Ctrl+F"),
                    shortcut(leaf(32, "Find Next"), "Ctrl+G"),
                ],
            ),
            menu_node(4, "View", vec![leaf(40, "Find in Sidebar")]),
            menu_node(5, "Go", vec![leaf(50, "Home")]),
            menu_node(6, "Window", vec![leaf(60, "Minimise")]),
            menu_node(7, "Help", vec![leaf(70, "Files Help")]),
        ],
    )
}

fn open_cascade(query: &str) -> (Vec<u8>, (u32, u32)) {
    let scheme = Scheme::Light;
    let mut rail = Rail::new(scheme, HEIGHT, apps(1.0));
    rail.frame(1.0);
    let header = rail.view.slot_rect(Slot::App).expect("the header");
    rail.view.set_hovered(Some(Slot::App));
    rail.view.set_open(Some(Slot::App));
    let palette = Palette::for_scheme(scheme);
    let mut text = TextSystem::new();
    let root = files_menus();
    let menu = cascade(&AppMenus {
        name: "Files",
        root: Some(&root),
        item: ":1.7/org/bande/MenuBar",
        actions: &[],
        windows: &[],
    });
    let mut tracker = MenuTracker::new(MenuPanel::new(menu, &mut text, &palette), false);
    for character in query.chars() {
        match tracker.key(
            Key::Character(character),
            Modifiers::default(),
            Instant::now(),
        ) {
            MenuOutcome::Search(found) => {
                tracker.searched(MenuPanel::new(found, &mut text, &palette));
            }
            outcome => panic!("typing searched nothing: {outcome:?}"),
        }
    }
    let panel = &tracker.panels()[0];
    let MenuPlacement::AtPoint(at) = rail.view.menu_placement(header) else {
        panic!("the cascade opens beside the header");
    };
    let size = (360, 360);
    let bar = rail.view.scene(size.0, size.1, 1.0);
    let outline = Shape::rounded(
        Rect::new(0.0, 0.0, panel.size().width, panel.size().height),
        bande_toolkit::ui::MENU_RADIUS,
    );
    let mut menu = Item::shadows(outline, Elevation::Overlay, scheme, true).to_vec();
    menu.extend(panel.paint(&palette, true));
    let mut items = vec![Item::Fill {
        shape: Shape::rect(Rect::new(0.0, 0.0, size.0 as f32, size.1 as f32)),
        paint: Color::hex(0x808080).into(),
    }];
    items.extend(bar.items);
    items.push(Item::Translate {
        offset: at,
        items: menu,
    });
    let scene = Scene {
        width: size.0,
        height: size.1,
        background: Color::TRANSPARENT,
        items,
    };
    let mut rgba = vec![0; (size.0 * size.1 * 4) as usize];
    CpuRenderer::new()
        .render_into(&scene, 1.0, &mut rgba)
        .expect("frame");
    (rgba, size)
}

#[test]
fn the_open_cascade_keeps_the_pressed_header_and_offers_type_to_search() {
    for (query, name) in [("", "cascade-light"), ("fin", "cascade-light-searching")] {
        let (rgba, size) = open_cascade(query);
        check_golden(name, size, rgba);
    }
}

#[test]
fn the_bluetooth_and_wifi_menus_show_batteries_forget_and_sign_in() {
    let scheme = Scheme::Light;
    let palette = Palette::for_scheme(scheme);
    let mut text = TextSystem::new();
    let gadget = |address: &str, name: &str, connected, battery| Gadget {
        address: address.to_owned(),
        name: name.to_owned(),
        connected,
        battery,
    };
    let menus = [
        bluetooth_menu(&Bluetooth {
            powered: true,
            devices: vec![
                gadget("00:11", "Studio Headphones", true, Some(80)),
                gadget("00:22", "Mouse", false, None),
            ],
        }),
        wifi_menu(&Wifi {
            enabled: true,
            networks: Vec::new(),
            sign_in: true,
        }),
    ];
    let size = (560, 240);
    let mut items = vec![Item::Fill {
        shape: Shape::rect(Rect::new(0.0, 0.0, size.0 as f32, size.1 as f32)),
        paint: Color::hex(0x808080).into(),
    }];
    let mut left = 16.0;
    for menu in menus {
        let panel = MenuPanel::new(menu, &mut text, &palette);
        let outline = Shape::rounded(
            Rect::new(0.0, 0.0, panel.size().width, panel.size().height),
            bande_toolkit::ui::MENU_RADIUS,
        );
        let mut drawn = Item::shadows(outline, Elevation::Overlay, scheme, true).to_vec();
        drawn.extend(panel.paint(&palette, false));
        items.push(Item::Translate {
            offset: Point::new(left, 16.0),
            items: drawn,
        });
        left += panel.size().width + 24.0;
    }
    let scene = Scene {
        width: size.0,
        height: size.1,
        background: Color::TRANSPARENT,
        items,
    };
    let mut rgba = vec![0; (size.0 * size.1 * 4) as usize];
    CpuRenderer::new()
        .render_into(&scene, 1.0, &mut rgba)
        .expect("frame");
    check_golden("status-menus-light", size, rgba);
}

#[test]
fn the_keyboard_reaches_the_more_cell_between_the_apps_and_the_trash_and_opens_it() {
    let mut rail = Rail::new(Scheme::Light, 600, numbered(14));
    rail.frame(1.0);
    let now = rail.clock.get();
    let plain = Modifiers::default();
    let mut state = rail.state.borrow_mut();
    state.start_navigation(None, now);
    assert!(state.navigate(Key::End, plain, now));
    assert!(state.navigate(Key::Up, plain, now));
    assert!(state.navigate(Key::Enter, plain, now));
    let actions = state.take_actions();
    let [
        DockAction::Menu {
            target: MenuTarget::More(hidden),
            ..
        },
    ] = actions.as_slice()
    else {
        panic!("Enter on the more cell opens its list and keeps the keyboard, got {actions:?}");
    };
    assert!(!hidden.is_empty());
    assert!(state.is_navigating());
}

fn shown_apps(rail: &Rail) -> Vec<(String, Rect)> {
    rail.state.borrow().icon_rects()
}

#[test]
fn an_app_leaving_a_full_rail_frees_room_before_the_next_one_comes_out_of_more() {
    let height = 600;
    let shown = {
        let mut rail = Rail::new(Scheme::Light, height, numbered(14));
        rail.frame(1.0);
        shown_apps(&rail).len()
    };
    let mut rail = Rail::new(Scheme::Light, height, numbered(shown + 2));
    rail.frame(1.0);
    assert_eq!(shown_apps(&rail).len(), shown);
    let region = rail.view.apps_region(height as f32);
    let x = bar_width(ICON) / 2.0;
    let trash_top = (region.top as i32..region.bottom as i32)
        .map(|y| y as f32)
        .find(|y| rail.state.borrow().drop_target(Point::new(x, *y)) == Some(MenuTarget::Trash))
        .expect("the trash");
    let start = rail.clock.get();
    rail.state
        .borrow_mut()
        .set_apps(numbered(shown + 2).split_off(1), start);
    let mut before: Vec<String> = Vec::new();
    for frame in 0..40 {
        rail.clock.set(start + Duration::from_millis(16 * frame));
        rail.frame(1.0);
        let now = shown_apps(&rail);
        let names: Vec<String> = now.iter().map(|(id, _)| id.clone()).collect();
        assert!(
            before.iter().all(|id| names.contains(id)),
            "frame {frame}: {before:?} went back into More as {names:?}"
        );
        assert!(
            now.iter()
                .all(|(_, rect)| rect.y + rect.height <= trash_top),
            "frame {frame}: an app is drawn over the trash: {now:?} below {trash_top}"
        );
        before = names;
    }
    assert_eq!(before.len(), shown + 1);
    assert!(before.contains(&format!("org.example.App{}", shown + 1)));
}

#[test]
fn an_app_using_the_microphone_and_camera_marks_control_center_and_names_itself() {
    let mut rail = Rail::new(Scheme::Light, HEIGHT, apps(1.0));
    rail.view.status_mut().capturing = vec![
        Capture {
            name: "Meet".to_owned(),
            icon: None,
            senses: vec![Sense::Microphone, Sense::Camera],
        },
        Capture {
            name: "Recorder".to_owned(),
            icon: None,
            senses: vec![Sense::Microphone],
        },
    ];
    rail.frame(1.0);
    rail.advance(Duration::from_secs(2));
    let frame = rail.frame(1.0);
    check_golden("bar-in-use-light", rail.size(), frame);

    let tree = rail.view.accessibility();
    let labels: Vec<String> = tree
        .nodes
        .iter()
        .map(|(_, node)| node.label().unwrap_or_default().to_owned())
        .collect();
    assert!(labels.contains(
        &"Control Center, Meet is using the microphone and camera, Recorder is using the microphone"
            .to_owned()
    ));
}
