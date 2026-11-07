#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::os::unix::net::UnixListener;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use bande_files::{SIDEBAR_WIDTH, TOOLBAR_HEIGHT};
use bande_test_bus::SessionBus;
use bande_test_wayland::{Compositor, Frames, chord, headless};

const BTN_LEFT: u32 = 0x110;
const KEY_LEFTCTRL: u32 = 29;
const KEY_T: u32 = 20;

struct Desk {
    server: Compositor,
    frames: Frames,
    listener: UnixListener,
    files: Child,
    time: u32,
}

impl Desk {
    fn run_until(&mut self, what: &str, mut done: impl FnMut(&mut Compositor) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(&mut self.server) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            if let Ok((stream, _)) = self.listener.accept() {
                self.server.insert_client(stream).expect("insert client");
            }
            let step = Instant::now() + Duration::from_millis(20);
            self.frames
                .run_until(&mut self.server, || Instant::now() >= step);
        }
    }

    fn settle(&mut self, millis: u64) {
        let until = Instant::now() + Duration::from_millis(millis);
        self.run_until("settling", |_| Instant::now() >= until);
    }

    fn windows(&mut self, count: usize) {
        self.run_until(&format!("{count} windows"), |server| {
            server
                .windows()
                .iter()
                .filter(|window| window.geometry().is_some())
                .count()
                == count
        });
    }

    fn pointer(&mut self, at: (f64, f64)) {
        self.time += 20;
        self.server.move_pointer_to(at.into(), self.time);
        self.settle(100);
    }

    fn button(&mut self, pressed: bool) {
        self.time += 20;
        self.server.press_button(BTN_LEFT, pressed, self.time);
        self.settle(200);
    }

    fn first_tab(&self) -> (f64, f64) {
        let window = self.server.windows()[0].geometry().expect("mapped").loc;
        (
            f64::from(window.x) + f64::from(SIDEBAR_WIDTH) + 60.0,
            f64::from(window.y) + f64::from(TOOLBAR_HEIGHT) + 18.0,
        )
    }

    fn tear_off(&mut self) -> (f64, f64) {
        let tab = self.first_tab();
        self.pointer(tab);
        self.button(true);
        self.pointer((tab.0 + 10.0, tab.1));
        self.pointer((tab.0 + 10.0, tab.1 + 200.0));
        self.windows(2);
        tab
    }
}

impl Drop for Desk {
    fn drop(&mut self) {
        let _ = self.files.kill();
        let _ = self.files.wait();
    }
}

#[test]
fn a_tab_torn_off_floats_in_its_own_window_and_comes_back_over_the_toolbar() {
    let home = tempfile::tempdir().unwrap();
    for folder in [".config", ".cache", ".local/share", ".local/state", "run"] {
        fs::create_dir_all(home.path().join(folder)).unwrap();
    }
    let socket = home.path().join("wayland");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let bus = SessionBus::start().unwrap();
    let files = Command::new(env!("CARGO_BIN_EXE_bande-files"))
        .arg(home.path())
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .env("XDG_CACHE_HOME", home.path().join(".cache"))
        .env("XDG_DATA_HOME", home.path().join(".local/share"))
        .env("XDG_STATE_HOME", home.path().join(".local/state"))
        .env("XDG_RUNTIME_DIR", home.path().join("run"))
        .env("WAYLAND_DISPLAY", &socket)
        .env("DBUS_SESSION_BUS_ADDRESS", bus.address())
        .spawn()
        .unwrap();
    let mut desk = Desk {
        server: headless(1800, 1200),
        frames: Frames::start(),
        listener,
        files,
        time: 1000,
    };
    desk.windows(1);
    desk.settle(500);
    chord(&mut desk.server, &[KEY_LEFTCTRL, KEY_T]);
    desk.settle(500);

    let tab = desk.tear_off();
    desk.pointer((tab.0, tab.1 - 20.0));
    desk.windows(1);
    desk.pointer((tab.0, tab.1 + 200.0));
    desk.windows(2);
    desk.button(false);
    desk.settle(300);
    desk.windows(2);
}
