#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use unbound_launcher::{Launcher, LauncherAction, LauncherApp, search};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{CpuRenderer, Palette, Point, Scheme, TextSystem};

const SCREEN: (u32, u32) = (1024, 640);
const MIN_SSIM: f64 = 0.99;

fn app(app_id: &str, name: &str, generic: Option<&str>, keywords: &[&str]) -> LauncherApp {
    LauncherApp {
        app_id: app_id.to_owned(),
        name: name.to_owned(),
        generic_name: generic.map(str::to_owned),
        comment: None,
        keywords: keywords
            .iter()
            .map(|keyword| (*keyword).to_owned())
            .collect(),
        icon: None,
    }
}

fn apps() -> Vec<LauncherApp> {
    vec![
        app(
            "org.example.Calculator",
            "Calculator",
            Some("Calculator"),
            &["math"],
        ),
        app(
            "unbound-term",
            "Terminal",
            Some("Terminal Emulator"),
            &["shell", "prompt"],
        ),
        app(
            "org.example.TextEditor",
            "Text Editor",
            Some("Text Editor"),
            &["notepad"],
        ),
        app(
            "org.example.Settings",
            "System Settings",
            None,
            &["preferences"],
        ),
        app("org.example.Fonts", "Fonts", None, &["typeface"]),
    ]
}

fn names(query: &str) -> Vec<String> {
    let apps = apps();
    search(&apps, query)
        .into_iter()
        .map(|index| apps[index].name.clone())
        .collect()
}

fn typed(launcher: &mut Launcher, text: &str) {
    for character in text.chars() {
        launcher.handle(&Event::Key {
            key: Key::Character(character),
            modifiers: Modifiers::default(),
        });
    }
}

fn key(launcher: &mut Launcher, key: Key) {
    launcher.handle(&Event::Key {
        key,
        modifiers: Modifiers::default(),
    });
}

#[test]
fn names_that_start_with_the_query_come_before_words_and_keywords() {
    assert_eq!(names("te"), ["Terminal", "Text Editor"]);
    assert_eq!(names("ed"), ["Text Editor"]);
    assert_eq!(names("set"), ["System Settings"]);
    assert_eq!(names("shell"), ["Terminal"]);
    assert_eq!(names("preferences"), ["System Settings"]);
    assert_eq!(names("t"), ["Terminal", "Text Editor", "Fonts"]);
    assert!(names("  ").is_empty());
}

#[test]
fn arrows_choose_a_result_and_return_opens_it() {
    let mut launcher = Launcher::new(
        apps(),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    );
    typed(&mut launcher, "te");
    key(&mut launcher, Key::Down);
    key(&mut launcher, Key::Down);
    key(&mut launcher, Key::Up);
    key(&mut launcher, Key::Down);
    key(&mut launcher, Key::Enter);

    assert_eq!(
        launcher.take_actions(),
        [LauncherAction::Open("org.example.TextEditor".to_owned())]
    );
}

#[test]
fn escape_and_a_click_beside_the_card_close_the_launcher() {
    let mut launcher = Launcher::new(
        apps(),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    );
    launcher.scene(SCREEN.0, SCREEN.1);
    key(&mut launcher, Key::Escape);
    launcher.handle(&Event::PointerDown(Point::new(10.0, 10.0)));

    assert_eq!(
        launcher.take_actions(),
        [LauncherAction::Close, LauncherAction::Close]
    );
}

#[test]
fn clearing_forgets_the_query() {
    let mut launcher = Launcher::new(
        apps(),
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
    );
    typed(&mut launcher, "te");
    launcher.clear();
    key(&mut launcher, Key::Enter);

    assert!(launcher.take_actions().is_empty());
    assert!(launcher.model.borrow().results.is_empty());
}

fn check_golden(name: &str, rgba: Vec<u8>) {
    let actual = image::RgbaImage::from_raw(SCREEN.0, SCREEN.1, rgba).expect("frame");
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

#[test]
fn the_launcher_matches_its_goldens_empty_and_with_results() {
    for (scheme, suffix) in [(Scheme::Light, "light"), (Scheme::Dark, "dark")] {
        let mut launcher = Launcher::new(apps(), Palette::for_scheme(scheme), TextSystem::new());
        let mut scene = launcher.scene(SCREEN.0, SCREEN.1);
        scene.background = unbound_toolkit::Color::hex(0x5A6B7D);
        let frame = CpuRenderer::new().render(&scene).expect("frame");
        check_golden(&format!("launcher-empty-{suffix}"), frame.rgba);

        typed(&mut launcher, "te");
        key(&mut launcher, Key::Down);
        let mut scene = launcher.scene(SCREEN.0, SCREEN.1);
        scene.background = unbound_toolkit::Color::hex(0x5A6B7D);
        let frame = CpuRenderer::new().render(&scene).expect("frame");
        check_golden(&format!("launcher-results-{suffix}"), frame.rgba);
    }
}
