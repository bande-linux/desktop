use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use unbound_compositor::BACKGROUND;

use crate::card::{Scanout, card_with_driver};
use crate::uinput::{KEY_ENTER, KEY_LEFTMETA, KEY_SPACE, VirtualDevice};

const COMPOSITOR: &str = "/usr/bin/unbound-compositor";
const RUNTIME_DIR: &str = "/run/user/0";
const TYPED: &str = "/tmp/typed-in-unbound-term";
const STARTUP: Duration = Duration::from_secs(60);

struct Desktop(Child);

impl Drop for Desktop {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.0.id().to_string()])
            .status();
        let _ = self.0.wait();
    }
}

fn scanout() -> Scanout {
    card_with_driver(&std::env::var("UNBOUND_TEST_DRIVER").unwrap_or_else(|_| "vkms".to_owned()))
        .scanout()
}

fn pixel(scanout: &Scanout, x: usize, y: usize) -> [u8; 3] {
    scanout.rgb[y * scanout.width + x]
}

fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < timeout, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(100));
    }
}

fn running(name: &str) -> bool {
    fs::read_dir("/proc").is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            fs::read_to_string(entry.path().join("comm")).is_ok_and(|comm| comm.trim() == name)
        })
    })
}

fn start_desktop() -> Desktop {
    fs::create_dir_all(RUNTIME_DIR).expect("runtime directory");
    fs::set_permissions(RUNTIME_DIR, fs::Permissions::from_mode(0o700)).expect("permissions");
    let mut child = Command::new(COMPOSITOR)
        .env("XDG_RUNTIME_DIR", RUNTIME_DIR)
        .env("XDG_CURRENT_DESKTOP", "Unbound")
        .env("HOME", "/root")
        .env("SHELL", "/bin/bash")
        .stdout(Stdio::piped())
        .spawn()
        .expect("start unbound-compositor");
    let stdout = child.stdout.take().expect("compositor stdout");
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("the compositor's socket line");
    assert!(line.starts_with("WAYLAND_DISPLAY="), "{line}");
    Desktop(child)
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn the_launcher_opens_a_terminal_that_takes_typing() {
    let _ = fs::remove_file(TYPED);
    let mut keyboard = VirtualDevice::keyboard_and_mouse();
    let _desktop = start_desktop();

    wait_for("the panel", STARTUP, || {
        running("unbound-panel") && pixel(&scanout(), 32, 200) != BACKGROUND
    });
    thread::sleep(Duration::from_secs(1));
    let desktop = scanout();
    desktop.save("m14-desktop-panel");
    let (center_x, card_y) = (desktop.width / 2, desktop.height * 24 / 100 + 20);
    let before = pixel(&desktop, center_x, card_y);

    keyboard.chord(KEY_LEFTMETA, KEY_SPACE);
    wait_for("the launcher", STARTUP, || {
        pixel(&scanout(), center_x, card_y) != before
    });
    keyboard.type_text("term");
    thread::sleep(Duration::from_secs(1));
    scanout().save("m14-launcher-results");

    keyboard.tap(KEY_ENTER);
    wait_for("the terminal", STARTUP, || running("unbound-term"));
    wait_for("the launcher to close", STARTUP, || {
        let now = scanout();
        pixel(&now, center_x, now.height / 2) == [0xFF, 0xFF, 0xFF]
    });
    thread::sleep(Duration::from_secs(2));

    keyboard.type_text("echo typed from the keyboard > /tmp/typed-in-unbound-term\n");
    wait_for("the shell to run the typed command", STARTUP, || {
        fs::read_to_string(TYPED).is_ok_and(|text| text.trim() == "typed from the keyboard")
    });
    keyboard.type_text("ls /usr/share/applications\n");
    thread::sleep(Duration::from_secs(1));
    scanout().save("m14-terminal-typed");
}
