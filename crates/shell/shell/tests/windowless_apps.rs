#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bande_compositor::{Compositor, SessionRole};
use bande_test_bus::SessionBus;
use bande_test_bus::atspi::{FakeRegistry, ObjectRef, accessible, find_named};
use bande_test_wayland::keys::{KEY_ENTER, KEY_UP};
use bande_test_wayland::{
    Frames, SeparateApp, act_as_separate_app, chord, headless, is_separate_app,
};
use zbus::blocking::{Connection, connection};

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
const ITEM: &str = "org.example.TrayItem";

fn home(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap()).join(name)
}

struct Item;

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    fn activate(&self, _x: i32, _y: i32) {
        fs::write(home("activated"), "").unwrap();
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

    fn press(&mut self, app: &str, button: u32, times: u32) {
        let found = self.button(app).expect("in the taskbar");
        let (x, y, width, height): (i32, i32, i32, i32) =
            accessible(&self.atspi, &found, "org.a11y.atspi.Component")
                .and_then(|component| component.call("GetExtents", &(WINDOW_COORDINATES,)))
                .expect("where the taskbar button is");
        let middle = (f64::from(x + width / 2), f64::from(y + height / 2));
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

    fn quit_from_menu(&mut self, app: &str) {
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
        let _item = connection::Builder::session()
            .and_then(|builder| builder.serve_at("/StatusNotifierItem", Item))
            .and_then(connection::Builder::build)
            .inspect(|bus| drop(bus.request_name(ITEM)))
            .unwrap();
        act_as_separate_app();
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

    let mut tray = scene.app(TRAY);
    scene.wait("a window marks its app", |scene| scene.marked(TRAY_NAME));
    scene.closed(&mut tray);
    assert!(scene.marked(TRAY_NAME), "it still runs");
    scene.press(TRAY_NAME, BTN_LEFT, 2);
    let launches = || fs::read_to_string(home("launches")).unwrap_or_default();
    assert_eq!(launches(), format!("{TRAY}\n"), "opened once");
    scene.wait("its status item is asked", |_| home("activated").exists());
    tray.show_window_of(&mut scene.server, 80, TRAY);
    scene.closed(&mut tray);
    scene.quit_from_menu(TRAY_NAME);
    scene.wait("Quit ends it", |scene| scene.button(TRAY_NAME).is_none());

    let mut tray = scene.app(TRAY);
    scene.closed(&mut tray);
    assert!(scene.marked(TRAY_NAME));
    drop(tray);
    scene.wait("an app that ended leaves", |scene| {
        scene.button(TRAY_NAME).is_none()
    });

    let mut tray = scene.app(TRAY);
    scene.wait("back in the taskbar", |scene| scene.marked(TRAY_NAME));
    scene.quit_from_menu(TRAY_NAME);
    scene.pass(SETTLED);
    tray.close_windows();
    scene.wait("Quit with a window open ends it too", |scene| {
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
