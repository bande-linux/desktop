#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use bande_compositor::{Compositor, SessionRole};
use bande_test_bus::SessionBus;
use bande_test_bus::atspi::{FakeRegistry, ObjectRef, accessible, find_named};
use bande_test_wayland::keys::{KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_UP};
use bande_test_wayland::{
    Frames, SeparateApp, act_as_separate_app, chord, headless, is_separate_app,
};
use zbus::blocking::{Connection, connection};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const TEST: &str = "an_app_is_in_the_taskbar_exactly_as_long_as_its_process_runs";
const PATIENCE: Duration = Duration::from_secs(40);
const SETTLED: Duration = Duration::from_millis(400);
const WINDOW_COORDINATES: u32 = 1;
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const TRAY: &str = "org.example.Tray";
const TRAY_NAME: &str = "Example Tray";
const KEPT: &str = "org.example.Kept";
const KEPT_NAME: &str = "Example Kept";
const HELPER: &str = "org.example.Helper";
const HELPER_NAME: &str = "Example Helper";
const BANDE: &str = "org.bande.Example";
const BANDE_NAME: &str = "Example Bande";
const UNNAMED: &str = "unnamed-dialog";
const ITEM_NAME: &str = "Example Item";
const LOGO: (f64, f64) = (40.0, 34.0);
const QUIT: i32 = 7;
const HELPER_SECONDS: &str = "1";
const ITEM: &str = "org.example.TrayItem";

fn home(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap()).join(name)
}

struct Item(String);

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    fn activate(&self, _x: i32, _y: i32) {
        wait("slow-activation");
        note("activations", "shown");
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        ITEM_NAME
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        &self.0
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from("/Menu").unwrap()
    }
}

type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

struct ItemMenu;

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl ItemMenu {
    fn get_layout(&self, _parent: i32, _depth: i32, _names: Vec<String>) -> (u32, Layout) {
        wait("slow-menu");
        let label = OwnedValue::try_from(Value::from("Leave")).unwrap();
        let leave: Layout = (
            QUIT,
            HashMap::from([("label".to_owned(), label)]),
            Vec::new(),
        );
        let leave = OwnedValue::try_from(Value::from(leave)).unwrap();
        (1, (0, HashMap::new(), vec![leave]))
    }

    fn event(&self, id: i32, event: &str, _data: Value<'_>, _time: u32) {
        if (id, event) == (QUIT, "clicked") {
            std::process::exit(0);
        }
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }
}

struct Watcher;

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_host(&self, _service: &str) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        vec!["org.example.Gone".to_owned(), ITEM.to_owned()]
    }
}

fn note(file: &str, what: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(home(file))
        .unwrap();
    writeln!(file, "{what}").unwrap();
}

fn wait(file: &str) {
    if let Ok(milliseconds) = noted(file).trim().parse() {
        std::thread::sleep(Duration::from_millis(milliseconds));
    }
}

fn noted(file: &str) -> String {
    fs::read_to_string(home(file)).unwrap_or_default()
}

fn separate_app() {
    let mut helpers = [HELPER_SECONDS, "30"].map(|seconds| {
        Command::new("sleep")
            .arg(seconds)
            .stdin(Stdio::null())
            .spawn()
            .unwrap()
    });
    let item = noted("item");
    let _bus = (!item.is_empty()).then(|| {
        let bus = connection::Builder::session()
            .unwrap()
            .serve_at("/StatusNotifierItem", Item(item))
            .unwrap()
            .serve_at("/Menu", ItemMenu)
            .unwrap()
            .name(ITEM)
            .unwrap()
            .build()
            .unwrap();
        bus.emit_signal(
            None::<&str>,
            "/StatusNotifierItem",
            "org.kde.StatusNotifierItem",
            "NewIcon",
            &(),
        )
        .unwrap();
        bus
    });
    act_as_separate_app();
    helpers[0].wait().unwrap();
}

fn install(app_id: &str, name: &str, more: &str) {
    let folder = home(".local/share/applications");
    fs::create_dir_all(&folder).unwrap();
    let script = home(&format!("{app_id}.sh"));
    let launches = home("launches");
    fs::write(
        &script,
        format!("echo {app_id} >> {}\n", launches.display()),
    )
    .unwrap();
    fs::write(
        folder.join(format!("{app_id}.desktop")),
        format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/sh {}\n{more}",
            script.display()
        ),
    )
    .unwrap();
}

struct Scene {
    server: Compositor,
    frames: Frames,
    atspi: Connection,
    root: ObjectRef,
}

impl Scene {
    fn pass(&mut self, time: Duration) {
        let until = Instant::now() + time;
        self.frames
            .run_until(&mut self.server, || Instant::now() >= until);
    }

