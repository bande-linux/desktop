#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use unbound_test_bus::SessionBus;
use zbus::blocking::fdo::DBusProxy;

#[test]
fn a_test_bus_activates_none_of_the_installed_session_services() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let connection = bus.connect().expect("connection");
    let proxy = DBusProxy::new(&connection).expect("bus proxy");

    let activatable = proxy
        .list_activatable_names()
        .expect("ListActivatableNames");

    assert_eq!(
        activatable
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>(),
        ["org.freedesktop.DBus"]
    );
}
