#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::PathBuf;

use bande_audio::{Device, Direction, Level, Snapshot};
use bande_bluetooth::Bluetooth;
use bande_keyboard::Layouts;
use bande_settings::sidebar_width;
use bande_settings::{
    About, App, AppNotifications, AvailableLocale, BatteryState, BluetoothState, Category, Charge,
    DefaultApp, Display, Grant, LocaleState, LoginItems, Mode, Package, Pair, PaneId,
    PointingDevices, SettingId, SettingsView, SoftwareState, Storage, TimeState, Update, Value,
    WifiState, thumbnail,
};
use bande_settings_panes::Permission;
use bande_test_bus::SessionBus;
use bande_test_bus::bluez::FakeBluez;
use bande_toolkit::ui::{Event, Key, Modifiers};
use bande_toolkit::{
    Appearance, CpuRenderer, FrameRegions, Palette, Scene, Scheme, TextSize, TextSystem,
    WINDOW_HEADER_HEIGHT, framed_scene,
};
use support::SIZE;

type Build = fn(Scheme) -> SettingsView;

fn framed(view: &mut SettingsView, scheme: Scheme, text: TextSize, size: (u32, u32)) -> Scene {
    let content = view.scene(size.0, size.1 - WINDOW_HEADER_HEIGHT);
    framed_scene(
        &content,
        "Settings",
        Some(FrameRegions {
            sidebar: sidebar_width(text),
            toolbar: 0.0,
        }),
        None,
        &Palette::for_scheme(scheme),
    )
}

fn new_view(scheme: Scheme) -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(scheme), TextSystem::new());
    view.set_icon_dirs(vec![
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data"),
    ]);
    view
}

fn sleep_search(scheme: Scheme) -> SettingsView {
    let mut view = new_view(scheme);
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
    let mut view = new_view(scheme);
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
        capturing: Vec::new(),
        default_output: Some("speakers".to_owned()),
        default_input: Some("microphone".to_owned()),
    })));
    open(&mut view, "sound");
    view
}

fn displays(scheme: Scheme) -> SettingsView {
    let mut view = new_view(scheme);
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

    view.apply(Update::Values(vec![
        (SettingId::Brightness, Value::Fraction(0.6)),
        (SettingId::NightLight, Value::Flag(false)),
    ]));
    open(&mut view, "displays");
    view
}

fn night_light(scheme: Scheme) -> SettingsView {
    let mut view = displays(scheme);
    view.set_clock(
        || "2026-10-31T23:10[Europe/Stockholm]".parse().expect("zoned"),
        None,
    );
    view.apply(Update::Values(vec![
        (SettingId::NightLight, Value::Flag(true)),
        (
            SettingId::NightSchedule,
            Value::Keyword("custom".to_owned()),
        ),
        (SettingId::NightFrom, Value::Text("22:00".to_owned())),
        (SettingId::NightTo, Value::Text("07:00".to_owned())),
        (SettingId::NightWarmth, Value::Number(60)),
    ]));
    view.select_pane(PaneId::NightLight);
    view
}

fn grant(app: &str, name: &str, allowed: bool) -> Grant {
    Grant {
        app: app.to_owned(),
        name: name.to_owned(),
        allowed,
        tokens: vec![format!("{app}-token")],
    }
}

fn granted(scheme: Scheme) -> SettingsView {
    let mut view = new_view(scheme);
    view.set_value(SettingId::LockAfter, Value::Seconds(600));
    view.apply(Update::Permissions(vec![
        (
            Permission::Camera,
            vec![
                grant("org.mozilla.firefox", "Firefox", true),
                grant("com.example.Chat", "Chat", false),
                grant("", "Other apps", true),
            ],
        ),
        (
            Permission::ScreenSharing,
            vec![grant("org.mozilla.firefox", "Firefox", true)],
        ),
    ]));
    view
}

fn camera(scheme: Scheme) -> SettingsView {
    let mut view = granted(scheme);
    view.select_pane(PaneId::Camera);
    view
}

fn screen_sharing(scheme: Scheme) -> SettingsView {
    let mut view = granted(scheme);
    view.select_pane(PaneId::ScreenSharing);
    view
}

fn large_text(scheme: Scheme) -> SettingsView {
    let mut view = accessibility(scheme);
    view.set_appearance(Appearance {
        text_size: TextSize::from_percent(LARGE_TEXT),
        ..Appearance::default()
    });
    view
}

const LARGE_TEXT: u32 = 150;

fn larger_power(scheme: Scheme) -> SettingsView {
    let mut view = power(scheme);
    view.set_appearance(Appearance {
        text_size: TextSize::from_percent(LARGE_TEXT),
        ..Appearance::default()
    });
    view
}

