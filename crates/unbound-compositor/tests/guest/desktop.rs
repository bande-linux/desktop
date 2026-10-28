use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use unbound_compositor::BACKGROUND;

use crate::card::{Scanout, card_with_driver};
use crate::uinput::{
    KEY_ENTER, KEY_ESC, KEY_F, KEY_F10, KEY_LEFTCTRL, KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_M, KEY_Q,
    KEY_SPACE, VirtualDevice,
};

const COMPOSITOR: &str = "/usr/bin/unbound-compositor";
const RUNTIME_DIR: &str = "/run/user/0";
const TYPED: &str = "/tmp/typed-in-unbound-term";
const STARTUP: Duration = Duration::from_secs(60);

struct Desktop {
    compositor: Child,
    display: String,
}

impl Drop for Desktop {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.compositor.id().to_string()])
            .status();
        let _ = self.compositor.wait();
    }
}

fn current_scanout() -> Option<Scanout> {
    card_with_driver(&std::env::var("UNBOUND_TEST_DRIVER").unwrap_or_else(|_| "vkms".to_owned()))
        .current_scanout()
}

fn scanout() -> Scanout {
    current_scanout().expect("a framebuffer on the CRTC")
}

fn pixel(scanout: &Scanout, x: usize, y: usize) -> [u8; 3] {
    scanout.rgb[y * scanout.width + x]
}

fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        if started.elapsed() >= timeout {
            if let Some(scanout) = current_scanout() {
                scanout.save(&format!("timed-out-{}", what.replace(' ', "-")));
            }
            panic!("timed out waiting for {what}");
        }
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
    let display = line
        .trim()
        .strip_prefix("WAYLAND_DISPLAY=")
        .unwrap_or_else(|| panic!("{line}"))
        .to_owned();
    Desktop {
        compositor: child,
        display,
    }
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn the_launcher_opens_a_terminal_that_takes_typing() {
    let _ = fs::remove_file(TYPED);
    let mut keyboard = VirtualDevice::keyboard_and_mouse();
    let _desktop = start_desktop();

    wait_for("the panel", STARTUP, || {
        running("unbound-panel")
            && current_scanout().is_some_and(|scanout| pixel(&scanout, 32, 200) != BACKGROUND)
    });
    thread::sleep(Duration::from_secs(1));
    let desktop = scanout();
    desktop.save("m14-desktop-panel");
    let (center_x, card_y) = (desktop.width / 2, desktop.height * 24 / 100 + 20);
    let before = pixel(&desktop, center_x, card_y);

    keyboard.chord(&[KEY_LEFTMETA], KEY_SPACE);
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

    let terminal_color = pixel(&scanout(), center_x, desktop.height / 2);
    let corners = |scanout: &Scanout| {
        [
            pixel(scanout, scanout.width - 8, 8),
            pixel(scanout, scanout.width - 8, scanout.height - 8),
        ]
    };
    let background = corners(&desktop);
    keyboard.chord(&[KEY_LEFTCTRL, KEY_LEFTMETA], KEY_F);
    wait_for("the terminal to fill the screen", STARTUP, || {
        corners(&scanout()) == [terminal_color; 2]
    });
    thread::sleep(Duration::from_secs(1));
    scanout().save("m15-terminal-full-screen");

    keyboard.chord(&[KEY_LEFTCTRL, KEY_LEFTMETA], KEY_F);
    wait_for("the terminal to return to its window", STARTUP, || {
        corners(&scanout()) == background
    });
    thread::sleep(Duration::from_secs(1));
    scanout().save("m15-terminal-windowed");

    keyboard.chord(&[KEY_LEFTMETA], KEY_M);
    wait_for("the terminal to minimize", STARTUP, || {
        let now = scanout();
        pixel(&now, now.width / 2, now.height / 2) == pixel(&desktop, center_x, desktop.height / 2)
    });
    scanout().save("m15-terminal-minimized");
}

struct Firefox(Child);

