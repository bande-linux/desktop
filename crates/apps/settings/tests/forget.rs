#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use accesskit::{NodeId, Role};
use bande_bluetooth::Bluetooth;
use bande_network::NetworkManager;
use bande_settings::{
    BluetoothCommand, BluetoothState, Command, NetworkCommand, PaneId, SettingsView, Update,
    WifiState,
};
use bande_test_bus::SessionBus;
use bande_test_bus::bluez::FakeBluez;
use bande_test_bus::networkmanager::FakeNetworkManager;
use bande_toolkit::ui::{Event, Key, Modifiers};
use bande_toolkit::{Palette, Scheme, TextSystem};
use support::{SIZE, click, has_text, nodes};

fn view() -> SettingsView {
    SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new())
}

fn button(view: &mut SettingsView, label: &str) -> Vec<NodeId> {
    nodes(view)
        .into_iter()
        .filter(|(_, node)| node.role() == Role::Button && node.label() == Some(label))
        .map(|(id, _)| id)
        .collect()
}

fn escape(view: &mut SettingsView) {
    view.handle(&Event::Key {
        key: Key::Escape,
        modifiers: Modifiers::default(),
    });
    view.scene(SIZE.0, SIZE.1);
}

#[test]
fn a_saved_network_is_forgotten_only_after_confirming() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let _fake = FakeNetworkManager::start(&bus).expect("fake NetworkManager");
    let client = NetworkManager::new(&bus.connect().expect("client"));
    let networks = client.networks().expect("networks");
    let studio = networks
        .iter()
        .find(|network| network.name() == "Studio")
        .cloned()
        .expect("studio");
    let mut view = view();
    view.apply(Update::Wifi(Some(WifiState {
        enabled: true,
        networks,
    })));
    view.select_pane(PaneId::Wifi);
    view.scene(SIZE.0, SIZE.1);
    view.take_commands();

    let forget = button(&mut view, "Forget…");
    assert_eq!(forget.len(), 1, "only the saved network offers Forget");
    click(&mut view, forget[0]);
    assert!(has_text(&mut view, "Forget “Studio”?"));
    escape(&mut view);
    assert!(!has_text(&mut view, "Forget “Studio”?"));
    assert!(view.take_commands().is_empty());

    let forget = button(&mut view, "Forget…");
    click(&mut view, forget[0]);
    let confirm = button(&mut view, "Forget");
    click(&mut view, confirm[0]);
    assert_eq!(
        view.take_commands(),
        [Command::Network(NetworkCommand::Forget(studio))]
    );
    assert!(!has_text(&mut view, "Forget “Studio”?"));
}

#[test]
fn a_paired_device_shows_its_battery_and_is_forgotten_after_confirming() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let _fake = FakeBluez::start(&bus).expect("fake BlueZ");
    let client = Bluetooth::new(&bus.connect().expect("client"));
    let paired = client.paired_devices().expect("paired");
    let mouse = paired
        .iter()
        .find(|device| device.name == "mouse")
        .cloned()
        .expect("mouse");
    let mut view = view();
    view.apply(Update::Bluetooth(Some(BluetoothState {
        powered: true,
        paired,
        nearby: Vec::new(),
    })));
    view.select_pane(PaneId::Bluetooth);
    view.scene(SIZE.0, SIZE.1);
    view.take_commands();
    assert!(has_text(&mut view, "Connected · Battery 80%"));

    let forget = button(&mut view, "Forget…");
    assert_eq!(forget.len(), 2);
    click(&mut view, forget[1]);
    assert!(has_text(&mut view, "Forget “mouse”?"));
    let confirm = button(&mut view, "Forget");
    click(&mut view, confirm[0]);
    assert_eq!(
        view.take_commands(),
        [Command::Bluetooth(BluetoothCommand::Forget(mouse))]
    );
}
