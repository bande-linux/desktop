#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use bande_compositor::Compositor;
use bande_test_bus::SessionBus;
use bande_test_wayland::keys::{KEY_F4, KEY_LEFTALT, KEY_SPACE, KEY_TAB};
use bande_test_wayland::{Frames, chord, headless};
use smithay::utils::Point;

const SETTLE: u64 = 300;

fn launch(bus: &SessionBus, state: &Path, arguments: &[&Path], display: UnixStream) -> Child {
    Command::new(env!("CARGO_BIN_EXE_bande-installer"))
        .args(arguments)
        .env("WAYLAND_SOCKET", "0")
        .env("DBUS_SESSION_BUS_ADDRESS", bus.address())
        .env("DBUS_SYSTEM_BUS_ADDRESS", bus.address())
        .env("XDG_STATE_HOME", state)
        .stdin(Stdio::from(OwnedFd::from(display)))
        .spawn()
        .expect("start Package Installer")
}

fn shown(server: &Compositor) -> Option<(f64, f64)> {
    let rect = server
        .windows()
        .iter()
        .filter_map(|w| w.geometry())
        .find(|g| g.size.w > 300)?;
    Some((
        f64::from(rect.loc.x + rect.size.w / 2),
        f64::from(rect.loc.y + rect.size.h / 2),
    ))
}

fn pass(frames: &mut Frames, server: &mut Compositor, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    frames.run_until(server, || Instant::now() > until);
}

fn until_shown(frames: &mut Frames, server: &mut Compositor, wanted: bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while shown(server).is_some() != wanted {
        assert!(Instant::now() < deadline, "shown is never {wanted}");
        pass(frames, server, 10);
    }
}

fn reopen(bus: &SessionBus, state: &Path, frames: &mut Frames, server: &mut Compositor) {
    let (unused, _) = UnixStream::pair().expect("socket pair");
    let mut again = launch(bus, state, &[], unused);
    frames.run_until(server, || again.try_wait().expect("state").is_some());
    until_shown(frames, server, true);
}

#[test]
fn a_package_closed_by_its_window_comes_back_and_stays_and_one_dismissed_is_done_with() {
    let dir = tempfile::tempdir().expect("directory");
    let deb = dir.path().join("partial.deb");
    std::fs::write(&deb, "not a package").expect("a file");
    let bus = SessionBus::start().expect("private bus");
    let mut server = headless(1280, 800);
    let mut frames = Frames::start();
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let mut app = launch(&bus, dir.path(), &[&deb], client_end);
    until_shown(&mut frames, &mut server, true);
    pass(&mut frames, &mut server, SETTLE);

    chord(&mut server, &[KEY_LEFTALT, KEY_F4]);
    until_shown(&mut frames, &mut server, false);
    reopen(&bus, dir.path(), &mut frames, &mut server);
    let (x, y) = shown(&server).expect("the window");
    for at in [(5.0, 5.0), (x, y), (x + 9.0, y), (5.0, 5.0), (x, y)] {
        server.move_pointer_to(Point::from(at), 0);
        pass(&mut frames, &mut server, 100);
    }
    assert!(shown(&server).is_some(), "hovered away");

    chord(&mut server, &[KEY_TAB]);
    pass(&mut frames, &mut server, SETTLE);
    chord(&mut server, &[KEY_SPACE]);
    let deadline = Instant::now() + Duration::from_secs(30);
    let ended = loop {
        pass(&mut frames, &mut server, 10);
        let state = app.try_wait().expect("Package Installer's state");
        if state.is_some() || Instant::now() > deadline {
            break state;
        }
    };
    if ended.is_none() {
        app.kill().expect("stop Package Installer");
        app.wait().expect("wait for Package Installer");
    }
    assert!(
        ended.is_some_and(|status| status.success()),
        "still running with a finished package"
    );
}
