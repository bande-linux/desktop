#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use unbound_audio::{Device, Direction, Level, Snapshot};
use unbound_keyboard::Layouts;
use unbound_settings::{
    About, App, BatteryState, Category, Charge, Display, LoginItems, Mode, Pair, PaneId,
    PointingDevices, SettingId, SettingsView, Storage, TimeState, Update, Value, thumbnail,
};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{CpuRenderer, Palette, Scheme, TextSystem};

const SIZE: (u32, u32) = (900, 600);
const MIN_SSIM: f64 = 0.99;

fn check_golden(name: &str, rgba: Vec<u8>) {
    check_golden_sized(name, SIZE, rgba);
}

fn check_golden_sized(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    let actual = image::RgbaImage::from_raw(size.0, size.1, rgba).expect("frame");
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

fn check_in_both_schemes_and_scales(name: &str, build: impl Fn(Scheme) -> SettingsView) {
    for (scheme, suffix) in [(Scheme::Light, "light"), (Scheme::Dark, "dark")] {
        let mut view = build(scheme);
        let scene = view.scene(SIZE.0, SIZE.1);
        for (scale, density) in [(1.0, ""), (1.5, "@1.5x")] {
            let pixels = (
                (f64::from(SIZE.0) * scale).round() as u32,
                (f64::from(SIZE.1) * scale).round() as u32,
            );
            let mut rgba = vec![0; pixels.0 as usize * pixels.1 as usize * 4];
            CpuRenderer::new()
                .render_into(&scene, scale, &mut rgba)
                .expect("frame");
            check_golden_sized(&format!("{name}-{suffix}{density}"), pixels, rgba);
        }
    }
}

fn general(scheme: Scheme, pane: PaneId) -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(scheme), TextSystem::new());
    view.apply(Update::About(About {
        name: "Studio".to_owned(),
        model: "Apple MacBook Pro (14-inch, M1 Pro, 2021)".to_owned(),
        chip: "Apple M1 Pro".to_owned(),
        memory: 16_000_000_000,
        capacity: 494_000_000_000,
        available: 210_000_000_000,
        unbound: "0.1.0".to_owned(),
        system: "Debian 13.7 (trixie)".to_owned(),
        serial: Some("C02XL0GHJGH5".to_owned()),
    }));
    view.select_pane(pane);
    view
}

#[test]
fn the_about_pane_matches_its_goldens() {
    check_in_both_schemes_and_scales("about", |scheme| general(scheme, PaneId::About));
}

#[test]
fn the_storage_pane_matches_its_goldens() {
    let home = std::env::home_dir().expect("home");
    check_in_both_schemes_and_scales("storage", |scheme| {
        let mut view = general(scheme, PaneId::Storage);
        view.apply(Update::Storage(Storage {
            capacity: 494_000_000_000,
            available: 210_000_000_000,
            categories: Some(vec![
                (Category::Apps, 21_000_000_000),
                (Category::Documents, 182_000_000_000),
                (Category::Trash, 4_100_000_000),
                (Category::System, 76_900_000_000),
            ]),
            largest: vec![
                (home.join("Movies/Holiday.mov"), 9_800_000_000),
                (home.join("Downloads/debian-13.iso"), 3_900_000_000),
                (home.join("Music/Live at the Barbican.flac"), 1_200_000_000),
            ],
            trash_items: 12,
        }));
        view
    });
}

#[test]
fn the_general_pane_matches_its_goldens() {
    check_in_both_schemes_and_scales("general", |scheme| general(scheme, PaneId::General));
}

#[test]
fn the_date_and_time_pane_matches_its_goldens() {
    check_in_both_schemes_and_scales("date-time", |scheme| {
        let mut view = general(scheme, PaneId::DateTime);
        view.apply(Update::Time(Some(TimeState {
            automatic: true,
            can_automatic: true,
            zone: "Asia/Tokyo".to_owned(),
            zones: Vec::new(),
        })));
        view.set_value(SettingId::Clock24Hour, Value::Flag(true));
        view
    });
}

#[test]
fn the_login_items_pane_matches_its_goldens() {
    check_in_both_schemes_and_scales("login-items", |scheme| {
        let mut view = general(scheme, PaneId::LoginItems);
        let app = |id: &str, name: &str| App {
            id: id.to_owned(),
            name: name.to_owned(),
        };
        view.apply(Update::Login(LoginItems {
            items: vec![
                app("org.mozilla.firefox.desktop", "Firefox"),
                app("org.unbound.Terminal.desktop", "Terminal"),
            ],
            apps: Vec::new(),
        }));
        view
    });
}

#[test]
fn the_battery_pane_matches_its_goldens() {
    check_in_both_schemes_and_scales("battery", |scheme| {
        let mut view = SettingsView::new(Palette::for_scheme(scheme), TextSystem::new());
        let history = (0..=48u32)
            .map(|step| {
                let level = if step < 30 {
                    95.0 - f64::from(step) * 2.0
                } else {
                    35.0 + f64::from(step - 30) * 3.0
                };
                (1_000_000 + step * 1800, level)
            })
            .collect();
        view.apply(Update::Battery(Some(BatteryState {
            percent: 89.0,
            charge: Charge::Charging { minutes: Some(25) },
            history,
            profiles: ["power-saver", "balanced", "performance"]
                .map(str::to_owned)
                .to_vec(),
            profile: Some("balanced".to_owned()),
        })));
        view.set_value(SettingId::LowPowerOnBattery, Value::Flag(true));
        view.set_value(SettingId::LidOnPower, Value::Keyword("sleep".to_owned()));
        view.select_pane(PaneId::Battery);
        view
    });
}
