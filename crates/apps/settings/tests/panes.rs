#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId};
use unbound_audio::{AppStream, Device, Direction, Level, Snapshot};
use unbound_config::{DisplaySettings, Placement};
use unbound_keyboard::Layouts;
use unbound_settings::{
    Answer, AudioCommand, BluetoothCommand, Command, Display, Feed, Mode, NetworkCommand, Pair,
    PaneId, PictureCommand, PointingDevices, SettingId, SettingsView, Update, Value, bundled_pairs,
    feed, follow_bluetooth, follow_network,
};
use unbound_test_bus::SessionBus;
use unbound_test_bus::bluez::{FakeBluez, PHONE};
use unbound_test_bus::networkmanager::FakeNetworkManager;
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{Palette, Scheme, TextSystem};

const SIZE: (u32, u32) = (900, 600);
const WAIT: Duration = Duration::from_secs(5);

fn view() -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.scene(SIZE.0, SIZE.1);
    view
}

fn nodes(view: &mut SettingsView) -> Vec<(NodeId, Node)> {
    view.scene(SIZE.0, SIZE.1);
    view.accessibility().nodes
}

fn find(view: &mut SettingsView, role: Role, label: &str) -> Option<NodeId> {
    nodes(view)
        .into_iter()
        .find(|(_, node)| {
            node.role() == role && node.label().is_some_and(|text| text.starts_with(label))
        })
        .map(|(id, _)| id)
}

fn has_text(view: &mut SettingsView, text: &str) -> bool {
    nodes(view).into_iter().any(|(_, node)| {
        node.label().is_some_and(|label| label.contains(text))
            || node.value().is_some_and(|value| value.contains(text))
    })
}

fn button_in_row(view: &mut SettingsView, row: &str, button: &str) -> NodeId {
    let nodes = nodes(view);
    let node = |id: &NodeId| {
        nodes
            .iter()
            .find(|(candidate, _)| candidate == id)
            .map(|(_, node)| node)
    };
    let mut pending: Vec<NodeId> = nodes
        .iter()
        .filter(|(_, node)| node.label().is_some_and(|label| label.starts_with(row)))
        .flat_map(|(_, node)| node.children().to_vec())
        .collect();
    while let Some(id) = pending.pop() {
        let Some(found) = node(&id) else { continue };
        if found.role() == Role::Button && found.label() == Some(button) {
            return id;
        }
        pending.extend_from_slice(found.children());
    }
    panic!("no {button:?} button in the {row:?} row")
}

fn click(view: &mut SettingsView, target: NodeId) {
    view.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: target,
        data: None,
    });
}

fn press(view: &mut SettingsView, key: Key) {
    view.handle(&Event::Key {
        key,
        modifiers: Modifiers::default(),
    });
}

fn open(view: &mut SettingsView, pane: &str) {
    let row = find(view, Role::ListItem, pane).unwrap_or_else(|| panic!("no {pane} pane"));
    click(view, row);
}

struct Router {
    network: Option<Sender<NetworkCommand>>,
    bluetooth: Option<(Sender<BluetoothCommand>, Sender<Answer>)>,
}

