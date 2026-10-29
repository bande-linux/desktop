use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Node, NodeId, Role, Toggled, TreeId};
use unbound_audio::Level;
use unbound_controlcenter::{Change, Changes, SEGMENTS, Status, filled_segments, panel};
use unbound_toolkit::ui::Ui;
use unbound_toolkit::{Palette, Scheme, TextSystem};

fn open(status: Status) -> (Ui, Changes) {
    let changes: Changes = Rc::new(RefCell::new(Vec::new()));
    let ui = panel(
        status,
        Palette::for_scheme(Scheme::Light),
        TextSystem::new(),
        &changes,
    );
    (ui, changes)
}

fn control(ui: &Ui, label: &str) -> Option<(NodeId, Node)> {
    ui.accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| node.label() == Some(label))
}

#[test]
fn only_the_controls_this_computer_has_are_shown() {
    let (ui, _) = open(Status {
        wifi: None,
        bluetooth: Some(true),
        do_not_disturb: false,
        volume: Some(0.5),
    });

    assert!(
        control(&ui, "Wi-Fi").is_none(),
        "no Wi-Fi device, no Wi-Fi switch"
    );
    let (_, bluetooth) = control(&ui, "Bluetooth").expect("a Bluetooth switch");
    assert_eq!(bluetooth.role(), Role::Switch);
    assert_eq!(bluetooth.toggled(), Some(Toggled::True));
    let (_, sound) = control(&ui, "Sound").expect("a sound slider");
    assert_eq!(sound.numeric_value(), Some(0.5));
}

#[test]
fn switching_a_control_asks_for_that_change() {
    let (mut ui, changes) = open(Status {
        wifi: Some(true),
        bluetooth: Some(false),
        do_not_disturb: false,
        volume: None,
    });
    let (wifi, _) = control(&ui, "Wi-Fi").expect("a Wi-Fi switch");
    let (do_not_disturb, _) = control(&ui, "Do Not Disturb").expect("a Do Not Disturb switch");

    for target in [wifi, do_not_disturb] {
        assert!(ui.perform(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: target,
            data: None,
        }));
    }

    assert_eq!(
        *changes.borrow(),
        [Change::Wifi(false), Change::DoNotDisturb(true)]
    );
}

#[test]
fn the_volume_display_fills_a_segment_per_step_and_none_when_muted() {
    let at = |volume, muted| filled_segments(Level { volume, muted });
    assert_eq!(at(1.0, false), SEGMENTS);
    assert_eq!(at(0.5, false), SEGMENTS / 2);
    assert_eq!(at(1.0 / SEGMENTS as f32, false), 1);
    assert_eq!(at(0.0, false), 0);
    assert_eq!(at(0.75, true), 0);
}
