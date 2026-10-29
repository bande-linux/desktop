#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use accesskit::Role;
use unbound_config::DockEdge;
use unbound_dock::{Actions, DockAction, DockApp, ICON_SIZE, dock_size, dock_ui, merge_apps};
use unbound_toolkit::ui::{Event, Pointer};
use unbound_toolkit::{CpuRenderer, Image, Palette, Point, Scheme, TextSystem};

const OUTPUT: (u32, u32) = (1280, 480);
const MIN_SSIM: f64 = 0.99;

fn blue_icon() -> Image {
    let pixels = (0..ICON_SIZE * ICON_SIZE)
        .flat_map(|index| {
            let y = index / ICON_SIZE;
            [0x20, (0x60 + y * 2) as u8, 0xE0, 0xFF]
        })
        .collect();
    Image::from_rgba(ICON_SIZE, ICON_SIZE, pixels).expect("icon")
}

fn apps() -> Vec<DockApp> {
    let pinned = [DockApp {
        app_id: "org.unbound.Terminal".to_owned(),
        name: "Terminal".to_owned(),
        icon: Some(blue_icon()),
        running: false,
    }];
    merge_apps(
        &pinned,
        &[
            "org.unbound.Terminal".to_owned(),
            "org.example.Editor".to_owned(),
        ],
        |app_id| DockApp {
            app_id: app_id.to_owned(),
            name: "Editor".to_owned(),
            icon: None,
            running: false,
        },
    )
}

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"))
}

fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    let actual = image::RgbaImage::from_raw(size.0, size.1, rgba).expect("frame");
    let actual = image::DynamicImage::ImageRgba8(actual).to_rgb8();
    let path = golden(name);
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
fn the_left_dock_matches_its_golden_in_light_and_dark() {
    for (scheme, name) in [
        (Scheme::Light, "dock-left-light"),
        (Scheme::Dark, "dock-left-dark"),
    ] {
        let size = dock_size(DockEdge::Left, OUTPUT);
        let mut ui = dock_ui(
            DockEdge::Left,
            apps(),
            "09:41",
            Palette::for_scheme(scheme),
            TextSystem::new(),
            Rc::new(RefCell::new(Vec::new())),
        );
        let scene = ui.scene(size.0, size.1);
        let frame = CpuRenderer::new().render(&scene).expect("frame");
        check_golden(name, size, frame.rgba);
    }
}

#[test]
fn the_bottom_dock_matches_its_golden() {
    let size = dock_size(DockEdge::Bottom, OUTPUT);
    let mut ui = dock_ui(
        DockEdge::Bottom,
        apps(),
        "09:41",
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        Rc::new(RefCell::new(Vec::new())),
    );
    let scene = ui.scene(size.0, size.1);
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    check_golden("dock-bottom-light", size, frame.rgba);
}

#[test]
fn clicking_an_app_asks_to_open_it_and_a_drag_off_cancels() {
    let actions: Actions = Rc::new(RefCell::new(Vec::new()));
    let size = dock_size(DockEdge::Left, OUTPUT);
    let mut ui = dock_ui(
        DockEdge::Left,
        apps(),
        "09:41",
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        actions.clone(),
    );
    ui.layout(unbound_toolkit::ui::Size::new(size.0 as f32, size.1 as f32));
    let second = Point::new(32.0, 8.0 + 52.0 + 26.0);

    ui.handle(&Event::PointerDown(Pointer::at(second)));
    ui.handle(&Event::PointerUp(Pointer::at(second)));
    ui.handle(&Event::PointerDown(Pointer::at(Point::new(32.0, 30.0))));
    ui.handle(&Event::PointerMove(Pointer::at(Point::new(200.0, 30.0))));
    ui.handle(&Event::PointerUp(Pointer::at(Point::new(200.0, 30.0))));

    assert_eq!(
        *actions.borrow(),
        [DockAction::Open("org.example.Editor".to_owned())]
    );
}

#[test]
fn every_app_is_a_labelled_button_for_assistive_technology() {
    let size = dock_size(DockEdge::Left, OUTPUT);
    let mut ui = dock_ui(
        DockEdge::Left,
        apps(),
        "09:41",
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        Rc::new(RefCell::new(Vec::new())),
    );
    ui.layout(unbound_toolkit::ui::Size::new(size.0 as f32, size.1 as f32));

    let tree = ui.accessibility();
    let buttons = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::Button)
        .map(|(_, node)| node.label().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(buttons, ["Terminal", "Editor"]);
    assert!(
        tree.nodes
            .iter()
            .any(|(_, node)| node.role() == Role::Label && node.value() == Some("09:41"))
    );
}
