#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rustix::io::{FdFlags, fcntl_setfd};
use smithay::utils::Size;
use unbound_compositor::{Headless, SessionRole};
use unbound_test_wayland::clipboard::TEXT;
use unbound_test_wayland::{ShmImage, TestClient, quadrants, roundtrip, roundtrip_until};
use wayland_client::protocol::wl_shm::Format;

const DAEMON: &str = env!("CARGO_BIN_EXE_unbound-clipboard");

fn server() -> Headless {
    Headless::new(Size::from((256, 192))).expect("headless compositor")
}

fn server_with_daemon() -> Headless {
    let mut server = server();
    server
        .start_session_client(Path::new(DAEMON), SessionRole::ClipboardDaemon)
        .expect("start unbound-clipboard");
    server
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
    client
}

fn wait_for_takeover(server: &mut Headless, source: &mut TestClient) {
    roundtrip_until(server, &mut [source], |clients| {
        clients[0].cancelled_sources() == 1
    });
}

fn read_all(mut pipe: std::io::PipeReader) -> String {
    let mut text = String::new();
    pipe.read_to_string(&mut text).expect("read pasted data");
    text
}

fn paste(server: &mut Headless, target: &mut TestClient) -> String {
    assert_eq!(target.selection_mimes(), Some(vec![TEXT.to_owned()]));
    let pipe = target.request_paste(TEXT);
    roundtrip(server, &mut [target]);
    read_all(pipe)
}

#[test]
fn a_copy_pastes_after_its_source_has_quit() {
    let mut server = server_with_daemon();
    let server = &mut server;
    let mut source = focused_client(server);
    source.copy(TEXT, b"kept by the daemon");
    wait_for_takeover(server, &mut source);
    drop(source);

    let mut target = focused_client(server);
    assert_eq!(paste(server, &mut target), "kept by the daemon");
}

#[test]
fn a_primary_selection_pastes_after_its_source_has_quit() {
    let mut server = server_with_daemon();
    let server = &mut server;
    let mut source = focused_client(server);
    source.copy_primary(TEXT, b"selected text");
    wait_for_takeover(server, &mut source);
    drop(source);

    let mut target = focused_client(server);
    assert_eq!(target.primary_mimes(), Some(vec![TEXT.to_owned()]));
    let pipe = target.request_primary_paste(TEXT);
    roundtrip(server, &mut [&mut target]);
    assert_eq!(read_all(pipe), "selected text");
}

#[test]
fn the_latest_copy_wins_after_both_sources_have_quit() {
    let mut server = server_with_daemon();
    let server = &mut server;
    let mut first = focused_client(server);
    first.copy(TEXT, b"first");
    wait_for_takeover(server, &mut first);
    drop(first);

    let mut second = focused_client(server);
    second.copy(TEXT, b"second");
    wait_for_takeover(server, &mut second);
    drop(second);

    let mut target = focused_client(server);
    assert_eq!(paste(server, &mut target), "second");
}

#[test]
fn a_daemon_that_dies_is_restarted_and_keeps_new_copies() {
    let mut server = server_with_daemon();
    let first = server
        .session_client_pid(Path::new(DAEMON))
        .expect("daemon started");
    let killed = Command::new("kill")
        .args(["-KILL", &first.to_string()])
        .status()
        .expect("kill");
    assert!(killed.success());

    let deadline = Instant::now() + Duration::from_secs(10);
    while server.session_client_pid(Path::new(DAEMON)) == Some(first)
        || server.session_client_pid(Path::new(DAEMON)).is_none()
    {
        assert!(
            Instant::now() < deadline,
            "daemon not restarted within 10 s"
        );
        server.dispatch().expect("server dispatch");
        let upkeep = server.keep_session_clients_running();
        assert!(upkeep.failures.is_empty(), "{:?}", upkeep.failures);
        assert_eq!(upkeep.retry_in, None);
        std::thread::sleep(Duration::from_millis(5));
    }

    let server = &mut server;
    let mut source = focused_client(server);
    source.copy(TEXT, b"after the restart");
    wait_for_takeover(server, &mut source);
    drop(source);

    let mut target = focused_client(server);
    assert_eq!(paste(server, &mut target), "after the restart");
}

#[test]
fn the_daemon_binary_on_an_ordinary_connection_gets_no_data_control() {
    let mut server = server();
    let runtime = tempfile::tempdir().expect("runtime dir");
    let socket = runtime.path().join("wayland-test");
    let listener = UnixListener::bind(&socket).expect("listen");
    let connected = UnixStream::connect(&socket).expect("connect");
    let (accepted, _) = listener.accept().expect("accept");
    server.insert_client(accepted).expect("insert client");

    let inherited = connected.as_raw_fd();
    let mut command = Command::new(DAEMON);
    command
        .env("WAYLAND_SOCKET", inherited.to_string())
        .env_remove("WAYLAND_DISPLAY")
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(move || {
            fcntl_setfd(BorrowedFd::borrow_raw(inherited), FdFlags::empty())
                .map_err(std::io::Error::from)
        });
    }
    let mut daemon = command.spawn().expect("exec unbound-clipboard");
    drop(connected);

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        server.dispatch().expect("server dispatch");
        if let Some(status) = daemon.try_wait().expect("wait") {
            break status;
        }
        assert!(Instant::now() < deadline, "daemon still running after 10 s");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(!status.success());

    let observer = TestClient::connect(&mut server);
    assert!(!observer.advertises("ext_data_control_manager_v1"));
}