impl Drop for Firefox {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

const PROFILE: &str = "/tmp/firefox-profile";
const PAGE: &str = "/tmp/unbound-page.html";
const FIRST_RUN_OFF: &str = r#"user_pref("browser.aboutwelcome.enabled", false);
user_pref("browser.startup.homepage_override.mstone", "ignore");
user_pref("datareporting.policy.dataSubmissionPolicyBypassNotification", true);
user_pref("toolkit.telemetry.reportingpolicy.firstRun", false);
"#;
const FIREFOX_STARTUP: Duration = Duration::from_secs(240);
const PAGE_COLOR: [u8; 3] = [0xFF, 0x00, 0xFF];

fn page_pixels(scanout: &Scanout) -> usize {
    scanout.rgb.iter().filter(|rgb| **rgb == PAGE_COLOR).count()
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn firefox_runs_on_wayland_and_opens_a_context_menu() {
    let mut keyboard = VirtualDevice::keyboard_and_mouse();
    let desktop = start_desktop();
    wait_for("the panel", STARTUP, || running("unbound-panel"));
    fs::create_dir_all(PROFILE).expect("profile directory");
    fs::write(format!("{PROFILE}/user.js"), FIRST_RUN_OFF).expect("profile preferences");
    fs::write(PAGE, "<body style='margin:0;background:#ff00ff'></body>").expect("test page");
    let _firefox = Firefox(
        Command::new("firefox-esr")
            .args([
                "--new-instance",
                "--profile",
                PROFILE,
                &format!("file://{PAGE}"),
            ])
            .env("XDG_RUNTIME_DIR", RUNTIME_DIR)
            .env("WAYLAND_DISPLAY", &desktop.display)
            .env("MOZ_ENABLE_WAYLAND", "1")
            .env_remove("DISPLAY")
            .env("HOME", "/root")
            .spawn()
            .expect("start firefox-esr"),
    );

    wait_for("Firefox to draw the page", FIREFOX_STARTUP, || {
        current_scanout().is_some_and(|scanout| page_pixels(&scanout) > 10_000)
    });
    thread::sleep(Duration::from_secs(2));
    let page = scanout();
    page.save("m16-firefox-page");

    keyboard.chord(&[KEY_LEFTSHIFT], KEY_F10);
    wait_for("the context menu", STARTUP, || {
        page_pixels(&scanout()) < page_pixels(&page)
    });
    thread::sleep(Duration::from_secs(1));
    scanout().save("m16-firefox-context-menu");

    keyboard.tap(KEY_ESC);
    wait_for("the context menu to close", STARTUP, || {
        page_pixels(&scanout()) == page_pixels(&page)
    });

    keyboard.chord(&[KEY_LEFTCTRL, KEY_LEFTMETA], KEY_F);
    wait_for("Firefox to go full screen", FIREFOX_STARTUP, || {
        page_pixels(&scanout()) > page_pixels(&page)
    });
    thread::sleep(Duration::from_secs(1));
    scanout().save("m16-firefox-full-screen");
}

const GUEST_PASSWORD: &str = "unbound-test";

#[test]
#[ignore = "needs the ci/vm guest"]
fn ctrl_super_q_locks_and_the_password_unlocks() {
    let mut keyboard = VirtualDevice::keyboard_and_mouse();
    let _desktop = start_desktop();
    let icon = (32, 33);
    wait_for("the panel", STARTUP, || {
        running("unbound-panel")
            && current_scanout().is_some_and(|scanout| pixel(&scanout, icon.0, 200) != BACKGROUND)
    });
    thread::sleep(Duration::from_secs(1));
    let unlocked = scanout();

    keyboard.chord(&[KEY_LEFTCTRL, KEY_LEFTMETA], KEY_Q);
    wait_for("the lock screen", STARTUP, || {
        running("unbound-lock") && pixel(&scanout(), icon.0, 200) == BACKGROUND
    });
    thread::sleep(Duration::from_secs(2));
    let locked = scanout();
    locked.save("m17-lock-screen");

    keyboard.type_text("wrong password\n");
    thread::sleep(Duration::from_secs(4));
    scanout().save("m17-lock-rejected");
    assert!(running("unbound-lock"));

    keyboard.type_text(GUEST_PASSWORD);
    keyboard.type_text("\n");
    wait_for("the desktop to come back", STARTUP, || {
        pixel(&scanout(), icon.0, 200) == pixel(&unlocked, icon.0, 200)
    });
    wait_for("the lock screen to exit", STARTUP, || {
        !running("unbound-lock")
    });
    thread::sleep(Duration::from_secs(1));
    scanout().save("m17-unlocked");
}
