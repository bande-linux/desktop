#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use bande_test_bus::SessionBus;
use bande_test_bus::atspi::{FakeRegistry, find_named};
use bande_test_wayland::{Frames, headless};
use zbus::blocking::connection;

const TIMEOUT: Duration = Duration::from_secs(180);

#[test]
fn a_screen_reader_finds_the_title_bar_and_the_terminal_beside_each_other() {
    let bus = SessionBus::start().expect("private session and AT-SPI bus");
    let registry = FakeRegistry::start(&bus).expect("AT-SPI registry");
    let mut server = headless(1024, 768);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let mut terminal = Command::new(env!("CARGO_BIN_EXE_bande-terminal"))
        .env("WAYLAND_SOCKET", "0")
        .env("DBUS_SESSION_BUS_ADDRESS", bus.address())
        .env("AT_SPI_BUS_ADDRESS", bus.address())
        .stdin(Stdio::from(OwnedFd::from(client_end)))
        .spawn()
        .expect("start Terminal");
    let atspi = connection::Builder::address(bus.address())
        .expect("address")
        .method_timeout(Duration::from_secs(2))
        .build()
        .expect("screen reader connection");
    let screen_reader = thread::spawn(move || {
        let root = registry.embedded_root(TIMEOUT).expect("an embedded window");
        let started = Instant::now();
        loop {
            let close = find_named(&atspi, &root, "Close");
            let menus = find_named(&atspi, &root, "Menus");
            if close.is_some() && menus.is_some() {
                break;
            }
            assert!(started.elapsed() < TIMEOUT, "no title bar over AT-SPI");
            thread::sleep(Duration::from_millis(50));
        }
    });

    Frames::start().run_until(&mut server, || {
        screen_reader.is_finished() || terminal.try_wait().expect("Terminal's state").is_some()
    });
    let ended = terminal.try_wait().expect("Terminal's state");
    terminal.kill().expect("stop Terminal");
    terminal.wait().expect("wait for Terminal");
    assert_eq!(ended, None, "Terminal ended when a screen reader was on");
    screen_reader.join().expect("screen reader thread");
}