fn accessibility(scheme: Scheme) -> SettingsView {
    let mut view = new_view(scheme);
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
    view.set_value(SettingId::TextSize, Value::Number(LARGE_TEXT));
    view.select_pane(PaneId::Accessibility);
    view
}

fn appearance(scheme: Scheme) -> SettingsView {
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
    let mut view = new_view(scheme);
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
    view.apply(Update::Values(vec![
        (SettingId::WallpaperFit, Value::Keyword("fill".to_owned())),
        (
            SettingId::Appearance,
            Value::Keyword("automatic".to_owned()),
        ),
        (SettingId::Accent, Value::Keyword("teal".to_owned())),
        (SettingId::TintedFrame, Value::Flag(false)),
        (SettingId::TextSize, Value::Number(100)),
    ]));
    view.select_pane(PaneId::Appearance);
    view
}

fn general(scheme: Scheme, pane: PaneId) -> SettingsView {
    let mut view = new_view(scheme);
    view.apply(Update::About(About {
        name: "Studio".to_owned(),
        model: "Apple MacBook Pro (14-inch, M1 Pro, 2021)".to_owned(),
        chip: "Apple M1 Pro".to_owned(),
        memory: 16_000_000_000,
        capacity: 494_000_000_000,
        available: 210_000_000_000,
        bande: "0.1.0".to_owned(),
        system: "Debian 13.7 (trixie)".to_owned(),
        serial: Some("C02XL0GHJGH5".to_owned()),
    }));
    view.select_pane(pane);
    view
}

fn about(scheme: Scheme) -> SettingsView {
    let mut view = general(scheme, PaneId::About);
    view.apply(Update::Software(SoftwareState::UpToDate));
    view
}

fn about_inactive(scheme: Scheme) -> SettingsView {
    let mut view = about(scheme);
    view.set_window_active(false);
    view
}

fn wifi_off(scheme: Scheme) -> SettingsView {
    let mut view = new_view(scheme);
    view.apply(Update::Wifi(Some(WifiState {
        enabled: false,
        networks: Vec::new(),
    })));
    view.select_pane(PaneId::Wifi);
    view
}

fn bluetooth(scheme: Scheme) -> SettingsView {
    let bus = SessionBus::start().expect("dbus-daemon");
    let _fake = FakeBluez::start(&bus).expect("fake BlueZ");
    let client = Bluetooth::new(&bus.connect().expect("client"));
    let mut view = new_view(scheme);
    view.apply(Update::Bluetooth(Some(BluetoothState {
        powered: true,
        paired: client.paired_devices().expect("paired"),
        nearby: client.nearby_devices().expect("nearby"),
    })));
    view.select_pane(PaneId::Bluetooth);
    view
}

fn bar(scheme: Scheme) -> SettingsView {
    let mut view = new_view(scheme);
    view.apply(Update::Values(vec![
        (SettingId::BarEdge, Value::Keyword("left".to_owned())),
        (SettingId::BarIconSize, Value::Number(44)),
        (SettingId::BarAutoHide, Value::Flag(false)),
        (SettingId::BarMenuBar, Value::Keyword("in-bar".to_owned())),
    ]));
    view.select_pane(PaneId::Bar);
    view
}

fn keyboard(scheme: Scheme, pane: PaneId) -> SettingsView {
    let mut view = new_view(scheme);
    view.set_layouts(Layouts::parse(
        r#"<xkbConfigRegistry><layoutList>
<layout><configItem><name>se</name><shortDescription>sv</shortDescription><description>Swedish</description></configItem></layout>
<layout><configItem><name>us</name><shortDescription>en</shortDescription><description>English (US)</description></configItem></layout>
</layoutList></xkbConfigRegistry>"#,
    ));
    view.apply(Update::Values(vec![
        (
            SettingId::InputSources,
            Value::Codes(vec!["us".to_owned(), "se".to_owned()]),
        ),
        (SettingId::KeyRepeatRate, Value::Number(30)),
        (SettingId::KeyRepeatDelay, Value::Number(450)),
        (SettingId::CapsLock, Value::Keyword("caps-lock".to_owned())),
    ]));
    view.select_pane(pane);
    view
}

fn keyboard_pane(scheme: Scheme) -> SettingsView {
    keyboard(scheme, PaneId::Keyboard)
}

fn keyboard_shortcuts(scheme: Scheme) -> SettingsView {
    keyboard(scheme, PaneId::KeyboardShortcuts)
}

