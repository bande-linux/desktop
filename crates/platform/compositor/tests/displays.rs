#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_compositor::{Compositor, OutputConfig};
use bande_config::Config;
use bande_test_wayland::keys::{BTN_LEFT, KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_RIGHT};
use bande_test_wayland::{TestClient, chord, headless, roundtrip};

fn frame_of_only_window(
    server: &Compositor,
) -> smithay::utils::Rectangle<i32, smithay::utils::Logical> {
    server.windows()[0].geometry().unwrap()
}

fn monitor(name: &str, serial: &str, width: i32, height: i32) -> OutputConfig {
    OutputConfig {
        serial: serial.to_owned(),
        ..OutputConfig::virtual_display(name, (width, height).into())
    }
}

#[test]
fn the_only_display_can_blink_or_be_swapped_and_the_windows_end_up_where_they_were() {
    let mut server = headless(1000, 800);
    let mut app = TestClient::window(&mut server, "org.example.Editor");
    let frame = frame_of_only_window(&server);
    let first = monitor("HEADLESS-1", "HEADLESS-1", 1000, 800);

    server.remove_output("HEADLESS-1");
    assert_eq!(server.outputs(), ["HEADLESS-1"]);
    assert!(!server.output_powered("HEADLESS-1"));
    assert_eq!(frame_of_only_window(&server), frame);
    assert!(server.connected_displays(&mut |_| false).is_empty());
    server.add_output(first.clone());
    roundtrip(&mut server, &mut [&mut app]);
    assert!(server.output_powered("HEADLESS-1"));
    assert_eq!(frame_of_only_window(&server), frame);

    server.remove_output("HEADLESS-1");
    server.add_output(monitor("HEADLESS-1", "another", 1600, 900));
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(server.outputs(), ["HEADLESS-1"]);
    assert!(server.output_powered("HEADLESS-1"));
    assert_eq!(
        server.output_size(),
        smithay::utils::Size::from((1600, 900))
    );
    assert_ne!(frame_of_only_window(&server), frame);

    server.remove_output("HEADLESS-1");
    server.add_output(first);
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(
        server.output_size(),
        smithay::utils::Size::from((1000, 800))
    );
    assert_eq!(frame_of_only_window(&server), frame);
}

#[test]
fn a_second_display_keeps_its_scale_place_and_windows_across_a_replug() {
    let mut server = headless(1000, 800);
    let mut app = TestClient::window(&mut server, "org.example.Editor");
    let second = monitor("HDMI-A-2", "second", 1600, 1200);
    let config = Config::parse(&format!(
        "display \"{}\" {{\n    scale 2\n}}\n",
        second.identity()
    ))
    .unwrap();
    server.set_display_config(config);
    server.add_output(second.clone());
    let settled = server.output_settings("HDMI-A-2").unwrap();
    assert_eq!(settled.scale, 2.0);

    let frame = frame_of_only_window(&server);
    let inside = (
        f64::from(frame.loc.x) + f64::from(frame.size.w) / 2.0,
        f64::from(frame.loc.y) + f64::from(frame.size.h) / 2.0,
    );
    server.move_pointer_to(inside.into(), 0);
    server.press_button(BTN_LEFT, true, 0);
    server.press_button(BTN_LEFT, false, 0);
    roundtrip(&mut server, &mut [&mut app]);
    chord(&mut server, &[KEY_LEFTSHIFT, KEY_LEFTMETA, KEY_RIGHT]);
    roundtrip(&mut server, &mut [&mut app]);
    let there = frame_of_only_window(&server);
    assert!(there.loc.x >= settled.position.x);

    server.remove_output("HDMI-A-2");
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(server.outputs(), ["HEADLESS-1"]);
    assert!(frame_of_only_window(&server).loc.x < 1000);

    server.add_output(second);
    roundtrip(&mut server, &mut [&mut app]);
    assert_eq!(server.output_settings("HDMI-A-2").unwrap(), settled);
    assert_eq!(frame_of_only_window(&server), there);
}
