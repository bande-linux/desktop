#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::Role;
use unbound_config::DockEdge;
use unbound_dock::bus::{app_id_from_uri, launcher_update};
use unbound_dock::{
    DockAction, DockApp, DockState, ICON_SIZE, LauncherUpdate, MenuTarget, Shared, arrange,
    dock_ui, scene, surface_size,
};
use unbound_toolkit::ui::{Event, Key, Modifiers, Pointer, PointerButton, Ui};
use unbound_toolkit::{Appearance, CpuRenderer, Image, Palette, Point, Rect, Scheme, TextSystem};
use zbus::zvariant::{OwnedValue, Value};

const OUTPUT: (u32, u32) = (1280, 480);
const MIN_SSIM: f64 = 0.99;
const WALLPAPER: u8 = 0x80;

fn blue_icon() -> Image {
    let size = ICON_SIZE as u32;
    let pixels = (0..size * size)
        .flat_map(|index| {
            let y = index / size;
            [0x20, (0x60 + y * 2) as u8, 0xE0, 0xFF]
        })
        .collect();
    Image::from_rgba(size, size, pixels).expect("icon")
}

fn app(app_id: &str, name: &str, kept: bool, running: bool) -> DockApp {
    DockApp {
        app_id: app_id.to_owned(),
        name: name.to_owned(),
        icon: (app_id == "org.unbound.Terminal").then(blue_icon),
        kept,
        running,
    }
}

fn apps() -> Vec<DockApp> {
    vec![
        app("org.unbound.Terminal", "Terminal", true, true),
        app("org.example.Editor", "Editor", false, true),
    ]
}

struct Harness {
    state: Shared,
    ui: Ui,
    clock: Rc<Cell<Instant>>,
    size: (u32, u32),
}

impl Harness {
    fn new(edge: DockEdge, scheme: Scheme) -> Self {
        let state = DockState::new(edge, ICON_SIZE);
        let appearance = Appearance {
            scheme,
            accent: None,
            reduce_motion: false,
        };
        state.borrow_mut().set_appearance(appearance);
        let clock = Rc::new(Cell::new(Instant::now()));
        state.borrow_mut().set_apps(apps(), clock.get());
        let mut ui = dock_ui(
            state.clone(),
            Palette::for_scheme(scheme),
            TextSystem::new(),
        );
        let time = clock.clone();
        ui.set_clock(move || time.get());
        let mut harness = Self {
            state,
            ui,
            clock,
            size: surface_size(edge, ICON_SIZE, OUTPUT),
        };
        harness.frame();
        harness
    }

    fn frame(&mut self) -> Vec<u8> {
        let scene = scene(&mut self.ui, self.size);
        CpuRenderer::new().render(&scene).expect("frame").rgba
    }

    fn advance(&mut self, by: Duration) {
        self.clock.set(self.clock.get() + by);
        self.state.borrow_mut().tick(self.clock.get());
        self.frame();
    }

    fn rect(&self, app_id: &str) -> Rect {
        self.state
            .borrow()
            .icon_rects()
            .into_iter()
            .find(|(known, _)| known == app_id)
            .map(|(_, rect)| rect)
            .unwrap_or_else(|| panic!("no icon for {app_id}"))
    }

    fn center(&self, app_id: &str) -> Point {
        let rect = self.rect(app_id);
        Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
    }

    fn click(&mut self, point: Point) {
        self.ui.handle(&Event::PointerDown(Pointer::at(point)));
        self.ui.handle(&Event::PointerUp(Pointer::at(point)));
    }

    fn drag(&mut self, from: Point, to: Point) {
        self.ui.handle(&Event::PointerDown(Pointer::at(from)));
        for step in 1..=8 {
            let along = step as f32 / 8.0;
            let point = Point::new(
                from.x + (to.x - from.x) * along,
                from.y + (to.y - from.y) * along,
            );
            self.ui.handle(&Event::PointerMove(Pointer::at(point)));
            self.advance(Duration::from_millis(16));
        }
        self.ui.handle(&Event::PointerUp(Pointer::at(to)));
        self.advance(Duration::from_secs(2));
    }

