#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bande_app::{Cx, Error, Info, Keys, Kind, Menus, Program, View};
use bande_app_menu::{Entry, MENU_PATH};
use bande_compositor::Compositor;
use bande_status::dbusmenu::{DBusMenu, Node, tree};
use bande_test_bus::SessionBus;
use bande_test_wayland::keys::{KEY_LEFTALT, KEY_LEFTCTRL, KEY_Q, KEY_TAB, KEY_W};
use bande_test_wayland::{Frames, TestClient, chord, headless};
use bande_toolkit::{Appearance, Scene, Window, WindowEvent};

static ASKED: Mutex<Option<Sender<Asked>>> = Mutex::new(None);

type Asked = (Vec<String>, PathBuf);

struct Pane;

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        Scene {
            width,
            height,
            background: window.appearance().palette().sheet,
            items: Vec::new(),
        }
    }

    fn event(&mut self, _window: &mut Window, _event: WindowEvent) -> bool {
        false
    }

    fn set_appearance(&mut self, _appearance: Appearance) {}

    fn animating(&self) -> bool {
        false
    }
}

struct Example;

fn open(cx: &mut Cx<Example>) {
    cx.open(Kind::Main, "Example", (200, 120), |_| Pane)
        .expect("window");
}

impl Program for Example {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, Error> {
        open(cx);
        Ok(Self)
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        open(cx);
    }

    fn menus(&self, _cx: &Cx<Self>) -> Menus {
        Menus {
            menus: vec![
                Entry::submenu(
                    "File",
                    vec![
                        Entry::item("New Window", "new")
                            .shortcut("Ctrl+N")
                            .primary(),
                        Entry::separator(),
                        Entry::item("Export…", "export"),
                    ],
                ),
                Entry::submenu("View", vec![Entry::item("Zoom In", "zoom-in").primary()]),
            ],
            window: Vec::new(),
        }
    }

    fn open(&mut self, _cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        if let Some(sender) = ASKED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            let _ = sender.send((arguments, directory.to_path_buf()));
        }
    }
}

static INFO: Info = Info {
    id: "org.bande.Example",
    package: "bande-example",
    version: "1.2.3",
    name: || "Example".to_owned(),
    keys: Keys::Standard,
    settings: None,
};

fn launch(arguments: &[&str], display: UnixStream) -> JoinHandle<Result<(), String>> {
    let arguments = arguments.iter().map(ToString::to_string).collect();
    thread::spawn(move || {
        bande_app::run_on::<Example>(&INFO, arguments, display).map_err(|error| error.to_string())
    })
}

fn labels(node: &Node) -> Vec<&str> {
    node.children
        .iter()
        .map(|child| child.label.as_str())
        .collect()
}

fn child<'a>(node: &'a Node, label: &str) -> &'a Node {
    node.children
        .iter()
        .find(|child| child.label == label)
        .unwrap_or_else(|| panic!("no {label} in {:?}", labels(node)))
}

fn wait_for_windows(frames: &mut Frames, server: &mut Compositor, wanted: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while server
        .windows()
        .iter()
        .filter(|window| window.geometry().is_some())
        .count()
        != wanted
    {
        assert!(Instant::now() < deadline, "never {wanted} windows");
        let mut stepped = false;
        frames.run_until(server, || std::mem::replace(&mut stepped, true));
    }
}

#[test]
fn a_second_launch_reaches_the_running_app_and_the_switcher_activates_it_windowless() {
    let bus = SessionBus::start().expect("private session bus");
    // SAFETY: this binary's only test sets these before it starts the app, so no other thread reads
    // the environment yet.
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
        std::env::set_var(
            "XDG_STATE_HOME",
            std::env::temp_dir().join("bande-app-state"),
        );
    }
    let (sender, asked) = mpsc::channel();
    *ASKED.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
    let mut server = headless(640, 480);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let app = launch(&[], client_end);
    let mut frames = Frames::start();

    wait_for_windows(&mut frames, &mut server, 1);
    let menus = bus.connect().expect("menu reader");
    let menu = DBusMenu::builder(&menus)
        .destination(INFO.id)
        .and_then(|builder| builder.path(MENU_PATH))
        .and_then(|builder| builder.build())
        .expect("the app's menus");
    let (_, layout) = menu.get_layout(0, -1, &[]).expect("menu layout");
    let root = tree(layout);
    assert_eq!(
        labels(&root),
        [
            "New Window",
            "Zoom In",
            "",
            "File",
            "Window",
            "",
            "About Example",
            "Quit Example"
        ]
    );
    assert_eq!(labels(child(&root, "File")), ["Export…"]);
    let window = child(&root, "Window");
    for item in ["Tile", "Keep on Top", "Close Window", "Hide Example"] {
        child(window, item);
    }
    let left = child(child(window, "Tile"), "Left");
    menu.event(left.id, "clicked", &zbus::zvariant::Value::from(""), 0)
        .expect("click Window ▸ Tile ▸ Left");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !server
        .windows()
        .iter()
        .filter_map(|window| window.geometry())
        .any(|geometry| geometry.loc.x < 40 && geometry.size.w > 200 && geometry.size.h > 300)
    {
        assert!(Instant::now() < deadline, "the window never tiled left");
        let mut stepped = false;
        frames.run_until(&mut server, || std::mem::replace(&mut stepped, true));
    }

    let (unused, _) = UnixStream::pair().expect("socket pair");
    let second_launch = launch(&["notes.txt"], unused);
    let wanted = (
        vec!["notes.txt".to_owned()],
        std::env::current_dir().expect("directory"),
    );
    frames.run_until(&mut server, || {
        asked.try_recv().is_ok_and(|seen| seen == wanted)
    });
    assert_eq!(second_launch.join().expect("second launch"), Ok(()));

    chord(&mut server, &[KEY_LEFTCTRL, KEY_W]);
    wait_for_windows(&mut frames, &mut server, 0);
    assert!(!app.is_finished());

    let (unused, _) = UnixStream::pair().expect("socket pair");
    let launched_again = launch(&[], unused);
    wait_for_windows(&mut frames, &mut server, 1);
    assert_eq!(launched_again.join().expect("launched again"), Ok(()));

    chord(&mut server, &[KEY_LEFTCTRL, KEY_W]);
    wait_for_windows(&mut frames, &mut server, 0);
    let _other = TestClient::window(&mut server, "org.bande.Other");
    wait_for_windows(&mut frames, &mut server, 1);
    chord(&mut server, &[KEY_LEFTALT, KEY_TAB]);
    let settled = Instant::now() + Duration::from_millis(500);
    frames.run_until(&mut server, || Instant::now() > settled);
    wait_for_windows(&mut frames, &mut server, 1);

    chord(&mut server, &[KEY_LEFTCTRL, KEY_Q]);
    frames.run_until(&mut server, || app.is_finished());
    assert_eq!(app.join().expect("app thread"), Ok(()));
}
