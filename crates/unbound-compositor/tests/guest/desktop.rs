use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use unbound_compositor::BACKGROUND;
use unbound_test_bus::SessionBus;

use crate::card::{Scanout, card_with_driver};
use crate::uinput::{
    BTN_LEFT, EV_KEY, EV_REL, KEY_3, KEY_4, KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_F, KEY_F10,
    KEY_LEFTCTRL, KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_M, KEY_Q, KEY_RIGHT, KEY_SPACE, REL_X, REL_Y,
    VirtualDevice,
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

const TASK_COMM_LENGTH: usize = 15;

fn running(name: &str) -> bool {
    let comm = &name[..name.len().min(TASK_COMM_LENGTH)];
    fs::read_dir("/proc").is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            fs::read_to_string(entry.path().join("comm")).is_ok_and(|read| read.trim() == comm)
        })
    })
}

fn start_desktop() -> Desktop {
    start_desktop_with(&[])
}

fn start_desktop_with(environment: &[(&str, &str)]) -> Desktop {
    start_desktop_with_arguments(&[], environment)
}

fn start_desktop_with_arguments(arguments: &[&str], environment: &[(&str, &str)]) -> Desktop {
    fs::create_dir_all(RUNTIME_DIR).expect("runtime directory");
    fs::set_permissions(RUNTIME_DIR, fs::Permissions::from_mode(0o700)).expect("permissions");
    let mut child = Command::new(COMPOSITOR)
        .args(arguments)
        .env("XDG_RUNTIME_DIR", RUNTIME_DIR)
        .env("XDG_CURRENT_DESKTOP", "Unbound")
        .env("HOME", "/root")
        .env("SHELL", "/bin/bash")
        .envs(environment.iter().copied())
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

#[derive(Clone, Default)]
struct LockedHint(Arc<Mutex<Option<bool>>>);

impl LockedHint {
    fn get(&self) -> Option<bool> {
        *self.0.lock().expect("locked hint")
    }
}

struct FakeSession {
    hint: LockedHint,
}

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl FakeSession {
    fn set_locked_hint(&self, locked: bool) {
        *self.hint.0.lock().expect("locked hint") = Some(locked);
    }
}

fn fake_logind(bus: &SessionBus) -> (zbus::blocking::Connection, LockedHint) {
    let hint = LockedHint::default();
    let logind = bus.connect().expect("logind stand-in connection");
    logind
        .object_server()
        .at(
            "/org/freedesktop/login1/session/auto",
            FakeSession { hint: hint.clone() },
        )
        .expect("serve the session");
    logind
        .request_name("org.freedesktop.login1")
        .expect("own logind's name");
    (logind, hint)
}

const ROUNDING: u8 = 2;
const CARET_PIXELS: usize = 2 * 32;

fn differs(shown: [u8; 3], expected: [u8; 3]) -> bool {
    shown
        .iter()
        .zip(expected)
        .any(|(shown, expected)| shown.abs_diff(expected) > ROUNDING)
}

fn field_differs_from(scanout: &Scanout, reference: &Scanout) -> usize {
    let lower_half = scanout.width * scanout.height / 2;
    scanout.rgb[lower_half..]
        .iter()
        .zip(&reference.rgb[lower_half..])
        .filter(|&(&shown, &expected)| differs(shown, expected))
        .count()
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn ctrl_super_q_locks_and_the_password_unlocks() {
    let bus = SessionBus::start().expect("private bus standing in for the system bus");
    let (_logind, locked_hint) = fake_logind(&bus);
    let mut keyboard = VirtualDevice::keyboard_and_mouse();
    let _desktop = start_desktop_with(&[("DBUS_SYSTEM_BUS_ADDRESS", bus.address())]);
    let icon = (32, 33);
    wait_for("the panel", STARTUP, || {
        running("unbound-panel")
            && current_scanout().is_some_and(|scanout| pixel(&scanout, icon.0, 200) != BACKGROUND)
    });
    thread::sleep(Duration::from_secs(1));

    keyboard.chord(&[KEY_LEFTCTRL, KEY_LEFTMETA], KEY_Q);
    wait_for("the session to lock", STARTUP, || {
        locked_hint.get() == Some(true) && running("unbound-lock")
    });
    thread::sleep(Duration::from_secs(2));
    let locked = scanout();
    locked.save("m17-lock-screen");

    keyboard.type_text("wrong password");
    wait_for("the typed password", STARTUP, || {
        field_differs_from(&scanout(), &locked) > CARET_PIXELS
    });
    keyboard.tap(KEY_ENTER);
    wait_for("the empty field after the rejection", STARTUP, || {
        field_differs_from(&scanout(), &locked) <= CARET_PIXELS
    });
    scanout().save("m17-lock-rejected");
    assert_eq!(
        locked_hint.get(),
        Some(true),
        "a wrong password leaves the session locked"
    );
    assert!(
        running("unbound-lock"),
        "a wrong password leaves the lock screen up"
    );

    keyboard.type_text(GUEST_PASSWORD);
    keyboard.type_text("\n");
    wait_for("the session to unlock", STARTUP, || {
        locked_hint.get() == Some(false)
    });
    wait_for("the lock screen to exit", STARTUP, || {
        !running("unbound-lock")
    });
    thread::sleep(Duration::from_secs(1));
    scanout().save("m17-unlocked");
}

fn saved_screenshots(desktop: &Path) -> Vec<PathBuf> {
    fs::read_dir(desktop)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name.starts_with("Screenshot ") && name.ends_with(".png")
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn super_shift_3_saves_the_screen_to_the_desktop() {
    let desktop_folder = Path::new("/root/Desktop");
    let _ = fs::remove_dir_all(desktop_folder);
    let mut keyboard = VirtualDevice::keyboard_and_mouse();
    let _desktop = start_desktop();
    let icon = (32, 33);
    wait_for("the panel and the screenshot tool", STARTUP, || {
        running("unbound-panel")
            && running("unbound-screenshot")
            && current_scanout().is_some_and(|scanout| pixel(&scanout, icon.0, 200) != BACKGROUND)
    });
    thread::sleep(Duration::from_secs(1));
    let screen = scanout();

    keyboard.chord(&[KEY_LEFTMETA, KEY_LEFTSHIFT], KEY_3);
    wait_for("a saved screenshot", STARTUP, || {
        saved_screenshots(desktop_folder).len() == 1
    });

    let path = saved_screenshots(desktop_folder).remove(0);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    assert_eq!(
        name.len(),
        "Screenshot 2026-10-29 at 09.05.07.png".len(),
        "{name}"
    );
    assert_eq!(&name[21..25], " at ", "{name}");
    let saved = image::open(&path)
        .expect("the screenshot is a PNG")
        .to_rgb8();
    assert_eq!(
        saved.dimensions(),
        (screen.width as u32, screen.height as u32)
    );
    let shot = Scanout {
        width: screen.width,
        height: screen.height,
        rgb: saved.pixels().map(|pixel| pixel.0).collect(),
    };
    let after = scanout();
    shot.save("m18-screenshot");
    screen.save("m18-screen-before-capture");
    let differing = |shown: &Scanout| {
        shot.rgb
            .iter()
            .zip(&shown.rgb)
            .filter(|(saved, shown)| saved != shown)
            .count()
    };
    let differing = differing(&screen).min(differing(&after));
    let cursor = (unbound_compositor::ARROW_WIDTH * unbound_compositor::ARROW_HEIGHT) as usize;
    assert!(
        differing <= cursor,
        "{differing} pixels differ from the screen"
    );
}

fn crosshair_centre(overlaid: &Scanout, before: &Scanout) -> (usize, usize) {
    let (width, height) = (overlaid.width / 2, overlaid.height / 2);
    let changed = |x: usize, y: usize| differs(pixel(overlaid, x, y), pixel(before, x, y));
    let x = (0..width)
        .max_by_key(|&x| (0..height).filter(|&y| changed(x, y)).count())
        .unwrap_or_default();
    let y = (0..height)
        .max_by_key(|&y| (0..width).filter(|&x| changed(x, y)).count())
        .unwrap_or_default();
    (x, y)
}

fn outlined(selecting: &Scanout, edge: (usize, usize), inward: (isize, isize)) -> bool {
    let at = |step: isize| {
        let x = edge.0.checked_add_signed(inward.0 * step)?;
        let y = edge.1.checked_add_signed(inward.1 * step)?;
        Some(pixel(selecting, x, y))
    };
    let (inside, outside) = (at(4), at(-4));
    (-1..=1).filter_map(at).any(|line| {
        inside.is_some_and(|inside| differs(line, inside))
            && outside.is_none_or(|outside| differs(line, outside))
    })
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn super_shift_4_saves_a_dragged_out_area_to_the_desktop() {
    let desktop_folder = Path::new("/root/Desktop");
    let _ = fs::remove_dir_all(desktop_folder);
    let mut mouse = VirtualDevice::keyboard_and_mouse();
    let _desktop = start_desktop();
    let icon = (32, 33);
    wait_for("the panel and the screenshot tool", STARTUP, || {
        running("unbound-panel")
            && running("unbound-screenshot")
            && current_scanout().is_some_and(|scanout| pixel(&scanout, icon.0, 200) != BACKGROUND)
    });
    for _ in 0..20 {
        mouse.frame(&[(EV_REL, REL_X, -200), (EV_REL, REL_Y, -200)]);
    }
    thread::sleep(Duration::from_secs(1));
    let corner = |scanout: &Scanout| {
        (0..32)
            .flat_map(|y| (0..32).map(move |x| (x, y)))
            .map(|(x, y)| pixel(scanout, x, y))
            .collect::<Vec<_>>()
    };
    let before = scanout();

    mouse.chord(&[KEY_LEFTMETA, KEY_LEFTSHIFT], KEY_4);
    wait_for("the crosshair of the area overlay", STARTUP, || {
        corner(&scanout()) != corner(&before)
    });
    thread::sleep(Duration::from_millis(500));
    let pressed_at = crosshair_centre(&scanout(), &before);
    let drag = (8, 5);
    mouse.frame(&[(EV_KEY, BTN_LEFT, 1)]);
    for _ in 0..30 {
        mouse.frame(&[(EV_REL, REL_X, drag.0), (EV_REL, REL_Y, drag.1)]);
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_secs(1));
    let selecting = scanout();
    selecting.save("m19-area-selection");
    mouse.frame(&[(EV_KEY, BTN_LEFT, 0)]);
    wait_for("a saved area screenshot", STARTUP, || {
        saved_screenshots(desktop_folder).len() == 1
    });
    thread::sleep(Duration::from_secs(1));
    let screen = scanout();
    screen.save("m19-after-area-capture");

    let saved = image::open(saved_screenshots(desktop_folder).remove(0))
        .expect("the screenshot is a PNG")
        .to_rgb8();
    let (width, height) = (saved.width() as usize, saved.height() as usize);
    let shot = Scanout {
        width,
        height,
        rgb: saved.pixels().map(|pixel| pixel.0).collect(),
    };
    shot.save("m19-area-screenshot");
    assert!(width > 20 && width < screen.width, "width {width}");
    assert!(height > 20 && height < screen.height, "height {height}");
    assert!(
        (width as isize * drag.1 as isize - height as isize * drag.0 as isize).abs()
            <= 2 * drag.0 as isize,
        "{width} x {height} is not the shape of a {drag:?} drag"
    );

    let cursor = (unbound_compositor::ARROW_WIDTH * unbound_compositor::ARROW_HEIGHT) as usize;
    let differing_at = |(left, top): (usize, usize)| {
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .filter(|&(x, y)| shot.rgb[y * width + x] != pixel(&screen, left + x, top + y))
            .count()
    };
    let origin = (pressed_at.0.saturating_sub(1)..=pressed_at.0 + 1)
        .flat_map(|x| (pressed_at.1.saturating_sub(1)..=pressed_at.1 + 1).map(move |y| (x, y)))
        .min_by_key(|&origin| differing_at(origin))
        .unwrap_or(pressed_at);
    let differing = differing_at(origin);
    assert!(
        differing <= cursor,
        "{differing} pixels differ from the screen at {origin:?}, where the drag began at {pressed_at:?}"
    );

    let (left, top) = origin;
    let (right, bottom) = (left + width, top + height);
    let (middle_x, middle_y) = (left + width / 2, top + height / 2);
    for (edge, inward, name) in [
        ((middle_x, top), (0, 1), "top"),
        ((middle_x, bottom), (0, -1), "bottom"),
        ((left, middle_y), (1, 0), "left"),
        ((right, middle_y), (-1, 0), "right"),
    ] {
        assert!(
            outlined(&selecting, edge, inward),
            "no outline on the {name} edge of the selection at {edge:?}"
        );
    }
    let centre = (middle_x, middle_y);
    assert!(
        differs(
            pixel(&selecting, centre.0, centre.1),
            pixel(&before, centre.0, centre.1)
        ),
        "the inside of the selection is drawn over the screen"
    );
    let beside = (right + 40, middle_y);
    assert!(
        !differs(
            pixel(&selecting, beside.0, beside.1),
            pixel(&before, beside.0, beside.1)
        ),
        "the screen outside the selection is left as it was"
    );
}

const FILES: &str = "/usr/bin/unbound-files";

#[test]
#[ignore = "needs the ci/vm guest"]
fn files_drills_into_a_folder_and_quick_looks_a_picture() {
    let home = Path::new("/root/m22");
    let _ = fs::remove_dir_all(home);
    fs::create_dir_all(home.join("Pictures")).expect("folders");
    fs::write(home.join("notes.txt"), "notes\n").expect("file");
    let magenta = [0xFF, 0x00, 0xFF];
    let (picture_width, picture_height) = (200, 150);
    image::RgbImage::from_pixel(picture_width, picture_height, image::Rgb(magenta))
        .save(home.join("Pictures/photo.png"))
        .expect("png");
    let mut keyboard = VirtualDevice::keyboard_and_mouse();
    let desktop = start_desktop();
    wait_for("the panel", STARTUP, || {
        running("unbound-panel")
            && current_scanout().is_some_and(|scanout| pixel(&scanout, 32, 200) != BACKGROUND)
    });
    let without_files = scanout();
    let mut files = Command::new(FILES)
        .arg(home)
        .env("WAYLAND_DISPLAY", &desktop.display)
        .env("XDG_RUNTIME_DIR", RUNTIME_DIR)
        .env("HOME", "/root")
        .spawn()
        .expect("start unbound-files");
    let centre = |scanout: &Scanout| pixel(scanout, scanout.width / 2, scanout.height / 2);
    wait_for("the files window", STARTUP, || {
        centre(&scanout()) != centre(&without_files)
    });
    let menu_bar = |scanout: &Scanout| scanout.rgb[..scanout.width * 24].to_vec();
    wait_for("the menu bar to name Files", STARTUP, || {
        running("unbound-menubar") && menu_bar(&scanout()) != menu_bar(&without_files)
    });
    thread::sleep(Duration::from_secs(1));

    let shown = scanout();
    keyboard.tap(KEY_DOWN);
    wait_for("the selected folder's column", STARTUP, || {
        scanout().rgb != shown.rgb
    });
    keyboard.tap(KEY_RIGHT);
    thread::sleep(Duration::from_secs(1));
    scanout().save("m22-files-columns");
    let magenta_pixels =
        |scanout: &Scanout| scanout.rgb.iter().filter(|&&rgb| rgb == magenta).count();
    assert_eq!(magenta_pixels(&scanout()), 0);

    keyboard.tap(KEY_SPACE);
    let picture = (picture_width * picture_height) as usize;
    wait_for("the picture in Quick Look", STARTUP, || {
        magenta_pixels(&scanout()) >= picture * 9 / 10
    });
    thread::sleep(Duration::from_millis(500));
    scanout().save("m22-files-quick-look");

    keyboard.tap(KEY_ESC);
    wait_for("Quick Look to close", STARTUP, || {
        magenta_pixels(&scanout()) == 0
    });
    let _ = files.kill();
    let _ = files.wait();
}

const GREETER_USER: &str = "ada:x:1000:1000:Ada Lovelace,,,:/root:/bin/bash";

#[test]
#[ignore = "needs the ci/vm guest"]
fn the_greeter_signs_in_through_greetd_and_hands_over() {
    use std::os::unix::net::UnixListener;
    use unbound_greeter::{AuthMessageType, Request, Response, read_message, write_message};

    let passwd = fs::read_to_string("/etc/passwd").expect("passwd");
    if !passwd.contains(GREETER_USER) {
        fs::write("/etc/passwd", format!("{passwd}{GREETER_USER}\n")).expect("add a user");
    }
    let socket = Path::new(RUNTIME_DIR).join("greetd.sock");
    fs::create_dir_all(RUNTIME_DIR).expect("runtime directory");
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("greetd socket");
    let greetd = thread::spawn(move || {
        let (mut greeter, _) = listener.accept().expect("the greeter connects");
        let mut requests = Vec::new();
        let replies = [
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Secret,
                auth_message: "Password: ".to_owned(),
            },
            Response::Success,
            Response::Success,
        ];
        for reply in replies {
            requests.push(read_message::<Request>(&mut greeter).expect("request"));
            write_message(&mut greeter, &reply).expect("reply");
        }
        requests
    });

    let mut keyboard = VirtualDevice::keyboard_and_mouse();
    let mut desktop = start_desktop_with_arguments(
        &["--greeter"],
        &[("GREETD_SOCK", socket.to_str().expect("socket path"))],
    );
    wait_for("the greeter", STARTUP, || running("unbound-greeter"));
    thread::sleep(Duration::from_secs(2));
    scanout().save("m24-greeter");
    assert!(
        !running("unbound-panel"),
        "the greeter session has no panel"
    );

    keyboard.type_text("secret\n");
    let requests = greetd.join().expect("greetd");
    assert_eq!(
        requests,
        [
            Request::CreateSession {
                username: "ada".to_owned()
            },
            Request::PostAuthMessageResponse {
                response: Some("secret".to_owned())
            },
            Request::StartSession {
                cmd: vec!["unbound-compositor".to_owned()],
                env: Vec::new()
            },
        ]
    );
    wait_for("the compositor to hand over", STARTUP, || {
        desktop
            .compositor
            .try_wait()
            .ok()
            .flatten()
            .is_some_and(|status| status.success())
    });
}
