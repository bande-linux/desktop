#![allow(clippy::expect_used, clippy::float_cmp)]

use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Node, NodeId, Role, Toggled, TreeId};
use std::time::{Duration, Instant};
use unbound_audio::Level;

use unbound_controlcenter::media::{Command, Playback, Player};
use unbound_controlcenter::{
    Anchor, Change, Changes, Choice, Hud, KeyboardLight, SEGMENTS, Status, clock, panel,
    place_beside, stepped,
};
use unbound_toolkit::ui::Ui;
use unbound_toolkit::{DisplayEdge, Palette, Rect, Scheme, TextSystem};

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

fn act(ui: &mut Ui, target: NodeId, action: Action) -> bool {
    ui.perform(&ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target,
        data: None,
    })
}

fn click(ui: &mut Ui, target: NodeId) {
    assert!(act(ui, target, Action::Click));
}

fn choice(key: &str, title: &str) -> Choice {
    Choice {
        key: key.to_owned(),
        title: title.to_owned(),
    }
}

fn song(playback: Playback) -> Player {
    Player {
        bus_name: "org.mpris.MediaPlayer2.tunes".to_owned(),
        identity: "Tunes".to_owned(),
        desktop_entry: None,
        title: "Harbour Lights".to_owned(),
        artist: "Ada".to_owned(),
        art_url: None,
        track: Some("/track/7".to_owned()),
        playback,
        can_play: true,
        can_pause: true,
        can_go_next: true,
        can_go_previous: false,
        can_seek: true,
        can_raise: true,
        position: Some(Duration::from_secs(30)),
        length: Some(Duration::from_secs(200)),
        rate: 1.0,
        sampled: Instant::now(),
    }
}

#[test]
fn now_playing_shows_the_track_and_offers_only_what_the_player_can_do() {
    let (mut ui, _, changes) = open(&Status {
        player: Some(song(Playback::Paused)),
        ..Status::default()
    });
    let (card, node) = control(&ui, "Now Playing").expect("the Now Playing card");
    assert_eq!(node.description(), Some("Harbour Lights, Ada, Tunes"));
    let (previous, node) = control(&ui, "Previous").expect("a previous button");
    assert!(node.is_disabled());
    let (play, _) = control(&ui, "Play").expect("a play button");
    let (next, _) = control(&ui, "Next").expect("a next button");
    let (position, node) = control(&ui, "Position").expect("a position slider");
    assert_eq!(node.numeric_value(), Some(30.0));
    assert_eq!(node.max_numeric_value(), Some(200.0));

    assert!(!act(&mut ui, previous, Action::Click));
    click(&mut ui, play);
    click(&mut ui, next);
    assert!(act(&mut ui, position, Action::Increment));
    click(&mut ui, card);

    assert_eq!(
        *changes.borrow(),
        [
            Change::Media(Command::PlayPause),
            Change::Media(Command::Next),
            Change::Media(Command::SetPosition {
                track: Some("/track/7".to_owned()),
                position: Duration::from_secs(40),
                from: Duration::from_secs(30),
            }),
            Change::RaisePlayer,
        ]
    );
}

#[test]
fn a_playing_track_offers_pause_and_several_players_offer_a_choice() {
    let (ui, _, _) = open(&Status {
        player: Some(song(Playback::Playing)),
        ..Status::default()
    });
    assert!(control(&ui, "Pause").is_some());
    assert!(control(&ui, "Playing From").is_none());

    let (ui, _, _) = open(&Status {
        player: Some(song(Playback::Playing)),
        players: vec![
            choice("org.mpris.MediaPlayer2.tunes", "Tunes"),
            choice("org.mpris.MediaPlayer2.video", "Video"),
        ],
        ..Status::default()
    });
    assert!(control(&ui, "Playing From").is_some());
}

