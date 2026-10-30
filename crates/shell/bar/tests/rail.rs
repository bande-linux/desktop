#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use unbound_bar::dock::{DockAction, DockApp, DockState, MenuTarget, Shared, dock_ui, scene};
use unbound_bar::menubar::{Slot, StatusView};
use unbound_bar::rail::{RailView, bar_width, surface_size};
use unbound_config::DockEdge;
use unbound_datetime::{DateFormats, posix_locale};
use unbound_dock_model::LauncherUpdate;
use unbound_status::status::{
    Battery, Bluetooth, Gadget, InputSource, InputSources, Power, Sound, Wifi,
};
use unbound_toolkit::ui::{Event, Pointer, PointerButton, Ui};
use unbound_toolkit::{Appearance, CpuRenderer, Image, Palette, Point, Rect, Scheme, TextSystem};

const ICON: f32 = 40.0;
const HEIGHT: u32 = 720;
const MIN_SSIM: f64 = 0.99;
const WALLPAPER: u8 = 0x80;

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data")
}

fn app_icon(name: &str, scale: f64) -> Option<Image> {
    let pixels = (f64::from(ICON) * scale).round() as u32;
    Image::load(
        &data().join(format!("icons/Unbound/scalable/apps/{name}.svg")),
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
        app("org.unbound.Files", "Files", true, true, scale),
        app("org.unbound.Terminal", "Terminal", true, true, scale),
        app("org.unbound.Settings", "Settings", true, false, scale),
        app("org.gnome.TextEditor", "Text Editor", false, true, scale),
    ]
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
            &data().join("icons/Unbound/scalable/apps/org.unbound.Files.svg"),
            (24.0 * scale).round() as u32,
        ));
        view.set_workspaces(3, 0);
        let at = "2026-10-29T14:32:00[Europe/Stockholm]"
            .parse()
            .expect("time");
        view.set_time(&at, &DateFormats::for_locale(&posix_locale("en_GB")));
        let state = DockState::new(DockEdge::Left, ICON);
        state.borrow_mut().set_appearance(Appearance {
            scheme,
            accent: None,
            reduce_motion: false,
        });
        state.borrow_mut().set_rail(view.apps_region(height as f32));
        let pixels = (f64::from(ICON) * scale).round() as u32;
        let trash = data().join("icons/Unbound/scalable/places/user-trash.svg");
        state
            .borrow_mut()
            .set_trash_icons(Image::load(&trash, pixels), None);
        let clock = Rc::new(Cell::new(Instant::now()));
        state.borrow_mut().set_apps(apps, clock.get());
        state.borrow_mut().activated("org.unbound.Files");
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

fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    let over_wallpaper = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| {
            let behind = u16::from(255 - pixel[3]) * u16::from(WALLPAPER) / 255;
            [0, 1, 2].map(|channel| (u16::from(pixel[channel]) + behind).min(255) as u8)
        })
        .collect();
    let actual = image::RgbImage::from_raw(size.0, size.1, over_wallpaper).expect("frame");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
    if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some() {
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

#[test]
fn the_combined_bar_matches_its_goldens_in_light_and_dark_at_one_and_one_and_a_half() {
    for (scheme, scale, name) in [
        (Scheme::Light, 1.0, "bar-light"),
        (Scheme::Dark, 1.0, "bar-dark"),
        (Scheme::Light, 1.5, "bar-light-1.5x"),
        (Scheme::Dark, 1.5, "bar-dark-1.5x"),
    ] {
        let mut rail = Rail::at_scale(scheme, HEIGHT, apps(scale), scale);
        rail.state.borrow_mut().update_launcher(
            "org.unbound.Terminal",
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
    assert_eq!(view.shown_tray_items(), 3);
}

#[test]
fn apps_shrink_once_and_then_fold_the_rest_into_a_more_cell_above_the_trash() {
    let many: Vec<DockApp> = (0..14)
        .map(|index| {
            app(
                &format!("org.example.App{index}"),
                &format!("App {index}"),
                true,
                false,
                1.0,
            )
        })
        .collect();
    let mut rail = Rail::new(Scheme::Light, 600, many);
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
