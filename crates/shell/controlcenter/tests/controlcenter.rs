#![allow(clippy::expect_used)]

use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Node, NodeId, Role, Toggled, TreeId};
use unbound_audio::Level;
use unbound_controlcenter::{Change, Changes, SEGMENTS, Status, panel, volume_segments};
use unbound_toolkit::ui::Ui;
use unbound_toolkit::{Palette, Scheme, TextSystem};

fn open(status: &Status) -> (Ui, f32, Changes) {
    let changes: Changes = Rc::new(RefCell::new(Vec::new()));
    let (ui, height) = panel(
        status,
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        &changes,
    );
    (ui, height, changes)
}

fn control(ui: &Ui, label: &str) -> Option<(NodeId, Node)> {
    ui.accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| node.label() == Some(label))
}

fn click(ui: &mut Ui, target: NodeId) {
    assert!(ui.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: target,
        data: None,
    }));
}

#[test]
fn only_the_controls_this_computer_has_are_shown() {
    let (ui, _, _) = open(&Status {
        bluetooth: Some(true),
        volume: Some(0.5),
        ..Status::default()
    });

    assert!(
        control(&ui, "Wi-Fi").is_none(),
        "no Wi-Fi device, no Wi-Fi switch"
    );
    assert!(
        control(&ui, "Display").is_none(),
        "no backlight, no brightness"
    );
    let (_, bluetooth) = control(&ui, "Bluetooth").expect("a Bluetooth switch");
    assert_eq!(bluetooth.role(), Role::Switch);
    assert_eq!(bluetooth.toggled(), Some(Toggled::True));
    assert!(
        ui.accessibility()
            .nodes
            .iter()
            .any(|(_, node)| node.role() == Role::Slider && node.numeric_value() == Some(0.5))
    );
}

#[test]
fn the_panel_grows_with_its_controls() {
    let (_, small, _) = open(&Status::default());
    let (_, large, _) = open(&Status {
        wifi: Some(true),
        network: Some("Home".to_owned()),
        bluetooth: Some(false),
        volume: Some(0.3),
        output: Some("Speakers".to_owned()),
        brightness: Some(0.8),
        ..Status::default()
    });
    assert!(large > small + 100.0);
}

#[test]
fn switching_a_control_or_choosing_a_network_asks_for_that_change() {
    let (mut ui, _, changes) = open(&Status {
        wifi: Some(true),
        network: Some("Home".to_owned()),
        bluetooth: Some(false),
        ..Status::default()
    });
    let (wifi, _) = control(&ui, "Wi-Fi").expect("a Wi-Fi switch");
    let (do_not_disturb, _) = control(&ui, "Do Not Disturb").expect("a Do Not Disturb switch");
    let (network, _) = control(&ui, "Home").expect("the network in use");

    for target in [wifi, do_not_disturb, network] {
        click(&mut ui, target);
    }

    assert_eq!(
        *changes.borrow(),
        [
            Change::Wifi(false),
            Change::DoNotDisturb(true),
            Change::ChooseNetwork
        ]
    );
}

#[test]
fn the_volume_display_fills_a_segment_per_step_and_none_when_muted() {
    let at = |volume, muted| volume_segments(Level { volume, muted });
    assert_eq!(at(1.0, false), SEGMENTS);
    assert_eq!(at(0.5, false), SEGMENTS / 2);
    assert_eq!(at(1.0 / SEGMENTS as f32, false), 1);
    assert_eq!(at(0.0, false), 0);
    assert_eq!(at(0.75, true), 0);
}
