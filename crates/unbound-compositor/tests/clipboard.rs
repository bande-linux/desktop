use std::io::Read;

use smithay::utils::Size;
use unbound_compositor::Headless;
use unbound_test_wayland::clipboard::TEXT;
use unbound_test_wayland::{ShmImage, TestClient, quadrants, roundtrip};
use wayland_client::protocol::wl_shm::Format;

fn server() -> Headless {
    Headless::new(Size::from((256, 192))).expect("headless compositor")
}

fn focused_client(server: &mut Headless) -> TestClient {
    let mut client = TestClient::connect(server);
    client.join_seat(server);
    let pixels = quadrants(4, 4, 16);
    client.show_window(
        server,
        &ShmImage {
            width: 4,
            height: 4,
            stride: 16,
            format: Format::Argb8888,
            bytes: &pixels,
        },
    );
    assert!(
        client.has_keyboard_focus(),
        "a newly mapped window takes focus"
    );
    client
}

fn read_all(mut pipe: std::io::PipeReader) -> String {
    let mut text = String::new();
    pipe.read_to_string(&mut text).expect("read pasted data");
    text
}

#[test]
fn text_copied_in_one_client_pastes_in_another() {
    let mut server = server();
    let mut source = focused_client(&mut server);
    source.copy(TEXT, b"copied from the source");
    source.exchange(&mut server);

    let mut target = focused_client(&mut server);
    roundtrip(&mut server, &mut [&mut source, &mut target]);
    assert!(!source.has_keyboard_focus());
    assert_eq!(target.selection_mimes(), Some(vec![TEXT.to_owned()]));

    let pipe = target.request_paste(TEXT);
    roundtrip(&mut server, &mut [&mut source, &mut target]);
    assert_eq!(read_all(pipe), "copied from the source");
}

#[test]
fn a_client_without_keyboard_focus_cannot_set_the_selection() {
    let mut server = server();
    let mut background = focused_client(&mut server);
    let mut foreground = focused_client(&mut server);
    roundtrip(&mut server, &mut [&mut background, &mut foreground]);
    assert!(!background.has_keyboard_focus());

    background.copy(TEXT, b"stolen focus");
    roundtrip(&mut server, &mut [&mut background, &mut foreground]);

    assert_eq!(foreground.selection_mimes(), None);
}

#[test]
fn primary_selection_pastes_in_another_client() {
    let mut server = server();
    let mut source = focused_client(&mut server);
    source.copy_primary(TEXT, b"middle click");
    source.exchange(&mut server);

    let mut target = focused_client(&mut server);
    assert_eq!(target.primary_mimes(), Some(vec![TEXT.to_owned()]));

    let pipe = target.request_primary_paste(TEXT);
    roundtrip(&mut server, &mut [&mut source, &mut target]);
    assert_eq!(read_all(pipe), "middle click");
}

fn server_trusting_this_test() -> Headless {
    let this_test = std::env::current_exe().expect("test executable");
    Headless::with_clipboard_daemons(Size::from((256, 192)), &[this_test.as_path()])
        .expect("headless compositor")
}

#[test]
fn data_control_is_hidden_from_clients_that_are_not_the_clipboard_daemon() {
    let mut server = server();
    let client = TestClient::connect(&mut server);
    assert!(client.advertises("wl_data_device_manager"));
    assert!(!client.advertises("ext_data_control_manager_v1"));
}

#[test]
fn data_control_is_offered_to_the_trusted_executable() {
    let mut server = server_trusting_this_test();
    let client = TestClient::connect(&mut server);
    assert!(client.advertises("ext_data_control_manager_v1"));
}

#[test]
fn a_data_control_manager_keeps_the_selection_after_the_source_exits() {
    let mut server = server_trusting_this_test();
    let mut manager = TestClient::connect(&mut server);
    manager.join_data_control(&mut server);

    let mut source = focused_client(&mut server);
    source.copy(TEXT, b"outlives its source");
    roundtrip(&mut server, &mut [&mut source, &mut manager]);
    let mimes = manager
        .control_mimes()
        .expect("data control sees the selection");
    assert_eq!(mimes, vec![TEXT.to_owned()]);

    let pipe = manager.request_control_paste(TEXT);
    roundtrip(&mut server, &mut [&mut source, &mut manager]);
    let saved = read_all(pipe);
    assert_eq!(saved, "outlives its source");

    manager.control_copy(&mimes, saved.as_bytes());
    roundtrip(&mut server, &mut [&mut source, &mut manager]);

    drop(source);
    manager.exchange(&mut server);

    let mut target = focused_client(&mut server);
    assert_eq!(target.selection_mimes(), Some(vec![TEXT.to_owned()]));
    let pipe = target.request_paste(TEXT);
    roundtrip(&mut server, &mut [&mut manager, &mut target]);
    assert_eq!(read_all(pipe), "outlives its source");
}