    fn buttons(&self) -> Vec<(String, String)> {
        self.ui
            .accessibility()
            .nodes
            .iter()
            .filter(|(_, node)| node.role() == Role::Button)
            .map(|(_, node)| {
                (
                    node.label().unwrap_or_default().to_owned(),
                    node.description().unwrap_or_default().to_owned(),
                )
            })
            .collect()
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

#[test]
fn the_dock_matches_its_goldens_on_each_edge_in_light_and_dark() {
    for (edge, scheme, name) in [
        (DockEdge::Left, Scheme::Light, "dock-left-light"),
        (DockEdge::Left, Scheme::Dark, "dock-left-dark"),
        (DockEdge::Bottom, Scheme::Light, "dock-bottom-light"),
    ] {
        let mut harness = Harness::new(edge, scheme);
        harness.state.borrow_mut().update_launcher(
            "org.unbound.Terminal",
            LauncherUpdate {
                count: Some(3),
                count_visible: Some(true),
                progress: Some(0.4),
                progress_visible: Some(true),
                urgent: None,
            },
            harness.clock.get(),
        );
        harness.advance(Duration::from_secs(2));
        let frame = harness.frame();
        check_golden(name, harness.size, frame);
    }
}

#[test]
fn a_click_opens_the_app_or_the_trash_and_a_drag_off_cancels() {
    let mut harness = Harness::new(DockEdge::Left, Scheme::Light);
    let editor = harness.center("org.example.Editor");
    let terminal = harness.center("org.unbound.Terminal");
    let trash = {
        let rects = harness.state.borrow().panel();
        Point::new(rects.x + rects.width / 2.0, rects.y + rects.height - 30.0)
    };

    harness.click(editor);
    harness
        .ui
        .handle(&Event::PointerDown(Pointer::at(terminal)));
    harness
        .ui
        .handle(&Event::PointerMove(Pointer::at(Point::new(
            terminal.x + 3.0,
            terminal.y + 1.0,
        ))));
    harness.ui.handle(&Event::PointerUp(Pointer::at(Point::new(
        terminal.x + 3.0,
        terminal.y + 1.0,
    ))));
    harness.drag(editor, Point::new(200.0, editor.y));
    harness.click(trash);

    assert_eq!(
        harness.state.borrow_mut().take_actions(),
        [
            DockAction::Open("org.example.Editor".to_owned()),
            DockAction::Open("org.unbound.Terminal".to_owned()),
            DockAction::OpenTrash
        ]
    );
}

fn three_apps() -> Vec<DockApp> {
    vec![
        app("org.unbound.Terminal", "Terminal", true, true),
        app("org.unbound.Files", "Files", true, false),
        app("org.example.Editor", "Editor", false, true),
    ]
}

fn kept(ids: &[&str]) -> DockAction {
    DockAction::Keep(ids.iter().map(|id| (*id).to_owned()).collect())
}

#[test]
fn dragging_a_kept_icon_reorders_the_dock_and_its_neighbours_slide_aside() {
    let mut harness = Harness::new(DockEdge::Left, Scheme::Light);
    let now = harness.clock.get();
    harness.state.borrow_mut().set_apps(three_apps(), now);
    harness.advance(Duration::from_secs(2));
    let terminal = harness.rect("org.unbound.Terminal");
    let files = harness.center("org.unbound.Files");

    harness.ui.handle(&Event::PointerDown(Pointer::at(files)));
    let above = Point::new(files.x, terminal.y);
    harness.ui.handle(&Event::PointerMove(Pointer::at(above)));
    assert!(harness.state.borrow().is_dragging());
    harness.advance(Duration::from_millis(40));
    assert!(harness.rect("org.unbound.Terminal").y > terminal.y);
    harness.ui.handle(&Event::PointerUp(Pointer::at(above)));

    assert_eq!(
        harness.state.borrow_mut().take_actions(),
        [kept(&["org.unbound.Files", "org.unbound.Terminal"])]
    );
    harness.advance(Duration::from_secs(2));
    assert!(harness.rect("org.unbound.Files").y < harness.rect("org.unbound.Terminal").y);
}

#[test]
fn dragging_a_kept_icon_off_the_dock_past_the_threshold_removes_it() {
    let mut harness = Harness::new(DockEdge::Left, Scheme::Light);
    let now = harness.clock.get();
    harness.state.borrow_mut().set_apps(three_apps(), now);
    harness.advance(Duration::from_secs(2));
    let files = harness.center("org.unbound.Files");

    harness.drag(files, Point::new(100.0, files.y));
    assert_eq!(
        harness.state.borrow_mut().take_actions(),
        [],
        "a short drag sideways only reorders, and there was nothing to reorder"
    );

    harness.drag(files, Point::new(200.0, files.y));
    assert_eq!(
        harness.state.borrow_mut().take_actions(),
        [kept(&["org.unbound.Terminal"])]
    );
    let mut apps = three_apps();
    apps.remove(1);
    let now = harness.clock.get();
    harness.state.borrow_mut().set_apps(apps, now);
    harness.advance(Duration::from_secs(2));
    assert_eq!(harness.state.borrow().icon_rects().len(), 2);

    let terminal = harness.center("org.unbound.Terminal");
    harness.drag(terminal, Point::new(200.0, terminal.y));
    assert_eq!(harness.state.borrow_mut().take_actions(), [kept(&[])]);
}

#[test]
fn dragging_a_running_app_into_the_kept_group_keeps_it() {
    let mut harness = Harness::new(DockEdge::Left, Scheme::Light);
    let now = harness.clock.get();
    harness.state.borrow_mut().set_apps(three_apps(), now);
    harness.advance(Duration::from_secs(2));
    let editor = harness.center("org.example.Editor");
    let terminal = harness.center("org.unbound.Terminal");

    harness.drag(editor, Point::new(editor.x, terminal.y - 10.0));
    assert_eq!(
        harness.state.borrow_mut().take_actions(),
        [kept(&[
            "org.example.Editor",
            "org.unbound.Terminal",
            "org.unbound.Files"
        ])]
    );
}

#[test]
fn kept_apps_that_are_not_installed_keep_their_place_when_the_dock_is_rearranged() {
    let kept = ["a", "gone", "b", "c"].map(str::to_owned);
    let shown = ["c", "a", "b"].map(str::to_owned);
    assert_eq!(
        arrange(&kept, &shown, |app_id| app_id == "gone"),
        ["c", "a", "gone", "b"]
    );
    assert_eq!(
        arrange(&kept, &["b".to_owned()], |app_id| app_id == "gone"),
        ["gone", "b"]
    );
}

#[test]
fn moving_the_dock_to_another_edge_slides_it_in_there() {
    let mut harness = Harness::new(DockEdge::Left, Scheme::Light);
    let now = harness.clock.get();
    harness.state.borrow_mut().set_edge(DockEdge::Bottom, now);
    harness.size = surface_size(DockEdge::Bottom, ICON_SIZE, OUTPUT);
    harness.advance(Duration::from_millis(30));
    let entering = harness.state.borrow().panel();
    harness.advance(Duration::from_secs(2));
    let panel = harness.state.borrow().panel();
    assert!(panel.width > panel.height);
    assert!(entering.y > panel.y);
    assert_eq!(panel.y + panel.height, harness.size.1 as f32 - 6.0);
}

#[test]
fn every_icon_is_a_labelled_button_that_tells_its_state() {
    let harness = Harness::new(DockEdge::Left, Scheme::Light);
    harness.state.borrow_mut().update_launcher(
        "org.example.Editor",
        launcher_update(&HashMap::from([
            ("count".to_owned(), OwnedValue::from(120_i64)),
            ("count-visible".to_owned(), OwnedValue::from(true)),
        ])),
        harness.clock.get(),
    );
    harness.state.borrow_mut().set_trash_full(true);

    assert_eq!(
        harness.buttons(),
        [
            ("Terminal".to_owned(), "running".to_owned()),
            ("Editor".to_owned(), "running, 120 new".to_owned()),
            ("Trash".to_owned(), "contains items".to_owned()),
        ]
    );
}

#[test]
fn a_launching_app_pulses_until_its_window_maps_or_ten_seconds_pass() {
    let mut harness = Harness::new(DockEdge::Left, Scheme::Light);
    let mut stopped = apps();
    stopped[0].running = false;
    let now = harness.clock.get();
    harness.state.borrow_mut().set_apps(stopped.clone(), now);
    harness
        .state
        .borrow_mut()
        .launching("org.unbound.Terminal", now);
    assert!(harness.state.borrow().pulsing(now));
    assert_eq!(harness.buttons()[0].1, "opening");

    harness.state.borrow_mut().set_apps(apps(), now);
    assert!(!harness.state.borrow().pulsing(now));

    harness
        .state
        .borrow_mut()
        .launching("org.unbound.Terminal", now);
    harness.state.borrow_mut().set_apps(stopped, now);
    harness.advance(Duration::from_secs(11));
    assert!(!harness.state.borrow().pulsing(harness.clock.get()));
    assert_eq!(harness.buttons()[0].1, "");
}

#[test]
fn neighbours_slide_into_the_gap_a_quitting_app_leaves() {
    let mut harness = Harness::new(DockEdge::Left, Scheme::Light);
    let before = harness.rect("org.unbound.Terminal");
    let mut quit = apps();
    quit.truncate(1);
    harness
        .state
        .borrow_mut()
        .set_apps(quit, harness.clock.get());
    harness.advance(Duration::from_millis(60));
    let moving = harness.rect("org.unbound.Terminal");
    harness.advance(Duration::from_secs(2));
    let after = harness.rect("org.unbound.Terminal");

    assert!(moving.y > before.y && moving.y < after.y);
    assert!(after.y > before.y);
    assert_eq!(harness.state.borrow().icon_rects().len(), 1);
    assert_eq!(after.x, 6.0 + 8.0);
}

#[test]
fn badges_follow_launcher_entry_updates_as_kde_reads_them() {
    assert_eq!(
        app_id_from_uri("application://org.mozilla.firefox.desktop").as_deref(),
        Some("org.mozilla.firefox")
    );
    let harness = Harness::new(DockEdge::Left, Scheme::Light);
    let now = harness.clock.get();
    let update = |pairs: &[(&str, Value<'static>)]| {
        launcher_update(
            &pairs
                .iter()
                .map(|(key, value)| {
                    (
                        (*key).to_owned(),
                        OwnedValue::try_from(value.clone()).expect("value"),
                    )
                })
                .collect(),
        )
    };
    let mut state = harness.state.borrow_mut();
    state.update_launcher(
        "org.example.Editor",
        update(&[
            ("count", Value::from(4_i64)),
            ("count-visible", Value::from(true)),
        ]),
        now,
    );
    state.update_launcher(
        "org.example.Editor",
        update(&[
            ("progress", Value::from(0.25)),
            ("progress-visible", Value::from(true)),
        ]),
        now,
    );
    drop(state);
    assert_eq!(harness.buttons()[1].1, "running, 4 new, 25 % done");

    harness.state.borrow_mut().update_launcher(
        "org.example.Editor",
        update(&[
            ("count-visible", Value::from(false)),
            ("urgent", Value::from(true)),
        ]),
        now,
    );
    assert_eq!(
        harness.buttons()[1].1,
        "running, 25 % done, needs attention"
    );
    harness.state.borrow_mut().activated("org.example.Editor");
    assert_eq!(harness.buttons()[1].1, "running, 25 % done");
}

#[test]
fn right_click_or_holding_an_icon_opens_its_menu_beside_the_dock_without_opening_the_app() {
    let mut harness = Harness::new(DockEdge::Left, Scheme::Light);
    let editor = harness.center("org.example.Editor");
    let panel = harness.state.borrow().panel();
    let icon = harness.rect("org.example.Editor");

    harness.ui.handle(&Event::PointerDown(
        Pointer::at(editor).with_button(PointerButton::Secondary),
    ));
    harness.ui.handle(&Event::PointerUp(
        Pointer::at(editor).with_button(PointerButton::Secondary),
    ));
    harness.ui.handle(&Event::PointerDown(Pointer::at(editor)));
    harness.advance(Duration::from_millis(600));
    harness.ui.handle(&Event::PointerUp(Pointer::at(editor)));

    let menu = DockAction::Menu {
        target: MenuTarget::App("org.example.Editor".to_owned()),
        at: Point::new(panel.x + panel.width + 6.0, icon.y),
    };
    assert_eq!(
        harness.state.borrow_mut().take_actions(),
        [menu.clone(), menu]
    );
}

#[test]
fn an_auto_hidden_dock_slides_in_on_an_edge_push_and_out_after_the_pointer_leaves() {
    let mut harness = Harness::new(DockEdge::Left, Scheme::Light);
    let now = harness.clock.get();
    harness.state.borrow_mut().set_auto_hide(true, now);
    harness.advance(Duration::from_secs(1));
    let strip = harness.state.borrow().input_region();
    assert_eq!((strip.x, strip.width), (0.0, 2.0));
    assert_eq!(harness.state.borrow().exclusive_zone(), 0);
    assert!(harness.state.borrow().panel().x + harness.state.borrow().panel().width < 0.0);

    let edge = Point::new(0.0, strip.y + strip.height / 2.0);
    harness.ui.handle(&Event::PointerMove(Pointer::at(edge)));
    harness.advance(Duration::from_millis(60));
    assert!(!harness.state.borrow().is_shown());
    harness.advance(Duration::from_millis(100));
    harness.advance(Duration::from_secs(1));
    assert!(harness.state.borrow().is_shown());
    assert_eq!(harness.state.borrow().panel().x, 6.0);
    let shown = harness.state.borrow().input_region();
    assert_eq!((shown.x, shown.width), (0.0, 6.0 + 64.0));

    harness.ui.handle(&Event::PointerLeave);
    harness.advance(Duration::from_millis(300));
    assert!(harness.state.borrow().is_shown());
    harness.advance(Duration::from_millis(200));
    assert!(!harness.state.borrow().is_shown());
}

#[test]
fn the_blur_behind_the_dock_stays_inside_its_rounded_corners() {
    let harness = Harness::new(DockEdge::Bottom, Scheme::Light);
    let state = harness.state.borrow();
    let panel = state.panel();
    let radius = ((ICON_SIZE + 16.0) * 0.3).round();
    let inside = |x: f32, y: f32| {
        let cx = x.clamp(panel.x + radius, panel.x + panel.width - radius);
        let cy = y.clamp(panel.y + radius, panel.y + panel.height - radius);
        (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius + 0.01
    };
    let rects = state.blur_rects(false);
    for rect in &rects {
        for (x, y) in [
            (rect.x, rect.y),
            (rect.x + rect.width, rect.y),
            (rect.x, rect.y + rect.height),
            (rect.x + rect.width, rect.y + rect.height),
        ] {
            assert!(inside(x, y), "{rect:?} leaves the panel {panel:?}");
        }
    }
    let covered: f32 = rects.iter().map(|rect| rect.width * rect.height).sum();
    assert!(covered > 0.95 * panel.width * panel.height);
}

#[test]
fn the_icon_size_setting_resizes_the_dock_within_sensible_bounds() {
    let mut harness = Harness::new(DockEdge::Left, Scheme::Light);
    assert!(harness.state.borrow_mut().set_icon_size(64.0));
    assert!(!harness.state.borrow_mut().set_icon_size(64.0));
    harness.size = surface_size(DockEdge::Left, 64.0, OUTPUT);
    harness.frame();
    assert_eq!(harness.rect("org.unbound.Terminal").width, 64.0);
    assert_eq!(harness.state.borrow().panel().width, 80.0);
    assert_eq!(harness.state.borrow().exclusive_zone(), 92);

    harness.state.borrow_mut().set_icon_size(400.0);
    assert_eq!(harness.state.borrow().icon_size(), 96.0);
}

#[test]
fn super_f3_lets_the_keyboard_walk_the_dock_open_apps_and_menus_and_give_the_keyboard_back() {
    let harness = Harness::new(DockEdge::Left, Scheme::Light);
    let now = harness.clock.get();
    let plain = Modifiers::default();
    let mut state = harness.state.borrow_mut();
    state.start_navigation(Some("org.unbound.Terminal"), now);
    assert!(state.is_navigating());
    assert!(state.navigate(Key::Down, plain, now));
    assert!(state.navigate(Key::Enter, plain, now));
    assert_eq!(
        state.take_actions(),
        [
            DockAction::Open("org.example.Editor".to_owned()),
            DockAction::ReleaseKeyboard
        ]
    );
    assert!(state.navigate(Key::End, plain, now));
    assert!(state.navigate(Key::Enter, plain, now));
    assert!(state.navigate(Key::Up, plain, now));
    assert!(state.navigate(
        Key::Function(10),
        Modifiers {
            shift: true,
            ..plain
        },
        now
    ));
    assert!(state.navigate(Key::Escape, plain, now));
    let actions = state.take_actions();
    assert_eq!(
        actions[..2],
        [DockAction::OpenTrash, DockAction::ReleaseKeyboard]
    );
    assert!(matches!(
        &actions[2],
        DockAction::Menu {
            target: MenuTarget::App(app_id),
            ..
        } if app_id == "org.example.Editor"
    ));
    assert_eq!(actions[3], DockAction::ReleaseKeyboard);
    state.end_navigation(now);
    assert!(!state.is_navigating());
    assert!(!state.navigate(Key::Down, plain, now));
}

#[test]
fn files_dragged_over_an_icon_find_their_target_and_grow_it() {
    let mut harness = Harness::new(DockEdge::Left, Scheme::Light);
    let editor = harness.center("org.example.Editor");
    let panel = harness.state.borrow().panel();
    let trash = Point::new(panel.x + panel.width / 2.0, panel.y + panel.height - 30.0);
    let state = harness.state.borrow();
    assert_eq!(
        state.drop_target(Point::new(1.0, editor.y)),
        Some(MenuTarget::App("org.example.Editor".to_owned()))
    );
    assert_eq!(state.drop_target(trash), Some(MenuTarget::Trash));
    assert_eq!(state.drop_target(Point::new(200.0, editor.y)), None);
    drop(state);

    let before = harness.rect("org.example.Editor");
    let now = harness.clock.get();
    harness
        .state
        .borrow_mut()
        .hover_drop(Some(&MenuTarget::App("org.example.Editor".to_owned())), now);
    harness.advance(Duration::from_secs(1));
    let frame = harness.frame();
    harness
        .state
        .borrow_mut()
        .hover_drop(None, harness.clock.get());
    harness.advance(Duration::from_secs(1));
    assert_ne!(
        frame,
        harness.frame(),
        "the hovered icon grows, then settles"
    );
    assert_eq!(harness.rect("org.example.Editor"), before);
}
