#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use bande_compositor::Compositor;
use bande_test_wayland::{RED, SessionEnd, ShmImage, TestClient};
use smithay::utils::Size;
use wayland_client::protocol::wl_shm::Format;

const EDITOR: &str = "org.example.Editor";

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn listed(file: &Path) -> String {
    fs::read_to_string(file).unwrap_or_default()
}

fn released(mut peer: UnixStream) -> bool {
    peer.set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    peer.read(&mut [0]).is_ok_and(|read| read == 0)
}

#[test]
fn every_session_ending_remembers_the_open_apps_once() {
    let data = tempfile::tempdir().expect("data dir");
    let applications = data.path().join("applications");
    fs::create_dir_all(&applications).expect("applications");
    fs::write(
        applications.join(format!("{EDITOR}.desktop")),
        "[Desktop Entry]\nType=Application\nName=Editor\nExec=editor\n",
    )
    .expect("desktop entry");
    unsafe {
        std::env::set_var("XDG_DATA_HOME", data.path());
        std::env::set_var("XDG_DATA_DIRS", data.path());
    }
    let state = tempfile::tempdir().expect("state dir");
    let file = state.path().join("bande/reopen");
    let mut server = Compositor::new(Size::from((256, 192))).expect("headless compositor");
    server.remember_apps_to_reopen_in(file.clone());
    let mut editor = TestClient::connect(&mut server);
    let pixels = RED.repeat(32 * 32);
    let image = ShmImage {
        width: 32,
        height: 32,
        stride: 32 * 4,
        format: Format::Xrgb8888,
        bytes: &pixels,
    };
    editor.show_window_of(&mut server, &image, EDITOR);
    let mut bar = TestClient::connect_as_shell(&mut server);
    bar.join_shell(&mut server);

    bar.confirm_session_end(&mut server, SessionEnd::LogOut);
    wait_for("the apps of the log out", || {
        listed(&file) == "org.example.Editor\n"
    });

    fs::remove_file(&file).expect("forget");
    let (delay, peer) = UnixStream::pair().expect("pair");
    server.shutting_down(OwnedFd::from(delay));
    assert!(released(peer), "the shutdown waited for nothing");
    assert_eq!(listed(&file), "org.example.Editor\n");

    fs::write(&file, "org.example.Viewer\n").expect("an earlier record");
    let (delay, peer) = UnixStream::pair().expect("pair");
    server.shutting_down(OwnedFd::from(delay));
    assert!(released(peer));
    assert_eq!(listed(&file), "org.example.Viewer\n");
}
