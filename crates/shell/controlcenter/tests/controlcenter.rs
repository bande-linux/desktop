#![allow(clippy::expect_used, clippy::float_cmp)]

use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Node, NodeId, Role, Toggled, TreeId};
use unbound_audio::Level;
use unbound_controlcenter::{
    Change, Changes, Hud, KeyboardLight, SEGMENTS, Status, panel, stepped,
};
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
fn the_keyboard_backlight_slider_appears_only_with_a_backlight_and_asks_for_its_level() {
    let (ui, _, _) = open(&Status::default());
    assert!(control(&ui, "Keyboard Brightness").is_none());

    let (mut ui, _, changes) = open(&Status {
        keyboard_brightness: Some(0.5),
        ..Status::default()
    });
    let (slider, node) = ui
        .accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| node.role() == Role::Slider)
        .expect("a keyboard brightness slider");
    assert_eq!(node.label(), Some("Keyboard Brightness"));
    assert!(ui.perform(&ActionRequest {
        action: Action::Increment,
        target_tree: TreeId::ROOT,
        target_node: slider,
        data: None,
    }));
    assert_eq!(*changes.borrow(), [Change::KeyboardBrightness(9.0 / 16.0)]);
}

#[test]
fn every_switch_is_named_on_screen_beside_it() {
    let (ui, _, _) = open(&Status {
        wifi: Some(false),
        bluetooth: Some(false),
        ..Status::default()
    });
    let shown: Vec<String> = ui
        .accessibility()
        .nodes
        .into_iter()
        .filter(|(_, node)| node.role() == Role::Label)
        .filter_map(|(_, node)| node.value().map(str::to_owned))
        .collect();

    for name in ["Wi-Fi", "Bluetooth", "Do Not Disturb"] {
        assert!(shown.iter().any(|text| text == name), "{name} in {shown:?}");
    }
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

fn filled(hud: Hud) -> f32 {
    hud.segment_fills().iter().sum()
}

#[test]
fn the_level_display_fills_a_segment_per_step_and_none_when_muted() {
    let volume = |volume, muted| filled(Hud::Volume(Level { volume, muted }));
    assert_eq!(volume(1.0, false), SEGMENTS as f32);
    assert_eq!(volume(0.5, false), SEGMENTS as f32 / 2.0);
    assert_eq!(volume(1.0 / SEGMENTS as f32, false), 1.0);
    assert_eq!(volume(0.0, false), 0.0);
    assert_eq!(volume(0.75, true), 0.0);
    assert_eq!(filled(Hud::Display(0.25)), 4.0);
    assert_eq!(filled(Hud::Keyboard(1.0)), SEGMENTS as f32);
}

#[test]
fn a_quarter_step_fills_a_quarter_of_a_segment() {
    let level = stepped(0.5, true, true);
    assert_eq!(level, 33.0 / 64.0);
    let fills = Hud::Display(level).segment_fills();
    assert_eq!(fills[7], 1.0);
    assert_eq!(fills[8], 0.25);
    assert_eq!(fills[9], 0.0);
}

#[test]
fn brightness_keys_move_one_of_sixteen_steps_and_stop_at_the_ends() {
    assert_eq!(stepped(0.5, true, false), 9.0 / 16.0);
    assert_eq!(stepped(0.3, false, false), 4.0 / 16.0);
    assert_eq!(stepped(1.0, true, false), 1.0);
    assert_eq!(stepped(0.0, false, false), 0.0);
}

#[test]
fn the_keyboard_backlight_is_found_among_the_leds() {
    let leds = tempfile::tempdir().expect("temporary directory");
    for (name, brightness) in [("input3::capslock", "1"), ("kbd_backlight", "128")] {
        let directory = leds.path().join(name);
        std::fs::create_dir_all(&directory).expect("directory");
        std::fs::write(directory.join("brightness"), brightness).expect("brightness");
        std::fs::write(directory.join("max_brightness"), "255\n").expect("maximum");
    }

    let light = KeyboardLight::find(leds.path()).expect("a keyboard backlight");
    assert_eq!(light.name, "kbd_backlight");
    assert_eq!(light.level(), Some(128.0 / 255.0));
    assert_eq!(KeyboardLight::find(&leds.path().join("missing")), None);
}
