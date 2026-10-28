use std::io::Read;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Child, Command};

use smithay::utils::Size;
use tempfile::TempDir;
use unbound_compositor::Headless;
use unbound_test_wayland::clipboard::TEXT;
use unbound_test_wayland::{ShmImage, TestClient, quadrants, roundtrip, roundtrip_until};
use wayland_client::protocol::wl_shm::Format;

struct Session {
    server: Headless,
    daemon: Child,
    _runtime: TempDir,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

fn session_with_daemon() -> Session {
    let daemon_exe = Path::new(env!("CARGO_BIN_EXE_unbound-clipboard"));
    let mut server = Headless::with_clipboard_daemons(Size::from((256, 192)), &[daemon_exe])
        .expect("headless compositor");
    let runtime = tempfile::tempdir().expect("runtime dir");
    let listener = UnixListener::bind(runtime.path().join("wayland-test")).expect("socket");
    let daemon = Command::new(daemon_exe)
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("WAYLAND_DISPLAY", "wayland-test")
        .env_remove("WAYLAND_SOCKET")
        .spawn()
        .expect("spawn unbound-clipboard");
    let (stream, _) = listener.accept().expect("daemon connects");
    server.insert_client(stream).expect("insert daemon");
    Session {
        server,
        daemon,
        _runtime: runtime,
    }
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
    let mut session = session_with_daemon();
    let server = &mut session.server;
    let mut source = focused_client(server);
    source.copy(TEXT, b"kept by the daemon");
    wait_for_takeover(server, &mut source);
    drop(source);

    let mut target = focused_client(server);
    assert_eq!(paste(server, &mut target), "kept by the daemon");
}

#[test]
fn a_primary_selection_pastes_after_its_source_has_quit() {
    let mut session = session_with_daemon();
    let server = &mut session.server;
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
    let mut session = session_with_daemon();
    let server = &mut session.server;
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