    fn button(&self, app: &str) -> Option<ObjectRef> {
        find_named(&self.atspi, &self.root, app)
    }

    fn marked(&self, app: &str) -> bool {
        self.button(app).is_some_and(|found| {
            accessible(&self.atspi, &found, "org.a11y.atspi.Accessible")
                .and_then(|node| node.get_property::<String>("Description"))
                .is_ok_and(|text| text.contains("running"))
        })
    }

    fn wait(&mut self, what: &str, mut done: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(self) {
            assert!(Instant::now() < deadline, "waited in vain: {what}");
            self.pass(SETTLED);
        }
    }

    fn middle(&self, app: &str) -> (f64, f64) {
        let found = self.button(app).expect("in the taskbar");
        let (x, y, width, height): (i32, i32, i32, i32) =
            accessible(&self.atspi, &found, "org.a11y.atspi.Component")
                .and_then(|component| component.call("GetExtents", &(WINDOW_COORDINATES,)))
                .expect("where the taskbar button is");
        (f64::from(x + width / 2), f64::from(y + height / 2))
    }

    fn right_click_at_once(&mut self, at: (f64, f64)) {
        self.server.move_pointer_to(at.into(), 0);
        self.server.press_button(BTN_RIGHT, true, 0);
        self.server.press_button(BTN_RIGHT, false, 0);
    }

    fn press(&mut self, app: &str, button: u32, times: u32) {
        let middle = self.middle(app);
        self.server.move_pointer_to(middle.into(), 0);
        self.pass(SETTLED);
        for _ in 0..times {
            self.server.press_button(button, true, 0);
            self.pass(Duration::from_millis(60));
            self.server.press_button(button, false, 0);
            self.pass(Duration::from_millis(60));
        }
        self.pass(SETTLED);
    }

    fn choose_last_from_menu(&mut self, app: &str) {
        self.press(app, BTN_RIGHT, 1);
        chord(&mut self.server, &[KEY_UP]);
        self.pass(SETTLED);
        chord(&mut self.server, &[KEY_ENTER]);
    }

    fn app(&mut self, app_id: &str) -> SeparateApp {
        let mut app = SeparateApp::launch(&mut self.server, TEST);
        app.show_window_of(&mut self.server, 64, app_id);
        app
    }

    fn closed(&mut self, app: &mut SeparateApp) {
        self.pass(SETTLED);
        app.close_windows();
        self.wait("the windows close", |scene| {
            scene.server.windows().is_empty()
        });
        self.pass(SETTLED);
    }
}

