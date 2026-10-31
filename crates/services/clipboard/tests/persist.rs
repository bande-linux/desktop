#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rustix::io::{FdFlags, fcntl_setfd};
use smithay::utils::Size;
use unbound_clipboard::Item;
use unbound_compositor::{Compositor, SessionRole};
use unbound_test_bus::SessionBus;
use unbound_test_wayland::clipboard::TEXT;
use unbound_test_wayland::{ShmImage, TestClient, quadrants, roundtrip, roundtrip_until};
use wayland_client::protocol::wl_shm::Format;

const DAEMON: &str = env!("CARGO_BIN_EXE_unbound-clipboard");

fn server() -> Compositor {
    Compositor::new(Size::from((256, 192))).expect("headless compositor")
}

struct Daemon {
    folder: tempfile::TempDir,
    launcher: PathBuf,
}

impl Daemon {
    fn new(bus: Option<&SessionBus>) -> Self {
        let folder = tempfile::tempdir().expect("state folder");
        let launcher = folder.path().join("unbound-clipboard");
        let address = bus.map_or("unix:path=/nonexistent", SessionBus::address);
        std::fs::write(
            &launcher,
            format!(
                "#!/bin/sh\nexport XDG_STATE_HOME='{}'\nexport DBUS_SESSION_BUS_ADDRESS='{address}'\nexec '{DAEMON}'\n",
                folder.path().display()
            ),
        )
        .expect("write the launcher");
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700))
            .expect("make it executable");
        Self { folder, launcher }
    }

    fn history(&self) -> PathBuf {
        self.folder.path().join("unbound/clipboard")
    }
}

fn server_with(daemon: &Daemon) -> Compositor {
    let mut server = server();
    server
        .start_session_client(&daemon.launcher, SessionRole::ClipboardDaemon)
        .expect("start unbound-clipboard");
    server
}

fn server_with_daemon() -> (Compositor, Daemon) {
    let daemon = Daemon::new(None);
    (server_with(&daemon), daemon)
}

fn focused_client(server: &mut Compositor) -> TestClient {
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

fn wait_for_takeover(server: &mut Compositor, source: &mut TestClient) {
    roundtrip_until(server, &mut [source], |clients| {
        clients[0].cancelled_sources() == 1
    });
}

fn read_all(mut pipe: std::io::PipeReader) -> String {
    let mut text = String::new();
    pipe.read_to_string(&mut text).expect("read pasted data");
    text
}

fn paste(server: &mut Compositor, target: &mut TestClient) -> String {
    assert_eq!(target.selection_mimes(), Some(vec![TEXT.to_owned()]));
    let pipe = target.request_paste(TEXT);
    roundtrip(server, &mut [target]);
    read_all(pipe)
}

#[test]
fn a_copy_pastes_after_its_source_has_quit() {
    let (mut server, daemon) = server_with_daemon();
    let server = &mut server;
    let mut source = focused_client(server);
    source.copy(TEXT, b"kept by the daemon");
    wait_for_takeover(server, &mut source);
    drop(source);

    let mut target = focused_client(server);
    assert_eq!(paste(server, &mut target), "kept by the daemon");
    assert_eq!(std::fs::read_dir(daemon.history()).unwrap().count(), 1);
}

#[test]
fn a_primary_selection_pastes_after_its_source_has_quit() {
    let (mut server, daemon) = server_with_daemon();
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
    assert!(!daemon.history().exists());
}

#[test]
fn the_latest_copy_wins_after_both_sources_have_quit() {
    let (mut server, _daemon) = server_with_daemon();
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
    let (mut server, daemon) = server_with_daemon();
    let first = server
        .session_client_pid(&daemon.launcher)
        .expect("daemon started");
    let killed = Command::new("kill")
        .args(["-KILL", &first.to_string()])
        .status()
        .expect("kill");
    assert!(killed.success());

    let deadline = Instant::now() + Duration::from_secs(10);
    while server.session_client_pid(&daemon.launcher) == Some(first)
        || server.session_client_pid(&daemon.launcher).is_none()
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

#[zbus::proxy(
    interface = "org.unbound.Clipboard",
    default_service = "org.unbound.Clipboard",
    default_path = "/org/unbound/Clipboard"
)]
trait Clipboard {
    fn items(&self) -> zbus::Result<Vec<Item>>;
    fn content(&self, id: u64, mime: &str) -> zbus::Result<Vec<u8>>;
    fn paste(&self, id: u64) -> zbus::Result<()>;
    fn remove(&self, id: u64) -> zbus::Result<()>;
    fn clear(&self) -> zbus::Result<()>;
}

fn history_proxy(server: &mut Compositor, bus: &SessionBus) -> ClipboardProxyBlocking<'static> {
    let connection = bus.connect().expect("connect to the test bus");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        server.dispatch().expect("server dispatch");
        if let Ok(proxy) = ClipboardProxyBlocking::new(&connection)
            && proxy.items().is_ok()
        {
            return proxy;
        }
        assert!(Instant::now() < deadline, "no clipboard history on the bus");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn earlier_copies_can_be_listed_and_put_back_on_the_clipboard() {
    let bus = SessionBus::start().expect("a test bus");
    let daemon = Daemon::new(Some(&bus));
    let mut server = server_with(&daemon);
    let history = history_proxy(&mut server, &bus);
    let server = &mut server;
    for text in [&b"first copy"[..], b"second copy"] {
        let mut source = focused_client(server);
        source.copy(TEXT, text);
        wait_for_takeover(server, &mut source);
        drop(source);
    }

    let items = history.items().expect("items");
    let texts: Vec<&str> = items.iter().map(|item| item.2.as_str()).collect();
    assert_eq!(texts, ["second copy", "first copy"]);
    assert_eq!(items[1].3, [TEXT]);
    assert_eq!(
        history.content(items[1].0, TEXT).expect("content"),
        b"first copy"
    );

    history.paste(items[1].0).expect("paste");
    let mut target = focused_client(server);
    roundtrip_until(server, &mut [&mut target], |clients| {
        clients[0].selection_mimes().is_some()
    });
    let pipe = target.request_paste(TEXT);
    let pasted = std::thread::spawn(move || read_all(pipe));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !pasted.is_finished() {
        assert!(Instant::now() < deadline, "nothing pasted");
        roundtrip(server, &mut [&mut target]);
    }
    assert_eq!(pasted.join().expect("pasted"), "first copy");
    let texts: Vec<String> = history
        .items()
        .expect("items")
        .into_iter()
        .map(|item| item.2)
        .collect();
    assert_eq!(texts, ["first copy", "second copy"]);

    history.remove(items[0].0).expect("remove");
    assert_eq!(history.items().expect("items").len(), 1);
    history.clear().expect("clear");
    assert!(history.items().expect("items").is_empty());
    assert_eq!(std::fs::read_dir(daemon.history()).unwrap().count(), 0);
}
