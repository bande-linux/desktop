#![allow(clippy::expect_used, clippy::panic)]

use std::path::Path;

#[test]
fn a_test_reaches_neither_the_files_nor_the_session_of_the_person_running_it() {
    let root = std::env::var_os("BANDE_TEST_ROOT")
        .expect("tests run through ci/hermetic.sh, the runner .cargo/config.toml sets");
    for name in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "XDG_RUNTIME_DIR",
        "GNUPGHOME",
        "TMPDIR",
    ] {
        let value = std::env::var_os(name).unwrap_or_else(|| panic!("{name} is not set"));
        assert!(
            Path::new(&value).starts_with(&root),
            "{name} is outside {root:?}"
        );
    }
    for name in [
        "WAYLAND_DISPLAY",
        "WAYLAND_SOCKET",
        "DISPLAY",
        "AT_SPI_BUS_ADDRESS",
        "SSH_AUTH_SOCK",
    ] {
        assert!(std::env::var_os(name).is_none(), "{name} is set");
    }
    let session = zbus::blocking::Connection::session().expect("a private session bus");
    let reply = session
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "ListActivatableNames",
            &(),
        )
        .expect("ListActivatableNames");
    let names: Vec<String> = reply.body().deserialize().expect("names");
    assert_eq!(
        names,
        ["org.freedesktop.DBus"],
        "the session bus is the test bus"
    );
    assert!(
        zbus::blocking::Connection::system().is_err(),
        "the system bus is out of reach"
    );
}