#[test]
fn an_app_is_in_the_taskbar_exactly_as_long_as_its_process_runs() {
    if is_separate_app() {
        separate_app();
        return;
    }
    let bus = SessionBus::start().expect("private session and AT-SPI bus");
    // SAFETY: this binary's only test sets these before it starts the registry or the shell, so
    // no other thread reads the environment yet.
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
        std::env::set_var("AT_SPI_BUS_ADDRESS", bus.address());
    }
    install(TRAY, TRAY_NAME, "");
    install(KEPT, KEPT_NAME, "");
    install(BANDE, BANDE_NAME, "");
    install(HELPER, HELPER_NAME, "NoDisplay=true\n");
    let config = home(".config/bande");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("config.kdl"),
        format!("bar {{\n    apps \"{KEPT}\"\n}}\n"),
    )
    .unwrap();
    let _watcher = connection::Builder::session()
        .unwrap()
        .name("org.kde.StatusNotifierWatcher")
        .unwrap()
        .serve_at("/StatusNotifierWatcher", Watcher)
        .unwrap()
        .build()
        .unwrap();
    let registry = FakeRegistry::start(&bus).expect("AT-SPI registry");
    let mut server = headless(1024, 768);
    let [mut first, second] = [64, 80].map(|side| {
        let mut app = SeparateApp::launch(&mut server, TEST);
        app.show_window_of(&mut server, side, TRAY);
        app
    });
    server
        .start_session_client(
            Path::new(env!("CARGO_BIN_EXE_bande-shell")),
            SessionRole::Shell,
        )
        .expect("start the shell");
    let mut frames = Frames::start();
    let mut root = None;
    frames.run_until(&mut server, || {
        root = registry.embedded_root(Duration::ZERO);
        root.is_some()
    });
    let mut scene = Scene {
        server,
        frames,
        atspi: connection::Builder::address(bus.address())
            .expect("address")
            .method_timeout(Duration::from_secs(2))
            .build()
            .expect("screen reader connection"),
        root: root.expect("the shell's accessible root"),
    };

    scene.wait("a window marks its app", |scene| scene.marked(TRAY_NAME));
    scene.pass(SETTLED);
    first.close_windows();
    scene.wait("the first closes", |scene| {
        scene.server.windows().len() == 1
    });
    drop(second);
    scene.wait("the second ends", |scene| scene.server.windows().is_empty());
    scene.pass(SETTLED);
    assert!(scene.marked(TRAY_NAME), "the first still runs");
    let mut third = SeparateApp::launch(&mut scene.server, TEST);
    third.show_window_of(&mut scene.server, 72, TRAY);
    scene.pass(SETTLED);
    drop(first);
    scene.pass(SETTLED);
    assert!(scene.marked(TRAY_NAME), "the third has a window");
    drop(third);
    scene.wait("an ended process never comes back", |scene| {
        scene.button(TRAY_NAME).is_none()
    });

    fs::write(home("item"), TRAY).unwrap();
    let mut hidden = SeparateApp::launch(&mut scene.server, TEST);
    scene.wait("an app that started in the tray", |scene| {
        scene.marked(TRAY_NAME)
    });
    scene.press(TRAY_NAME, BTN_LEFT, 2);
    scene.wait("its status item is asked", |_| {
        !noted("activations").is_empty()
    });
    scene.pass(SETTLED);
    assert_eq!(noted("activations"), "shown\n", "asked once");
    hidden.show_window_of(&mut scene.server, 80, TRAY);
    scene.closed(&mut hidden);
    assert!(scene.marked(TRAY_NAME), "it still runs");

    scene.wait("its status item is shown", |scene| {
        scene.button(ITEM_NAME).is_some()
    });
    scene.press(ITEM_NAME, BTN_LEFT, 1);
    chord(&mut scene.server, &[KEY_ESC]);
    scene.pass(SETTLED);
    fs::write(home("slow-menu"), "1500").unwrap();
    let icon = scene.middle(TRAY_NAME);
    scene.right_click_at_once(icon);
    scene.right_click_at_once(LOGO);
    scene.pass(Duration::from_secs(3));
    chord(&mut scene.server, &[KEY_DOWN]);
    chord(&mut scene.server, &[KEY_ENTER]);
    scene.pass(Duration::from_secs(1));
    assert!(
        scene.marked(TRAY_NAME),
        "a late menu took the place of the one that was open"
    );
    assert_eq!(noted("activations"), "shown\n", "nor did the icon menu");
    fs::remove_file(home("slow-menu")).unwrap();

    fs::write(home("slow-activation"), "28000").unwrap();
    scene.press(TRAY_NAME, BTN_LEFT, 1);
    scene.pass(Duration::from_secs(30));
    assert_eq!(noted("launches"), "", "a slow status item opens nothing");
    fs::remove_file(home("slow-activation")).unwrap();
    scene.choose_last_from_menu(TRAY_NAME);
    scene.wait("its own menu ends it", |scene| {
        scene.button(TRAY_NAME).is_none()
    });
    assert_eq!(noted("launches"), "");

    fs::remove_file(home("item")).unwrap();
    let mut plain = scene.app(TRAY);
    scene.closed(&mut plain);
    scene.pass(Duration::from_secs(2));
    assert!(scene.marked(TRAY_NAME), "only a helper ended");
    scene.press(TRAY_NAME, BTN_LEFT, 2);
    scene.wait("without a status item it is opened", |_| {
        !noted("launches").is_empty()
    });
    scene.pass(SETTLED);
    assert_eq!(noted("launches"), format!("{TRAY}\n"), "opened once");
    drop(plain);
    scene.wait("a helper left behind is not the app", |scene| {
        scene.button(TRAY_NAME).is_none()
    });

    let mut kept = scene.app(KEPT);
    scene.closed(&mut kept);
    assert!(scene.marked(KEPT_NAME));
    drop(kept);
    scene.wait("a kept app loses only its mark", |scene| {
        scene.button(KEPT_NAME).is_some() && !scene.marked(KEPT_NAME)
    });

    for (app_id, name) in [
        (HELPER, HELPER_NAME),
        (UNNAMED, UNNAMED),
        (BANDE, BANDE_NAME),
    ] {
        let mut other = scene.app(app_id);
        scene.closed(&mut other);
        assert!(scene.button(name).is_none(), "{name} left an icon");
    }

    let mut both = scene.app(TRAY);
    both.show_window_of(&mut scene.server, 80, KEPT);
    scene.closed(&mut both);
    assert!(
        scene.button(TRAY_NAME).is_none() && !scene.marked(KEPT_NAME),
        "a process shared by two apps says nothing about either"
    );
}
