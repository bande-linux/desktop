use std::ffi::OsStr;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::sync::{Arc, Mutex};

use bande_compositor::{CursorTheme, export_session_environment, notify_ready};
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
    notify_ready(Some(path.as_os_str())).expect("notified");
    assert_eq!(received(&socket), "READY=1");
    std::fs::remove_file(&path).expect("remove the notify socket");
}

#[test]
fn readiness_reaches_an_abstract_notify_socket() {
    let name = format!("bande-notify-abstract-{}", std::process::id());
    let address = SocketAddr::from_abstract_name(name.as_bytes()).expect("abstract address");
    let socket = UnixDatagram::bind_addr(&address).expect("notify socket");
    notify_ready(Some(OsStr::new(&format!("@{name}")))).expect("notified");
    assert_eq!(received(&socket), "READY=1");
}
