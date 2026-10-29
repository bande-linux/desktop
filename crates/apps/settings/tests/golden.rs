#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use unbound_settings::{SettingId, SettingsView, Value};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{CpuRenderer, Palette, Scheme, TextSystem};

const SIZE: (u32, u32) = (900, 600);
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

fn lock_screen_search(scheme: Scheme) -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(scheme), TextSystem::new());
    view.set_value(SettingId::DimAfter, Value::Seconds(300));
    view.set_value(SettingId::LockAfter, Value::Seconds(600));
    view.set_value(
        SettingId::SleepAfter,
        Value::Keyword("with-display".to_owned()),
    );
    for character in "sleep".chars() {
        view.handle(&Event::Key {
            key: Key::Character(character),
            modifiers: Modifiers::default(),
        });
    }
    view
}

#[test]
fn a_search_matches_its_golden_in_light_and_dark() {
    for (scheme, name) in [
        (Scheme::Light, "search-sleep-light"),
        (Scheme::Dark, "search-sleep-dark"),
    ] {
        let mut view = lock_screen_search(scheme);
        let scene = view.scene(SIZE.0, SIZE.1);
        let frame = CpuRenderer::new().render(&scene).expect("frame");
        check_golden(name, frame.rgba);
    }
}

#[test]
fn the_keyboard_pane_matches_its_golden() {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.set_value(SettingId::KeyRepeatRate, Value::Number(25));
    view.set_value(SettingId::KeyRepeatDelay, Value::Number(600));
    for key in [Key::Down, Key::Down, Key::Down] {
        view.handle(&Event::Key {
            key: Key::Tab,
            modifiers: Modifiers::default(),
        });
        view.handle(&Event::Key {
            key,
            modifiers: Modifiers::default(),
        });
    }
    let scene = view.scene(SIZE.0, SIZE.1);
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    check_golden("keyboard-light", frame.rgba);
}
