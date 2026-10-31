#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::time::Duration;

use bande_compositor::{Compositor, OutputConfig, OutputMode};
use bande_settings::Displays;
use wayland_client::Connection;

#[test]
fn the_displays_pane_lists_each_output_by_connector_with_its_modes() {
    let mut server = Compositor::with_output(OutputConfig {
        name: "eDP-1".to_owned(),
        make: "Apple".to_owned(),
        model: "Built-in Retina Display".to_owned(),
        serial: String::new(),
        physical_size_mm: (302, 196).into(),
        modes: vec![OutputMode {
            size: (3024, 1964).into(),
            refresh: Duration::from_nanos(1_000_000_000 / 120),
        }],
    })
    .expect("headless compositor");
    let (client, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("client");
    let listing = std::thread::spawn(move || {
        Displays::on(Connection::from_socket(client).expect("connection"))
            .map(|displays| displays.list())
    });
    while !listing.is_finished() {
        server.dispatch().expect("dispatch");
        std::thread::sleep(Duration::from_millis(5));
    }
    let displays = listing.join().expect("lister").expect("displays");

    assert_eq!(displays.len(), 1);
    let display = &displays[0];
    assert_eq!(display.connector, "eDP-1");
    assert_eq!(
        display
            .current
            .map(|mode| (mode.width, mode.height, mode.hertz())),
        Some((3024, 1964, 120))
    );
    assert_eq!(display.refresh_rates(), [120]);
    assert_eq!(display.identity, "Apple Built-in Retina Display");
    assert_eq!(display.position, (0, 0));
}
