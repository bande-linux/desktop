#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{Awaited, SessionRole};
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
    server.serve_login_window();
    server.hold_frames_until(Awaited::LoginWindow, Duration::ZERO, Duration::MAX);
    assert!(server.awaited_never_appeared(Duration::MAX));
    let (compositor_end, greeter_end) = UnixStream::pair().expect("socket pair");
    server
        .insert_session_client(compositor_end, SessionRole::Shell)
        .expect("the greeter's connection");
    let home = tempfile::tempdir().expect("home");
    let mut greeter = Command::new(env!("CARGO_BIN_EXE_bande-greeter"))
        .stdin(OwnedFd::from(greeter_end))
        .env("WAYLAND_SOCKET", "0")
        .env("DBUS_SESSION_BUS_ADDRESS", bus.address())
        .env("DBUS_SYSTEM_BUS_ADDRESS", "unix:path=/nonexistent")
        .env("BANDE_LOGIN_SOCK", "/nonexistent")
        .env("HOME", home.path())
        .spawn()
        .expect("the greeter");

    let deadline = Instant::now() + PATIENCE;
    while !server.awaited_drawable() && Instant::now() < deadline {
        server.dispatch().expect("dispatch");
        server.flush_clients().expect("flush");
        server.import_awaited(&mut renderer);
        thread::sleep(Duration::from_millis(5));
    }
    let drawn = server.awaited_drawable();
    let appeared = !server.awaited_never_appeared(Duration::MAX);
    greeter.kill().expect("the greeter runs until it is ended");
    greeter.wait().expect("the greeter ends");

    assert!(drawn, "the login window drew nothing");
    assert!(appeared, "a login window that is there is not given up");
    assert_eq!(questions.try_iter().next(), None, "the portal was asked");
}
