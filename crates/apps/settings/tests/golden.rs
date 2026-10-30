#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use unbound_audio::{Device, Direction, Level, Snapshot};
use unbound_keyboard::Layouts;
use unbound_settings::{
    Display, Mode, Pair, PointingDevices, SettingId, SettingsView, Update, Value, thumbnail,
};
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

fn open(view: &mut SettingsView, query: &str) {
    for character in query.chars() {
        view.handle(&Event::Key {
            key: Key::Character(character),
            modifiers: Modifiers::default(),
        });
    }
    view.handle(&Event::Key {
        key: Key::Escape,
        modifiers: Modifiers::default(),
    });
}

fn device(id: u32, name: &str, description: &str, direction: Direction, volume: f32) -> Device {
    Device {
        id,
        name: name.to_owned(),
        description: description.to_owned(),
        direction,
        level: Level {
            volume,
            muted: false,
        },
    }
}

#[test]
fn the_sound_pane_matches_its_golden() {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    for (id, value) in [
        (SettingId::AlertSound, Value::Keyword("mallet".to_owned())),
        (SettingId::AlertVolume, Value::Fraction(0.75)),
        (SettingId::SoundEffects, Value::Flag(true)),
        (SettingId::VolumeFeedback, Value::Flag(true)),
    ] {
        view.set_value(id, value);
    }
    view.apply(Update::Audio(Some(Snapshot {
        generation: 1,
        connected: true,
        outputs: vec![
            device(
                1,
                "speakers",
                "MacBook Pro Speakers",
                Direction::Output,
                0.6,
            ),
            device(2, "headset", "USB Headset", Direction::Output, 0.3),
        ],
        inputs: vec![device(
            3,
            "microphone",
            "MacBook Pro Microphone",
            Direction::Input,
            0.8,
        )],
        apps: Vec::new(),
        default_output: Some("speakers".to_owned()),
        default_input: Some("microphone".to_owned()),
    })));
    open(&mut view, "sound");
    let scene = view.scene(SIZE.0, SIZE.1);
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    check_golden("sound-light", frame.rgba);
}

#[test]
fn the_displays_pane_matches_its_golden_in_dark() {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Dark), TextSystem::new());
    let modes: Vec<Mode> = [120, 60]
        .into_iter()
        .map(|hertz| Mode {
            width: 3024,
            height: 1964,
            millihertz: hertz * 1000,
        })
        .collect();
    let external = [(3840, 2160), (2560, 1440), (1920, 1080)]
        .map(|(width, height)| Mode {
            width,
            height,
            millihertz: 60_000,
        })
        .to_vec();
    view.set_displays(vec![
        Display {
            connector: "eDP-1".to_owned(),
            identity: "Built-in Retina Display".to_owned(),
            model: "Built-in Retina Display".to_owned(),
            current: modes.first().copied(),
            modes,
            scale: 2.0,
            ..Display::default()
        },
        Display {
            connector: "DP-1".to_owned(),
            identity: "Dell DELL U2720Q 7ZD5K53".to_owned(),
            make: "Dell".to_owned(),
            model: "DELL U2720Q".to_owned(),
            current: external.first().copied(),
            modes: external,
            position: (1512, -200),
            scale: 1.5,
            ..Display::default()
        },
    ]);

    open(&mut view, "displays");
    let scene = view.scene(SIZE.0, SIZE.1);
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    check_golden("displays-dark", frame.rgba);
}

#[test]
fn the_keyboard_pane_matches_its_golden() {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.set_value(SettingId::KeyRepeatRate, Value::Number(25));
    view.set_value(SettingId::KeyRepeatDelay, Value::Number(600));
    view.set_layouts(Layouts::parse(
        r#"<xkbConfigRegistry><layoutList>
<layout><configItem><name>se</name><shortDescription>sv</shortDescription><description>Swedish</description></configItem></layout>
<layout><configItem><name>us</name><shortDescription>en</shortDescription><description>English (US)</description></configItem></layout>
</layoutList></xkbConfigRegistry>"#,
    ));
    view.set_value(
        SettingId::InputSources,
        Value::Codes(vec!["se".to_owned(), "us".to_owned()]),
    );
    view.set_value(SettingId::CapsLock, Value::Keyword("caps-lock".to_owned()));
    for key in [Key::Down, Key::Down, Key::Down, Key::Down] {
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

#[test]
fn the_wallpaper_pane_matches_its_golden() {
    let wallpapers = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data/wallpapers");
    let pairs: Vec<(String, Pair)> = ["aurora", "drift", "horizon"]
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                Pair {
                    light: wallpapers.join(format!("{name}-light.svg")),
                    dark: wallpapers.join(format!("{name}-dark.svg")),
                },
            )
        })
        .collect();
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.apply(Update::Pictures(pairs.clone()));
    for (_, pair) in &pairs {
        let picture = thumbnail(pair).expect("thumbnail");
        view.apply(Update::Thumbnail(pair.clone(), picture));
    }
    view.set_value(
        SettingId::Wallpaper,
        Value::Pictures {
            light: String::new(),
            dark: String::new(),
        },
    );
    view.set_value(SettingId::WallpaperFit, Value::Keyword("fill".to_owned()));
    open(&mut view, "wallpaper");
    let scene = view.scene(SIZE.0, SIZE.1);
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    check_golden("wallpaper-light", frame.rgba);
}

#[test]
fn the_trackpad_and_mouse_pane_matches_its_golden() {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Dark), TextSystem::new());
    view.set_pointing_devices(PointingDevices {
        trackpad: true,
        mouse: true,
    });
    view.apply(Update::Values(vec![
        (SettingId::TrackpadTapToClick, Value::Flag(true)),
        (SettingId::TrackpadNaturalScrolling, Value::Flag(true)),
        (SettingId::TrackpadSpeed, Value::Number(5)),
        (
            SettingId::TrackpadSecondaryClick,
            Value::Keyword("two-fingers".to_owned()),
        ),
        (SettingId::TrackpadDisableWhileTyping, Value::Flag(true)),
        (SettingId::MouseNaturalScrolling, Value::Flag(false)),
        (SettingId::MouseSpeed, Value::Number(5)),
    ]));
    open(&mut view, "trackpad");
    let scene = view.scene(SIZE.0, SIZE.1);
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    check_golden("trackpad-mouse-dark", frame.rgba);
}
