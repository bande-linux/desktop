use std::ffi::OsStr;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::sync::{Arc, Mutex};

use std::time::Duration;

use bande_compositor::{
    CursorTheme, export_session_environment, notify_service_manager, watchdog_keepalive_interval,
};
use bande_test_bus::SessionBus;

struct Systemd {
    assignments: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
impl Systemd {
    fn set_environment(&self, assignments: Vec<String>) {
        if let Ok(mut recorded) = self.assignments.lock() {
            recorded.extend(assignments);
        }
    }
}

#[test]
fn the_wayland_socket_desktop_and_cursor_theme_reach_the_user_services_environment() {
    let bus = SessionBus::start().expect("private session bus");
    let assignments = Arc::new(Mutex::new(Vec::new()));
    let systemd = bus.connect().expect("systemd stand-in connection");
    systemd
        .object_server()
        .at(
            "/org/freedesktop/systemd1",
            Systemd {
                assignments: Arc::clone(&assignments),
            },
        )
        .expect("serve the manager");
    systemd
        .request_name("org.freedesktop.systemd1")
        .expect("own systemd's name");

    let session = bus.connect().expect("session connection");
    let cursor = CursorTheme {
        search_path: Vec::new(),
        name: "Adwaita".to_owned(),
        size: 32,
    };
    export_session_environment(&session, "wayland-1", &cursor).expect("exported");

    assert_eq!(
        *assignments.lock().expect("assignments"),
        [
            "WAYLAND_DISPLAY=wayland-1",
            "XDG_CURRENT_DESKTOP=Bande",
            "XDG_SESSION_TYPE=wayland",
            "XCURSOR_THEME=Adwaita",
            "XCURSOR_SIZE=32"
        ]
    );
}

#[test]
fn the_shell_starts_in_a_wayland_session_of_the_bande_desktop() {
    let folder =
        std::env::temp_dir().join(format!("bande-shell-environment-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("scratch folder");
    let script = folder.join("shell");
    let recorded = folder.join("environment");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nenv > \"{}\"\n", recorded.display()),
    )
    .expect("write the shell stand-in");
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("make it executable");

    let mut server = crate::support::server();
    let mut shell = server.spawn_shell_client(&script).expect("start the shell");
    shell.wait().expect("the shell stand-in ran");

    let environment = std::fs::read_to_string(&recorded).expect("recorded environment");
    std::fs::remove_dir_all(&folder).expect("remove the scratch folder");
    for wanted in ["XDG_CURRENT_DESKTOP=Bande", "XDG_SESSION_TYPE=wayland"] {
        assert!(
            environment.lines().any(|line| line == wanted),
            "{wanted} missing from {environment}"
        );
    }
}

fn received(socket: &UnixDatagram) -> String {
    let mut message = [0; 64];
    let length = socket.recv(&mut message).expect("readiness message");
    String::from_utf8_lossy(&message[..length]).into_owned()
}

#[test]
fn readiness_reaches_a_notify_socket_at_a_path() {
    let path = std::env::temp_dir().join(format!("bande-notify-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let socket = UnixDatagram::bind(&path).expect("notify socket");
    notify_service_manager(Some(path.as_os_str()), "READY=1").expect("notified");
    assert_eq!(received(&socket), "READY=1");
    std::fs::remove_file(&path).expect("remove the notify socket");
}

#[test]
fn keepalives_reach_an_abstract_notify_socket() {
    let name = format!("bande-notify-abstract-{}", std::process::id());
    let address = SocketAddr::from_abstract_name(name.as_bytes()).expect("abstract address");
    let socket = UnixDatagram::bind_addr(&address).expect("notify socket");
    notify_service_manager(Some(OsStr::new(&format!("@{name}"))), "WATCHDOG=1").expect("notified");
    assert_eq!(received(&socket), "WATCHDOG=1");
}

#[test]
fn the_watchdog_is_reassured_twice_per_timeout_when_it_watches_this_process() {
    let own = std::process::id().to_string();
    assert_eq!(
        watchdog_keepalive_interval(Some(OsStr::new("20000000")), None),
        Some(Duration::from_secs(10))
    );
    assert_eq!(
        watchdog_keepalive_interval(Some(OsStr::new("20000000")), Some(OsStr::new(&own))),
        Some(Duration::from_secs(10))
    );
}

#[test]
fn no_watchdog_is_reassured_when_it_is_off_malformed_or_watches_another_process() {
    let other = (std::process::id() + 1).to_string();
    assert_eq!(watchdog_keepalive_interval(None, None), None);
    assert_eq!(
        watchdog_keepalive_interval(Some(OsStr::new("0")), None),
        None
    );
    assert_eq!(
        watchdog_keepalive_interval(Some(OsStr::new("soon")), None),
        None
    );
    assert_eq!(
        watchdog_keepalive_interval(Some(OsStr::new("20000000")), Some(OsStr::new(&other))),
        None
    );
}