impl Router {
    fn send(&self, commands: Vec<Command>) {
        for command in commands {
            match command {
                Command::Network(command) => {
                    self.network
                        .as_ref()
                        .expect("network")
                        .send(command)
                        .expect("network");
                }
                Command::Bluetooth(command) => {
                    let (bluetooth, _) = self.bluetooth.as_ref().expect("bluetooth");
                    bluetooth.send(command).expect("bluetooth");
                }
                Command::Answer(answer) => {
                    let (_, answers) = self.bluetooth.as_ref().expect("bluetooth");
                    answers.send(answer).expect("answer");
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }
}

fn pump(
    view: &mut SettingsView,
    feed: &Feed,
    router: &Router,
    done: impl Fn(&mut SettingsView) -> bool,
) {
    let deadline = Instant::now() + WAIT;
    loop {
        for update in feed.drain() {
            view.apply(update);
        }
        router.send(view.take_commands());
        if done(view) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the view never reached the expected state"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn wifi_lists_networks_joins_open_ones_and_asks_for_passwords() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let fake = FakeNetworkManager::start(&bus).expect("fake NetworkManager");
    let system = bus.connect().expect("system connection");
    let (feed, updates) = feed().expect("feed");
    let router = Router {
        network: Some(follow_network(&system, updates)),
        bluetooth: None,
    };
    let mut view = view();
    pump(&mut view, &feed, &router, |view| {
        view.visible_panes().contains(&PaneId::Wifi)
    });
    open(&mut view, "Wi-Fi");
    assert_eq!(view.selected_pane(), PaneId::Wifi);
    router.send(view.take_commands());
    assert!(has_text(&mut view, "Home, Connected"));
    assert!(has_text(&mut view, "Studio, Saved"));

    let cafe = find(&mut view, Role::Button, "Cafe").expect("Cafe row");
    click(&mut view, cafe);
    pump(&mut view, &feed, &router, |_| {
        fake.saved_ssids().contains(&b"Cafe".to_vec())
    });
    pump(&mut view, &feed, &router, |view| {
        has_text(view, "Cafe, Saved")
    });
    assert!(fake.scans() >= 1);

    let modern = find(&mut view, Role::Button, "Modern").expect("Modern row");
    click(&mut view, modern);
    assert!(
        view.take_commands().is_empty(),
        "a secured network waits for its password"
    );
    assert!(has_text(&mut view, "Password for “Modern”"));
    for character in "guess".chars() {
        press(&mut view, Key::Character(character));
    }
    press(&mut view, Key::Enter);
    pump(&mut view, &feed, &router, |view| {
        has_text(view, "The password is not correct")
    });
    assert!(!fake.saved_ssids().contains(&b"Modern".to_vec()));

    press(&mut view, Key::Escape);
    assert!(!has_text(&mut view, "Password for"));

    let power = find(&mut view, Role::Switch, "Wi-Fi").expect("Wi-Fi switch");
    click(&mut view, power);
    pump(&mut view, &feed, &router, |_| {
        !fake.wireless_enabled().expect("manager")
    });
    pump(&mut view, &feed, &router, |view| {
        find(view, Role::Button, "Cafe").is_none()
    });
}

#[test]
fn bluetooth_pairs_after_the_code_is_confirmed_in_settings() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let fake = FakeBluez::start(&bus).expect("fake BlueZ");
    let system = bus.connect().expect("system connection");
    let (feed, updates) = feed().expect("feed");
    let router = Router {
        network: None,
        bluetooth: Some(follow_bluetooth(&system, updates)),
    };
    let mut view = view();
    pump(&mut view, &feed, &router, |view| {
        view.visible_panes().contains(&PaneId::Bluetooth)
    });
    open(&mut view, "Bluetooth");
    pump(&mut view, &feed, &router, |_| {
        fake.discovering().expect("adapter")
    });
    assert!(has_text(&mut view, "Studio Headphones, Connected"));

    let connect = button_in_row(&mut view, "mouse", "Connect");
    click(&mut view, connect);
    pump(&mut view, &feed, &router, |view| {
        has_text(view, "mouse, Connected")
    });

    let pair = button_in_row(&mut view, "Phone", "Pair");
    click(&mut view, pair);
    pump(&mut view, &feed, &router, |view| {
        has_text(view, "Does Phone show the code 123456?")
    });
    let accept = button_in_row(&mut view, "Does Phone", "Pair");
    click(&mut view, accept);
    pump(&mut view, &feed, &router, |_| {
        fake.device(PHONE).expect("phone").paired
    });
    pump(&mut view, &feed, &router, |view| {
        has_text(view, "Phone, Connected")
    });
    assert!(!has_text(&mut view, "Does Phone"));

    let off = find(&mut view, Role::Switch, "Bluetooth").expect("switch");
    click(&mut view, off);
    pump(&mut view, &feed, &router, |_| {
        !fake.powered().expect("adapter")
    });
}

fn device(id: u32, name: &str, direction: Direction, volume: f32) -> Device {
    Device {
        id,
        name: name.to_owned(),
        description: name.replace('.', " "),
        direction,
        level: Level {
            volume,
            muted: false,
        },
    }
}

#[test]
fn sound_shows_both_directions_and_sends_level_and_device_changes() {
    let mut view = view();
    view.apply(Update::Audio(Some(Snapshot {
        generation: 1,
        connected: true,
        outputs: vec![
            device(1, "Built-in.Speakers", Direction::Output, 0.5),
            device(2, "USB.Headset", Direction::Output, 0.3),
        ],
        inputs: vec![device(3, "Built-in.Microphone", Direction::Input, 0.8)],
        apps: Vec::new(),
        default_output: Some("Built-in.Speakers".to_owned()),
        default_input: Some("Built-in.Microphone".to_owned()),
    })));
    open(&mut view, "Sound");
    assert_eq!(
        view.value(SettingId::OutputVolume),
        Some(&Value::Fraction(0.5))
    );
    assert_eq!(
        view.value(SettingId::InputVolume),
        Some(&Value::Fraction(0.8))
    );

    let headset = find(&mut view, Role::Button, "USB Headset").expect("headset row");
    click(&mut view, headset);
    let mute = find(&mut view, Role::Switch, "Mute").expect("mute");
    click(&mut view, mute);
    let volume = find(&mut view, Role::Slider, "Output volume").expect("volume");
    view.perform(&ActionRequest {
        action: Action::Increment,
        target_tree: TreeId::ROOT,
        target_node: volume,
        data: None,
    });
    assert_eq!(
        view.take_commands(),
        [
            Command::Audio(AudioCommand::Listen(true)),
            Command::Audio(AudioCommand::Default(
                Direction::Output,
                "USB.Headset".to_owned()
            )),
            Command::Audio(AudioCommand::Mute(Direction::Output, true)),
            Command::Audio(AudioCommand::Volume(Direction::Output, 0.5625)),
        ]
    );
    assert_eq!(
        view.value(SettingId::OutputMuted),
        Some(&Value::Flag(false))
    );
}

#[test]
fn sound_offers_the_alert_effects_a_live_input_level_and_a_volume_per_app() {
    let mut view = view();
    view.apply(Update::Values(vec![
        (SettingId::AlertSound, Value::Keyword("mallet".to_owned())),
        (SettingId::AlertVolume, Value::Fraction(0.75)),
        (SettingId::SoundEffects, Value::Flag(true)),
        (SettingId::VolumeFeedback, Value::Flag(true)),
    ]));
    view.apply(Update::Audio(Some(Snapshot {
        generation: 1,
        connected: true,
        outputs: vec![device(1, "Built-in.Speakers", Direction::Output, 0.5)],
        inputs: vec![device(3, "Built-in.Microphone", Direction::Input, 0.8)],
        apps: vec![AppStream {
            ids: vec![40, 41],
            name: "Radio".to_owned(),
            icon: None,
            level: Level {
                volume: 0.5,
                muted: false,
            },
        }],
        default_output: Some("Built-in.Speakers".to_owned()),
        default_input: Some("Built-in.Microphone".to_owned()),
    })));
    open(&mut view, "Sound");
    assert_eq!(
        view.take_commands(),
        [Command::Audio(AudioCommand::Listen(true))]
    );

    view.apply(Update::InputLevel(1.0));
    let level = nodes(&mut view)
        .into_iter()
        .find(|(_, node)| node.role() == Role::Meter && node.label() == Some("Input level"))
        .map(|(_, node)| node)
        .expect("an input level");
    assert_eq!(level.numeric_value(), Some(100.0));

    let feedback = find(
        &mut view,
        Role::Switch,
        "Play feedback when volume is changed",
    )
    .expect("feedback switch");
    click(&mut view, feedback);
    let radio = find(&mut view, Role::Slider, "Radio volume").expect("a slider for Radio");
    view.perform(&ActionRequest {
        action: Action::Increment,
        target_tree: TreeId::ROOT,
        target_node: radio,
        data: None,
    });
    assert_eq!(
        view.take_commands(),
        [
            Command::Set(SettingId::VolumeFeedback, Value::Flag(false)),
            Command::Audio(AudioCommand::App("Radio".to_owned(), 0.5625)),
        ]
    );
    assert!(nodes(&mut view).iter().any(|(_, node)| {
        node.role() == Role::ComboBox
            && node.label() == Some("Alert sound")
            && node.value() == Some("Mallet")
    }));

    open(&mut view, "Bar");
    assert!(
        view.take_commands()
            .contains(&Command::Audio(AudioCommand::Listen(false)))
    );
}

fn display(connector: &str, rates: &[i32]) -> Display {
    let modes: Vec<Mode> = rates
        .iter()
        .map(|rate| Mode {
            width: 3024,
            height: 1964,
            millihertz: rate * 1000,
        })
        .collect();
    Display {
        connector: connector.to_owned(),
        identity: format!("Built-in display ({connector})"),
        current: modes.first().copied(),
        modes,
        scale: 1.0,
        ..Display::default()
    }
}

#[test]
fn a_display_change_keeps_its_other_settings_and_only_offers_what_the_panel_can() {
    let mut view = view();
    view.set_displays(vec![
        display("eDP-1", &[120, 60, 24]),
        display("HDMI-A-1", &[60]),
    ]);
    view.apply(Update::Displays(BTreeMap::from([(
        "Built-in display (eDP-1)".to_owned(),
        DisplaySettings {
            refresh_rate: Some(120.0),
            variable_refresh_rate: Some(true),
            ..DisplaySettings::default()
        },
    )])));
    open(&mut view, "Displays");

    let scales = nodes(&mut view)
        .into_iter()
        .filter(|(_, node)| {
            node.role() == Role::RadioGroup && node.label() == Some("Size of text and controls")
        })
        .count();
    assert_eq!(scales, 2);
    let rates: Vec<Option<String>> = nodes(&mut view)
        .into_iter()
        .filter(|(_, node)| node.role() == Role::RadioGroup && node.label() == Some("Refresh rate"))
        .map(|(_, node)| node.value().map(str::to_owned))
        .collect();
    assert_eq!(rates, [Some("120 Hz".to_owned())]);
    assert!(find(&mut view, Role::Switch, "Variable refresh rate").is_some());

    let scale = find(&mut view, Role::RadioGroup, "Size of text and controls").expect("scale");
    view.perform(&ActionRequest {
        action: Action::Increment,
        target_tree: TreeId::ROOT,
        target_node: scale,
        data: None,
    });
    assert_eq!(
        view.take_commands(),
        [Command::Display(
            "Built-in display (eDP-1)".to_owned(),
            DisplaySettings {
                scale: Some(1.0),
                refresh_rate: Some(120.0),
                variable_refresh_rate: Some(true),
                ..DisplaySettings::default()
            }
        )]
    );
}

fn perform(view: &mut SettingsView, target: NodeId, action: Action) {
    view.perform(&ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target,
        data: None,
    });
}

#[test]
fn a_display_is_made_main_and_a_new_resolution_comes_back_unless_kept() {
    let mut view = view();
    let mode = |width: i32, height: i32| Mode {
        width,
        height,
        millihertz: 60_000,
    };
    let external = Display {
        connector: "DP-1".to_owned(),
        identity: "Dell DELL U2720Q 7ZD5K53".to_owned(),
        make: "Dell".to_owned(),
        model: "DELL U2720Q".to_owned(),
        modes: vec![mode(3840, 2160), mode(2560, 1440), mode(1280, 1024)],
        current: Some(mode(3840, 2160)),
        position: (1512, 0),
        scale: 2.0,
        rotation: 0,
    };
    view.set_displays(vec![display("eDP-1", &[60]), external]);
    open(&mut view, "Displays");
    assert!(has_text(&mut view, "Arrangement"));
    assert!(has_text(&mut view, "DELL U2720Q"));

    let uses: Vec<NodeId> = nodes(&mut view)
        .into_iter()
        .filter(|(_, node)| node.role() == Role::RadioGroup && node.label() == Some("Use as"))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(uses.len(), 2);
    perform(&mut view, uses[1], Action::Decrement);
    let placement = |display: &str, x: i32, main: bool| Placement {
        display: display.to_owned(),
        x,
        y: 0,
        main,
        mirror: None,
    };
    assert_eq!(
        view.take_commands(),
        [Command::Arrangement(vec![
            placement("Built-in display (eDP-1)", 0, false),
            placement("Dell DELL U2720Q 7ZD5K53", 1512, true),
        ])]
    );

    let resolution = find(&mut view, Role::RadioGroup, "Resolution").expect("resolutions");
    assert_eq!(
        value_of(&mut view, Role::RadioGroup, "Resolution").as_deref(),
        Some("3840 × 2160")
    );
    perform(&mut view, resolution, Action::Increment);
    let chosen = DisplaySettings {
        resolution: Some((2560, 1440)),
        ..DisplaySettings::default()
    };
    assert_eq!(
        view.take_commands(),
        [Command::Display(
            "Dell DELL U2720Q 7ZD5K53".to_owned(),
            chosen
        )]
    );
    assert!(has_text(&mut view, "Keep these display settings?"));

    view.tick(Instant::now() + Duration::from_secs(16));

    assert_eq!(
        view.take_commands(),
        [Command::Display(
            "Dell DELL U2720Q 7ZD5K53".to_owned(),
            DisplaySettings::default()
        )]
    );
    assert!(!has_text(&mut view, "Keep these display settings?"));
}

fn pair(name: &str) -> Pair {
    Pair {
        light: PathBuf::from(format!("/usr/share/unbound/wallpapers/{name}-light.svg")),
        dark: PathBuf::from(format!("/usr/share/unbound/wallpapers/{name}-dark.svg")),
    }
}

fn value_of(view: &mut SettingsView, role: Role, label: &str) -> Option<String> {
    nodes(view)
        .into_iter()
        .find(|(_, node)| node.role() == role && node.label() == Some(label))
        .and_then(|(_, node)| node.value().map(str::to_owned))
}

#[test]
fn the_wallpaper_pane_offers_the_bundled_pairs_and_a_picture_of_ones_own() {
    let mut view = view();
    view.apply(Update::Pictures(vec![
        ("aurora".to_owned(), pair("aurora")),
        ("drift".to_owned(), pair("drift")),
    ]));
    view.apply(Update::Values(vec![(
        SettingId::Wallpaper,
        Value::Pictures {
            light: String::new(),
            dark: String::new(),
        },
    )]));
    open(&mut view, "Wallpaper");
    assert_eq!(
        value_of(&mut view, Role::RadioGroup, "Wallpaper").as_deref(),
        Some("Drift")
    );

    let grid = find(&mut view, Role::RadioGroup, "Wallpaper").expect("pictures");
    view.perform(&ActionRequest {
        action: Action::Decrement,
        target_tree: TreeId::ROOT,
        target_node: grid,
        data: None,
    });
    let chosen = Value::Pictures {
        light: "/usr/share/unbound/wallpapers/aurora-light.svg".to_owned(),
        dark: "/usr/share/unbound/wallpapers/aurora-dark.svg".to_owned(),
    };
    assert_eq!(
        view.take_commands(),
        [Command::Set(SettingId::Wallpaper, chosen)]
    );

    view.apply(Update::Values(vec![(
        SettingId::Wallpaper,
        Value::Pictures {
            light: "/home/me/Pictures/lake.jpg".to_owned(),
            dark: String::new(),
        },
    )]));
    assert_eq!(
        value_of(&mut view, Role::RadioGroup, "Wallpaper").as_deref(),
        Some("lake")
    );
    let lake = Pair {
        light: PathBuf::from("/home/me/Pictures/lake.jpg"),
        dark: PathBuf::from("/home/me/Pictures/lake.jpg"),
    };
    assert_eq!(
        view.take_commands(),
        [Command::Picture(PictureCommand::Thumbnail(lake))]
    );

    let add = button_in_row(&mut view, "Your own picture", "Add Picture…");
    click(&mut view, add);
    assert_eq!(
        view.take_commands(),
        [Command::Picture(PictureCommand::Choose)]
    );
}

#[test]
fn bundled_wallpapers_are_the_pictures_that_come_as_a_light_and_dark_pair() {
    let data = tempfile::tempdir().expect("data dir");
    let folder = data.path().join("unbound/wallpapers");
    std::fs::create_dir_all(&folder).expect("folder");
    for file in [
        "sea-light.svg",
        "sea-dark.svg",
        "dune-light.svg",
        "notes.txt",
    ] {
        std::fs::write(folder.join(file), "<svg/>").expect("file");
    }
    assert_eq!(
        bundled_pairs(&[data.path().to_path_buf()]),
        [(
            "sea".to_owned(),
            Pair {
                light: folder.join("sea-light.svg"),
                dark: folder.join("sea-dark.svg"),
            }
        )]
    );
}

#[test]
fn the_bar_pane_has_position_size_names_auto_hide_and_the_menu_bar() {
    let mut view = view();
    view.apply(Update::Values(vec![
        (SettingId::BarIconSize, Value::Number(40)),
        (SettingId::BarEdge, Value::Keyword("left".to_owned())),
        (SettingId::BarAutoHide, Value::Flag(false)),
        (SettingId::BarNames, Value::Flag(false)),
        (SettingId::BarMenuBar, Value::Keyword("in-bar".to_owned())),
    ]));
    open(&mut view, "Bar");
    let size = find(&mut view, Role::Slider, "Size").expect("size slider");
    view.perform(&ActionRequest {
        action: Action::Increment,
        target_tree: TreeId::ROOT,
        target_node: size,
        data: None,
    });
    assert_eq!(
        view.take_commands(),
        [Command::Set(SettingId::BarIconSize, Value::Number(44))]
    );
    assert!(find(&mut view, Role::Switch, "Automatically hide").is_some());
    assert!(find(&mut view, Role::Switch, "Show names").is_some());
    assert!(find(&mut view, Role::RadioGroup, "Menu bar").is_some());
}

const RULES: &str = r#"<xkbConfigRegistry><layoutList>
<layout><configItem><name>se</name><shortDescription>sv</shortDescription><description>Swedish</description></configItem></layout>
<layout><configItem><name>us</name><shortDescription>en</shortDescription><description>English (US)</description></configItem>
<variantList><variant><configItem><name>dvorak</name><description>English (Dvorak)</description></configItem></variant></variantList></layout>
<layout><configItem><name>de</name><shortDescription>de</shortDescription><description>German</description></configItem></layout>
</layoutList></xkbConfigRegistry>"#;

fn keyboard_view(codes: &[&str]) -> SettingsView {
    let mut view = view();
    view.set_layouts(Layouts::parse(RULES));
    view.apply(Update::Values(vec![(
        SettingId::InputSources,
        Value::Codes(codes.iter().map(|code| (*code).to_owned()).collect()),
    )]));
    open(&mut view, "Keyboard");
    view
}

fn codes(codes: &[&str]) -> Command {
    Command::Set(
        SettingId::InputSources,
        Value::Codes(codes.iter().map(|code| (*code).to_owned()).collect()),
    )
}

#[test]
fn input_sources_are_reordered_and_removed_but_never_emptied() {
    let mut view = keyboard_view(&["se", "us"]);
    assert!(has_text(&mut view, "English (US)"));

    let down = button_in_row(&mut view, "Swedish", "↓");
    click(&mut view, down);
    assert_eq!(view.take_commands(), [codes(&["us", "se"])]);

    let remove = button_in_row(&mut view, "Swedish", "Remove");
    click(&mut view, remove);
    assert_eq!(view.take_commands(), [codes(&["us"])]);
    assert!(find(&mut view, Role::Button, "Remove").is_none());
}

#[test]
fn a_layout_is_added_by_searching_its_name_and_pressing_return() {
    let mut view = keyboard_view(&["se"]);
    let add = find(&mut view, Role::Button, "Add Input Source…").expect("add button");
    click(&mut view, add);
    for character in "germ".chars() {
        press(&mut view, Key::Character(character));
    }
    assert_eq!(view.adding_source(), Some("germ"));
    assert!(has_text(&mut view, "German"));
    assert!(!has_text(&mut view, "English (Dvorak)"));
    assert_eq!(view.adding_source(), Some("germ"));

    press(&mut view, Key::Enter);
    assert_eq!(view.take_commands(), [codes(&["se", "de"])]);
    assert_eq!(view.adding_source(), None);
}

#[test]
fn escape_stops_adding_a_layout_and_a_full_list_offers_no_more() {
    let mut view = keyboard_view(&["se"]);
    let add = find(&mut view, Role::Button, "Add Input Source…").expect("add button");
    click(&mut view, add);
    press(&mut view, Key::Escape);
    assert_eq!(view.adding_source(), None);
    assert!(view.take_commands().is_empty());

    let mut full = keyboard_view(&["se", "us", "de", "us(dvorak)"]);
    assert!(find(&mut full, Role::Button, "Add Input Source…").is_none());
}

#[test]
fn the_keyboard_pane_lists_the_desktop_shortcuts_with_key_symbols() {
    let mut view = keyboard_view(&["se"]);
    assert!(has_text(&mut view, "Alt+Super+Space"));
    assert!(has_text(&mut view, "Lock Screen"));
}

#[test]
fn trackpad_settings_appear_only_with_a_trackpad_and_mouse_settings_only_with_a_mouse() {
    let mut view = view();
    view.apply(Update::Values(vec![
        (SettingId::TrackpadTapToClick, Value::Flag(true)),
        (SettingId::MouseNaturalScrolling, Value::Flag(false)),
        (SettingId::MouseSpeed, Value::Number(5)),
    ]));
    assert!(find(&mut view, Role::ListItem, "Trackpad & Mouse").is_none());

    view.set_pointing_devices(PointingDevices {
        trackpad: false,
        mouse: true,
    });
    open(&mut view, "Trackpad & Mouse");
    assert!(find(&mut view, Role::Switch, "Tap to click").is_none());
    let natural = find(&mut view, Role::Switch, "Natural scrolling").expect("mouse scrolling");
    click(&mut view, natural);
    assert_eq!(
        view.take_commands(),
        [Command::Set(
            SettingId::MouseNaturalScrolling,
            Value::Flag(true)
        )]
    );

    view.set_pointing_devices(PointingDevices {
        trackpad: true,
        mouse: true,
    });
    assert!(find(&mut view, Role::Switch, "Tap to click").is_some());
}