#[test]
fn track_times_read_as_minutes_and_hours() {
    assert_eq!(clock(Duration::from_secs(5)), "0:05");
    assert_eq!(clock(Duration::from_secs(200)), "3:20");
    assert_eq!(clock(Duration::from_secs(3725)), "1:02:05");
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
fn every_switch_is_a_named_tile_that_says_its_state() {
    let (ui, _, _) = open(&Status {
        wifi: Some(false),
        bluetooth: Some(true),
        ..Status::default()
    });
    let switches: Vec<(String, String, Option<Toggled>)> = ui
        .accessibility()
        .nodes
        .into_iter()
        .filter(|(_, node)| node.role() == Role::Switch)
        .map(|(_, node)| {
            (
                node.label().unwrap_or_default().to_owned(),
                node.description().unwrap_or_default().to_owned(),
                node.toggled(),
            )
        })
        .collect();
    let switch = |name: &str, state: &str, toggled: Toggled| {
        (name.to_owned(), state.to_owned(), Some(toggled))
    };

    assert_eq!(
        switches,
        [
            switch("Wi-Fi", "Off", Toggled::False),
            switch("Bluetooth", "On", Toggled::True),
            switch("Do Not Disturb", "Off", Toggled::False),
        ]
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
        outputs: vec![choice("speakers", "Speakers"), choice("hdmi", "Television")],
        output: Some("speakers".to_owned()),
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
    let (_, node) = control(&ui, "Wi-Fi").expect("a Wi-Fi switch");
    assert_eq!(node.description(), Some("Home"));

    assert!(act(&mut ui, wifi, Action::Expand));
    for target in [wifi, do_not_disturb] {
        click(&mut ui, target);
    }

    assert_eq!(
        *changes.borrow(),
        [
            Change::ChooseNetwork,
            Change::Wifi(false),
            Change::DoNotDisturb(true),
        ]
    );
}

#[test]
fn behind_a_captive_portal_the_wifi_tile_asks_to_sign_in() {
    let (ui, _, _) = open(&Status {
        wifi: Some(true),
        network: Some("Cafe".to_owned()),
        sign_in: true,
        ..Status::default()
    });
    let (_, node) = control(&ui, "Wi-Fi").expect("a Wi-Fi switch");
    assert_eq!(node.description(), Some("Sign in required"));
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

#[test]
fn the_microphone_switch_mutes_the_input_and_its_level_and_choice_are_shown() {
    let (ui, _, _) = open(&Status::default());
    assert!(
        control(&ui, "Microphone").is_none(),
        "no input, no microphone"
    );

    let status = Status {
        input_muted: Some(false),
        inputs: vec![
            choice("mic", "Built-in Microphone"),
            choice("usb", "Headset"),
        ],
        input: Some("mic".to_owned()),
        ..Status::default()
    };
    status.input_level.set(0.5);
    let (mut ui, _, changes) = open(&status);
    let (microphone, node) = control(&ui, "Microphone").expect("a microphone switch");
    assert_eq!(node.toggled(), Some(Toggled::True));
    let (_, level) = control(&ui, "Input Level").expect("an input level");
    assert_eq!(level.role(), Role::Meter);
    assert_eq!(level.numeric_value(), Some(50.0));
    assert!(control(&ui, "Input").is_some(), "a choice of two inputs");

    click(&mut ui, microphone);
    assert_eq!(*changes.borrow(), [Change::InputMuted(true)]);
}

#[test]
fn the_panel_opens_beside_its_bar_item_on_every_edge_and_stays_on_screen() {
    let screen = (1920.0, 1080.0);
    let size = (320.0, 400.0);
    let beside = |edge, item| place_beside(Some(Anchor { edge, item }), screen, size);
    let left = beside(DisplayEdge::Left, Rect::new(6.0, 950.0, 52.0, 58.0));
    assert_eq!((left.x, left.y + left.height), (66.0, 1008.0));
    let right = beside(DisplayEdge::Right, Rect::new(1862.0, 950.0, 52.0, 58.0));
    assert_eq!(right.x + right.width, 1854.0);
    let bottom = beside(DisplayEdge::Bottom, Rect::new(1700.0, 1026.0, 52.0, 48.0));
    assert_eq!(
        (bottom.x + bottom.width, bottom.y + bottom.height),
        (1752.0, 1018.0)
    );
    let top = beside(DisplayEdge::Top, Rect::new(10.0, 0.0, 52.0, 24.0));
    assert_eq!((top.x, top.y), (8.0, 32.0));
    let high = beside(DisplayEdge::Left, Rect::new(6.0, 60.0, 52.0, 58.0));
    assert_eq!(high.y, 8.0);
    let unanchored = place_beside(None, screen, size);
    assert_eq!((unanchored.x, unanchored.y), (1592.0, 8.0));
}
