#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::PathBuf;

use support::SIZE;
use unbound_audio::{Device, Direction, Level, Snapshot};
use unbound_settings::SIDEBAR_WIDTH;
use unbound_settings::{
    About, AppNotifications, BatteryState, Category, Charge, Display, Mode, Package, Pair, PaneId,
    SettingId, SettingsView, SoftwareState, Storage, Update, Value, thumbnail,
};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{
    CpuRenderer, FrameRegions, HEADER_HEIGHT, Palette, Scene, Scheme, TextSystem, framed_scene,
};

type Build = fn(Scheme) -> SettingsView;

fn framed(view: &mut SettingsView, scheme: Scheme) -> Scene {
    let content = view.scene(SIZE.0, SIZE.1 - HEADER_HEIGHT);
    framed_scene(
        &content,
        "Settings",
        Some(FrameRegions {
            sidebar: SIDEBAR_WIDTH,
            toolbar: 0.0,
        }),
        None,
        &Palette::for_scheme(scheme),
    )
}

fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    unbound_golden::assert_golden!(name, &unbound_golden::from_rgba(size.0, size.1, rgba));
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

fn sound(scheme: Scheme) -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(scheme), TextSystem::new());
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
    view
}

fn displays(scheme: Scheme) -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(scheme), TextSystem::new());
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
    view
}

fn accessibility(scheme: Scheme) -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(scheme), TextSystem::new());
    for (id, on) in [
        (SettingId::ScreenReader, false),
        (SettingId::Zoom, true),
        (SettingId::IncreaseContrast, true),
        (SettingId::ReduceTransparency, false),
        (SettingId::ReduceMotion, false),
        (SettingId::FlashScreen, false),
        (SettingId::StickyKeys, true),
        (SettingId::SlowKeys, false),
        (SettingId::BounceKeys, false),
    ] {
        view.set_value(id, Value::Flag(on));
    }
    view.set_value(SettingId::CursorSize, Value::Number(32));
    view.select_pane(PaneId::Accessibility);
    view
}

fn wallpaper(scheme: Scheme) -> SettingsView {
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
    let mut view = SettingsView::new(Palette::for_scheme(scheme), TextSystem::new());
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
    view
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

fn appearance(scheme: Scheme) -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(scheme), TextSystem::new());
    view.apply(Update::Values(vec![
        (
            SettingId::Appearance,
            Value::Keyword("automatic".to_owned()),
        ),
        (SettingId::Accent, Value::Keyword("teal".to_owned())),
        (SettingId::TintedFrame, Value::Flag(false)),
        (SettingId::ReduceMotion, Value::Flag(false)),
    ]));
    view.select_pane(PaneId::Appearance);
    view
}

fn storage(scheme: Scheme) -> SettingsView {
    let home = std::env::home_dir().expect("home");
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
}

fn battery(scheme: Scheme) -> SettingsView {
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
}

fn notifications(scheme: Scheme) -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(scheme), TextSystem::new());
    view.set_value(SettingId::DoNotDisturb, Value::Flag(false));
    let app =
        |group: &str, name: &str, allow: bool, banners: &str, in_center: bool| AppNotifications {
            group: group.to_owned(),
            name: name.to_owned(),
            allow,
            banners: banners.to_owned(),
            in_center,
        };
    view.apply(Update::NotificationApps(vec![
        app("org.example.Chat", "Chat", true, "persistent", true),
        app("org.mozilla.firefox", "Firefox", false, "temporary", true),
        app("Mail", "Mail", true, "temporary", false),
    ]));
    view.select_pane(PaneId::Notifications);
    view.scene(SIZE.0, SIZE.1);
    let chat = view
        .accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| node.label().is_some_and(|label| label.starts_with("Chat,")))
        .map(|(id, _)| id)
        .expect("Chat row");
    view.perform(&accesskit::ActionRequest {
        action: accesskit::Action::Click,
        target_tree: accesskit::TreeId::ROOT,
        target_node: chat,
        data: None,
    });
    view
}

fn software_update(scheme: Scheme) -> SettingsView {
    let mut view = general(scheme, PaneId::SoftwareUpdate);
    let package = |name: &str, version: &str, size: u64| Package {
        id: format!("{name};{version};arm64;debian"),
        name: name.to_owned(),
        version: version.to_owned(),
        summary: String::new(),
        size,
    };
    view.apply(Update::Software(SoftwareState::Available(vec![
        package("firefox-esr", "128.4.0esr-1~deb13u1", 70_400_000),
        package("libc6", "2.41-12", 3_100_000),
        package("linux-image-asahi", "6.12.9-1", 121_000_000),
    ])));
    view
}

fn network(scheme: Scheme) -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(scheme), TextSystem::new());
    view.apply(Update::Network(unbound_settings::NetworkState {
        ethernet: Some(unbound_network::Ethernet {
            interface: "end0".to_owned(),
            cable: true,
            managed: true,
            connected: true,
            address: Some("192.168.1.20/24".to_owned()),
            router: Some("192.168.1.1".to_owned()),
            dns: vec!["1.1.1.1".to_owned(), "9.9.9.9".to_owned()],
            manual: true,
        }),
        vpns: vec![
            unbound_network::Vpn {
                path: "/a".to_owned(),
                name: "Home WireGuard".to_owned(),
                state: unbound_network::VpnState::Connected,
            },
            unbound_network::Vpn {
                path: "/b".to_owned(),
                name: "Office".to_owned(),
                state: unbound_network::VpnState::Off,
            },
        ],
    }));
    view.select_pane(PaneId::Network);
    view
}

#[test]
fn the_panes_match_their_goldens() {
    let panes: [(&str, Scheme, Build, f64); 13] = [
        ("appearance-light", Scheme::Light, appearance, 1.0),
        ("appearance-dark", Scheme::Dark, appearance, 1.0),
        ("appearance-light@1.5x", Scheme::Light, appearance, 1.5),
        ("search-sleep-light", Scheme::Light, lock_screen_search, 1.0),
        ("accessibility-light", Scheme::Light, accessibility, 1.0),
        ("wallpaper-light", Scheme::Light, wallpaper, 1.0),
        ("sound-light", Scheme::Light, sound, 1.0),
        ("displays-dark", Scheme::Dark, displays, 1.0),
        ("network-light", Scheme::Light, network, 1.0),
        ("notifications-light", Scheme::Light, notifications, 1.0),
        ("battery-light", Scheme::Light, battery, 1.0),
        ("storage-light", Scheme::Light, storage, 1.0),
        ("software-update-light", Scheme::Light, software_update, 1.0),
    ];
    std::thread::scope(|scope| {
        for (name, scheme, build, scale) in panes {
            scope.spawn(move || {
                let mut view = build(scheme);
                let scene = framed(&mut view, scheme);
                let pixels = (
                    (f64::from(SIZE.0) * scale).round() as u32,
                    (f64::from(SIZE.1) * scale).round() as u32,
                );
                let mut rgba = vec![0; pixels.0 as usize * pixels.1 as usize * 4];
                CpuRenderer::new()
                    .render_into(&scene, scale, &mut rgba)
                    .expect("frame");
                check_golden(name, pixels, rgba);
            });
        }
    });
}
