#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId};
use unbound_audio::{Device, Direction, Level, Snapshot};
use unbound_config::Output;
use unbound_settings::{
    Answer, AudioCommand, BluetoothCommand, Command, Display, Feed, Mode, NetworkCommand, PaneId,
    SettingId, SettingsView, Update, Value, feed, follow_bluetooth, follow_network,
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
        description: format!("Built-in display ({connector})"),
        current: modes.first().copied(),
        modes,
    }
}

#[test]
fn a_display_change_keeps_its_other_settings_and_only_offers_what_the_panel_can() {
    let mut view = view();
    view.set_displays(vec![
        display("eDP-1", &[120, 60, 24]),
        display("HDMI-A-1", &[60]),
    ]);
    view.apply(Update::Outputs(BTreeMap::from([(
        "eDP-1".to_owned(),
        Output {
            scale: None,
            refresh_rate: Some(120.0),
            variable_refresh_rate: Some(true),
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
        [Command::Output(
            "eDP-1".to_owned(),
            Output {
                scale: Some(1.0),
                refresh_rate: Some(120.0),
                variable_refresh_rate: Some(true),
            }
        )]
    );
}
