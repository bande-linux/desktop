use std::sync::{Arc, Mutex};

use unbound_compositor::export_session_environment;
use unbound_test_bus::SessionBus;

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
fn the_wayland_socket_and_desktop_reach_the_user_services_environment() {
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
    export_session_environment(&session, "wayland-1").expect("exported");

    assert_eq!(
        *assignments.lock().expect("assignments"),
        [
            "WAYLAND_DISPLAY=wayland-1",
            "XDG_CURRENT_DESKTOP=Unbound",
            "XDG_SESSION_TYPE=wayland"
        ]
    );
}
