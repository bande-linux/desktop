#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{Awaited, Compositor, OutputConfig, SessionRole};
use bande_render::VulkanRenderer;
use bande_test_bus::SessionBus;
use bande_test_wayland::headless;
use zbus::blocking::MessageIterator;

const PATIENCE: Duration = Duration::from_secs(60);

#[test]
fn the_login_window_is_ready_before_a_display_is_taken_and_starts_no_portal() {
    let bus = SessionBus::start().expect("session bus");
    let portal = bus.connect().expect("connection");
    portal
        .request_name("org.freedesktop.portal.Desktop")
        .expect("the portal's name");
    let (asked, questions) = mpsc::channel();
    let messages = MessageIterator::from(&portal);
    thread::spawn(move || {
        for message in messages.flatten() {
            if message.message_type() == zbus::message::Type::MethodCall {
                let _ = asked.send(message.header().member().map(ToString::to_string));
            }
        }
    });

    let mut server = headless(1280, 800);
    let mut renderer = VulkanRenderer::new(None).expect("Vulkan renderer");
    server.hold_frames_until(Awaited::LoginWindow, Duration::ZERO, Duration::MAX);
    assert!(server.awaited_has_not_appeared(Duration::MAX));
    let home = tempfile::tempdir().expect("home");
    let mut greeter = login_window_on(&mut server, &bus, home.path());

    let deadline = Instant::now() + PATIENCE;
    while !server.awaited_drawable() && Instant::now() < deadline {
        server.dispatch().expect("dispatch");
        server.flush_clients().expect("flush");
        server.import_awaited(&mut renderer);
        thread::sleep(Duration::from_millis(5));
    }
    let drawn = server.awaited_drawable();
    let appeared = !server.awaited_has_not_appeared(Duration::MAX);
    greeter.kill().expect("the greeter runs until it is ended");
    greeter.wait().expect("the greeter ends");

    assert!(drawn, "the login window drew nothing");
    assert!(appeared, "a login window that is there is not given up");
    assert_eq!(questions.try_iter().next(), None, "the portal was asked");
}

fn login_window_on(server: &mut Compositor, bus: &SessionBus, home: &Path) -> Child {
    server.serve_login_window();
    let (compositor_end, greeter_end) = UnixStream::pair().expect("socket pair");
    server
        .insert_session_client(compositor_end, SessionRole::Shell)
        .expect("the greeter's connection");
    Command::new(env!("CARGO_BIN_EXE_bande-greeter"))
        .stdin(OwnedFd::from(greeter_end))
        .env("WAYLAND_SOCKET", "0")
        .env("DBUS_SESSION_BUS_ADDRESS", bus.address())
        .env("DBUS_SYSTEM_BUS_ADDRESS", "unix:path=/nonexistent")
        .env("BANDE_LOGIN_SOCK", "/nonexistent")
        .env("HOME", home)
        .spawn()
        .expect("the greeter")
}

fn appears(server: &mut Compositor) -> bool {
    server.hold_frames_until(Awaited::LoginWindow, Duration::ZERO, Duration::MAX);
    let deadline = Instant::now() + PATIENCE;
    while server.awaited_has_not_appeared(Duration::MAX) && Instant::now() < deadline {
        serve_for(server, Duration::from_millis(5));
    }
    !server.awaited_has_not_appeared(Duration::MAX)
}

fn serve_for(server: &mut Compositor, time: Duration) {
    let until = Instant::now() + time;
    while Instant::now() < until {
        server.dispatch().expect("dispatch");
        server.flush_clients().expect("flush");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn the_login_window_moves_to_another_display_when_its_own_goes_away() {
    let bus = SessionBus::start().expect("session bus");
    let home = tempfile::tempdir().expect("home");
    let mut server = headless(1280, 800);
    server.add_output(OutputConfig::virtual_display(
        "HEADLESS-2",
        (1280, 800).into(),
    ));
    let mut greeter = login_window_on(&mut server, &bus, home.path());
    assert!(appears(&mut server));
    let first = server.main_output();
    server.remove_output(&first);
    assert_ne!(server.main_output(), first);
    let moved = appears(&mut server);
    serve_for(&mut server, Duration::from_millis(500));
    let ended = greeter.try_wait().expect("the greeter's status");
    greeter.kill().expect("end the greeter");
    greeter.wait().expect("the greeter ends");
    assert!(moved, "the login window is not on the other display");
    assert_eq!(ended, None, "the login window ended with its display");
}