fn pointing(scheme: Scheme, pane: PaneId) -> SettingsView {
    let mut view = new_view(scheme);
    view.set_pointing_devices(PointingDevices {
        trackpad: true,
        mouse: true,
    });
    view.apply(Update::Values(vec![
        (SettingId::TrackpadTapToClick, Value::Flag(true)),
        (
            SettingId::TrackpadSecondaryClick,
            Value::Keyword("two-fingers".to_owned()),
        ),
        (SettingId::TrackpadNaturalScrolling, Value::Flag(true)),
        (SettingId::TrackpadSpeed, Value::Number(5)),
        (SettingId::TrackpadDisableWhileTyping, Value::Flag(true)),
        (SettingId::TrackpadWorkspaceSwipe, Value::Flag(true)),
        (SettingId::TrackpadOverviewSwipe, Value::Flag(true)),
        (SettingId::TrackpadDesktopSpread, Value::Flag(true)),
        (SettingId::TrackpadLauncherPinch, Value::Flag(true)),
        (SettingId::MouseNaturalScrolling, Value::Flag(false)),
        (SettingId::MouseSpeed, Value::Number(5)),
        (
            SettingId::MousePrimaryButton,
            Value::Keyword("left".to_owned()),
        ),
        (SettingId::MouseAcceleration, Value::Flag(true)),
        (SettingId::TrackpadAcceleration, Value::Flag(true)),
        (
            SettingId::TrackpadDragging,
            Value::Keyword("tap".to_owned()),
        ),
        (
            SettingId::TrackpadScrollMethod,
            Value::Keyword("two-fingers".to_owned()),
        ),
    ]));
    view.select_pane(pane);
    view
}

fn mouse(scheme: Scheme) -> SettingsView {
    pointing(scheme, PaneId::Mouse)
}

fn trackpad(scheme: Scheme) -> SettingsView {
    pointing(scheme, PaneId::Trackpad)
}

fn privacy(scheme: Scheme) -> SettingsView {
    let mut view = granted(scheme);
    view.select_pane(PaneId::PrivacySecurity);
    view
}

fn apps(scheme: Scheme) -> SettingsView {
    let mut view = new_view(scheme);
    let app = |id: &str, name: &str| App {
        id: id.to_owned(),
        name: name.to_owned(),
    };
    view.apply(Update::Login(LoginItems {
        items: vec![app("org.bande.Terminal", "Terminal")],
        apps: vec![app("org.bande.Terminal", "Terminal")],
    }));
    let role = |role: &'static str, apps: Vec<App>| DefaultApp {
        role,
        current: apps.first().map(|app| app.id.clone()),
        apps,
    };
    view.apply(Update::DefaultApps(vec![
        role(
            "web-browser",
            vec![app("firefox-esr.desktop", "Firefox ESR")],
        ),
        role("mail", Vec::new()),
        role("music", Vec::new()),
        role("video", vec![app("mpv.desktop", "mpv Media Player")]),
        role("pictures", vec![app("org.bande.Files.desktop", "Files")]),
        role(
            "pdf",
            vec![app("org.gnome.Evince.desktop", "Document Viewer")],
        ),
        role("text", vec![app("org.bande.Editor.desktop", "Text Editor")]),
    ]));
    view.select_pane(PaneId::Apps);
    view
}

