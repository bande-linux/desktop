#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{Awaited, Compositor, UNRESPONSIVE_AFTER};
use bande_test_bus::SessionBus;
use bande_test_wayland::keys::KEY_A;
use bande_test_wayland::{chord, headless};

fn serve_for(server: &mut Compositor, time: Duration) {
    let until = Instant::now() + time;
    while Instant::now() < until {
        server.dispatch().expect("dispatch");
        server.flush_clients().expect("flush");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn only_a_frozen_login_window_leaves_pings_unanswered() {
    let bus = SessionBus::start().expect("session bus");
    let home = tempfile::tempdir().expect("home");
    // SAFETY: this binary's only test sets its environment before it starts anything that reads it.
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
        std::env::set_var("DBUS_SYSTEM_BUS_ADDRESS", "unix:path=/nonexistent");
        std::env::set_var("BANDE_LOGIN_SOCK", "/nonexistent");
        std::env::set_var("HOME", home.path());
    }
    let mut server = headless(1280, 800);
    server.serve_login_window();
    server.hold_frames_until(Awaited::LoginWindow, Duration::ZERO, Duration::MAX);
    let mut greeter = server
        .spawn_shell_client(Path::new(env!("CARGO_BIN_EXE_bande-greeter")))
        .expect("the greeter");
    let deadline = Instant::now() + Duration::from_secs(60);
    while server.awaited_has_not_appeared(Duration::MAX) && Instant::now() < deadline {
        serve_for(&mut server, Duration::from_millis(5));
    }
    serve_for(&mut server, Duration::from_millis(500));
    let pid = greeter.id();
    let press = |server: &mut Compositor, at: Duration| {
        chord(server, &[KEY_A]);
        server.watchdog_tick(at);
        serve_for(server, Duration::from_secs(1));
        server.watchdog_tick(at + UNRESPONSIVE_AFTER);
        server.is_unresponsive(pid)
    };
    let responsive = !press(&mut server, Duration::from_secs(10));
    let stopped = Command::new("kill")
        .args(["-STOP", &pid.to_string()])
        .status()
        .expect("stop the greeter");
    let frozen = press(&mut server, Duration::from_secs(20));
    greeter.kill().expect("end the greeter");
    greeter.wait().expect("the greeter ends");
    assert!(stopped.success());
    assert!(responsive, "a living login window counts as frozen");
    assert!(frozen, "a frozen login window is not noticed");
}
