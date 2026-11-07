#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use bande_compositor::Compositor;
use bande_protocols::client::bande_shell_v1::SessionEnd;
use bande_test_bus::SessionBus;
use bande_test_wayland::{Frames, TestClient, act_as_separate_app, headless};

const DIALOG: &str = "org.bande.MenuBar";
const PATIENCE: Duration = Duration::from_secs(40);

fn dialog_shown(server: &Compositor) -> bool {
    server
        .windows()
        .iter()
        .any(|window| window.app_id() == DIALOG && window.size().is_some())
}

#[test]
fn a_helper_replaced_on_disk_still_starts_as_the_build_the_session_began_with() {
    if act_as_separate_app() {
        return;
    }
    let state = tempfile::tempdir().expect("state dir");
    let bus = SessionBus::start().expect("private bus");
    unsafe {
        std::env::set_var("DBUS_SYSTEM_BUS_ADDRESS", bus.address());
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
        std::env::set_var("XDG_STATE_HOME", state.path());
    }
    let installed = state.path().join("bande-shell");
    fs::copy(env!("CARGO_BIN_EXE_bande-shell"), &installed).unwrap();

    let mut server = headless(900, 900);
    server.set_session_dialog(&installed);

    let newer = state.path().join("bande-shell.dpkg-new");
    fs::write(&newer, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&newer, fs::Permissions::from_mode(0o755)).unwrap();
    fs::rename(&newer, &installed).unwrap();

    let mut shell = TestClient::connect_as_shell(&mut server);
    shell.listen_as_shell(&mut server);
    shell.confirm_session_end(&mut server, SessionEnd::LogOut);
    let mut frames = Frames::start();
    let deadline = Instant::now() + PATIENCE;
    while !dialog_shown(&server) {
        assert!(
            Instant::now() < deadline,
            "the session's own build never opened"
        );
        server.keep_session_clients_running();
        let mut first = true;
        frames.run_until(&mut server, || !std::mem::take(&mut first));
    }
}