fn language_time(scheme: Scheme) -> SettingsView {
    let mut view = new_view(scheme);
    view.apply(Update::Locale(Some(LocaleState {
        available: vec![
            AvailableLocale {
                code: "en_US.UTF-8".to_owned(),
                name: "English (US)".to_owned(),
            },
            AvailableLocale {
                code: "sv_SE.UTF-8".to_owned(),
                name: "Swedish (Sweden)".to_owned(),
            },
        ],
        language: "en_US.UTF-8".to_owned(),
        region: Some("sv_SE.UTF-8".to_owned()),
        pending: false,
        login_sources: Vec::new(),
    })));
    view.apply(Update::Time(Some(TimeState {
        automatic: true,
        can_automatic: true,
        zone: "Atlantic/Reykjavik".to_owned(),
        zones: vec!["Atlantic/Reykjavik".to_owned()],
    })));
    view.set_value(SettingId::Clock24Hour, Value::Flag(true));
    view.select_pane(PaneId::LanguageTime);
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

fn power(scheme: Scheme) -> SettingsView {
    let mut view = new_view(scheme);
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
    view.select_pane(PaneId::Power);
    view
}

fn notifications(scheme: Scheme) -> SettingsView {
    let mut view = new_view(scheme);
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
    let mut view = new_view(scheme);
    view.apply(Update::Network(bande_settings::NetworkState {
        ethernet: Some(bande_network::Ethernet {
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
            bande_network::Vpn {
                path: "/a".to_owned(),
                name: "Home WireGuard".to_owned(),
                state: bande_network::VpnState::Connected,
            },
            bande_network::Vpn {
                path: "/b".to_owned(),
                name: "Office".to_owned(),
                state: bande_network::VpnState::Off,
            },
        ],
    }));
    view.select_pane(PaneId::Network);
    view
}

#[test]
fn the_panes_match_their_goldens() {
    let panes: [(&str, Scheme, Build, f64); 31] = [
        ("appearance-light", Scheme::Light, appearance, 1.0),
        ("appearance-dark", Scheme::Dark, appearance, 1.0),
        ("appearance-light@1.5x", Scheme::Light, appearance, 1.5),
        ("search-sleep-light", Scheme::Light, sleep_search, 1.0),
        ("wifi-off-light", Scheme::Light, wifi_off, 1.0),
        ("bluetooth-light", Scheme::Light, bluetooth, 1.0),
        ("network-light", Scheme::Light, network, 1.0),
        ("bar-light", Scheme::Light, bar, 1.0),
        ("notifications-light", Scheme::Light, notifications, 1.0),
        ("accessibility-light", Scheme::Light, accessibility, 1.0),
        ("displays-light", Scheme::Light, displays, 1.0),
        ("displays-dark", Scheme::Dark, displays, 1.0),
        ("night-light-light", Scheme::Light, night_light, 1.0),
        ("camera-light", Scheme::Light, camera, 1.0),
        ("screen-sharing-light", Scheme::Light, screen_sharing, 1.0),
        ("accessibility-150-light", Scheme::Light, large_text, 1.0),
        ("sound-light", Scheme::Light, sound, 1.0),
        ("keyboard-light", Scheme::Light, keyboard_pane, 1.0),
        (
            "keyboard-shortcuts-light",
            Scheme::Light,
            keyboard_shortcuts,
            1.0,
        ),
        ("mouse-light", Scheme::Light, mouse, 1.0),
        ("trackpad-light", Scheme::Light, trackpad, 1.0),
        ("power-light", Scheme::Light, power, 1.0),
        ("power-150-light", Scheme::Light, larger_power, 1.0),
        ("privacy-security-light", Scheme::Light, privacy, 1.0),
        ("apps-light", Scheme::Light, apps, 1.0),
        ("language-time-light", Scheme::Light, language_time, 1.0),
        ("about-light", Scheme::Light, about, 1.0),
        ("about-dark", Scheme::Dark, about, 1.0),
        ("about-inactive-light", Scheme::Light, about_inactive, 1.0),
        ("storage-light", Scheme::Light, storage, 1.0),
        ("software-update-light", Scheme::Light, software_update, 1.0),
    ];
    std::thread::scope(|scope| {
        for (name, scheme, build, scale) in panes {
            scope.spawn(move || {
                let mut view = build(scheme);
                let text = match name.contains("-150") {
                    true => TextSize::from_percent(LARGE_TEXT),
                    false => TextSize::DEFAULT,
                };
                let scene = framed(&mut view, scheme, text, SIZE);
                let pixels = (
                    (f64::from(SIZE.0) * scale).round() as u32,
                    (f64::from(SIZE.1) * scale).round() as u32,
                );
                let mut rgba = vec![0; pixels.0 as usize * pixels.1 as usize * 4];
                CpuRenderer::new()
                    .render_into(&scene, scale, &mut rgba)
                    .expect("frame");
                bande_golden::assert_golden_rgba!(name, pixels, rgba);
            });
        }
    });
}

#[test]
fn long_row_labels_wrap_beside_their_control_at_large_text() {
    const NARROW: (u32, u32) = (660, 720);
    let mut view = large_text(Scheme::Light);
    let scene = framed(
        &mut view,
        Scheme::Light,
        TextSize::from_percent(LARGE_TEXT),
        NARROW,
    );
    let nodes = view.accessibility().nodes;
    let row_height = |label: &str| {
        nodes
            .iter()
            .find(|(_, node)| node.role() == accesskit::Role::Row && node.label() == Some(label))
            .and_then(|(_, node)| node.bounds())
            .map(|bounds| bounds.height())
            .unwrap_or_else(|| panic!("no row {label:?}"))
    };
    assert!(row_height("Flash the screen when an alert sound plays") > row_height("Zoom"));
    let mut rgba = vec![0; NARROW.0 as usize * NARROW.1 as usize * 4];
    CpuRenderer::new()
        .render_into(&scene, 1.0, &mut rgba)
        .expect("frame");
    bande_golden::assert_golden_rgba!("accessibility-150-narrow-light", NARROW, rgba);
}
