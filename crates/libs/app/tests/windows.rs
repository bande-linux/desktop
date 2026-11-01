#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use bande_app::{Cx, Error, Info, Keys, Kind, Menus, Program, Settings, View, WindowId};
use bande_app_menu::{Entry, MENU_PATH, edit_submenu};
use bande_compositor::{Compositor, FrameClock, FrameScheduler, REFRESH, Renderer};
use bande_test_bus::SessionBus;
use bande_toolkit::ui::{Event, Key};
use bande_toolkit::{Appearance, Scene, Scheme, Window, WindowEvent};
use smithay::utils::Size;
use zbus::zvariant::{OwnedValue, Value};

const OUTPUT: (i32, i32) = (640, 480);
const TIMEOUT: Duration = Duration::from_secs(60);
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const APPEARANCE: &str = "org.freedesktop.appearance";
const KEY_LEFTCTRL: u32 = 29;
const KEY_Q: u32 = 16;
const KEY_W: u32 = 17;
const KEY_C: u32 = 46;
const KEY_N: u32 = 49;
const KEY_COMMA: u32 = 51;
const NEW: &str = "new";
const COPY_ALL: &str = "copy-all";

static RECORD: Mutex<Option<Sender<Seen>>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq)]
enum Seen {
    Opened(WindowId),
    Active(WindowId),
    Dark(WindowId),
    Copied(WindowId),
    Command(Option<WindowId>, String),
    AskedToClose(WindowId),
    Asked(Vec<String>, PathBuf),
    SettingsBuilt,
    SettingsActive,
}

fn record(seen: Seen) {
    if let Some(sender) = RECORD
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        let _ = sender.send(seen);
    }
}

struct Pane {
    id: Option<WindowId>,
}

impl Pane {
    fn record(&self, seen: fn(WindowId) -> Seen) {
        if let Some(id) = self.id {
            record(seen(id));
        }
    }
}

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

    fn event(&mut self, _window: &mut Window, event: WindowEvent) -> bool {
        match event {
            WindowEvent::Input(Event::Key {
                key: Key::Character('c'),
                modifiers,
            }) if modifiers.control => {
                self.record(Seen::Copied);
                true
            }
            _ => false,
        }
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        if appearance.scheme == Scheme::Dark {
            self.record(Seen::Dark);
        }
    }

    fn set_active(&mut self, active: bool) {
        if active {
            self.record(Seen::Active);
        }
    }
}

struct SettingsPane;

impl View for SettingsPane {
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

    fn set_active(&mut self, active: bool) {
        if active {
            record(Seen::SettingsActive);
        }
    }
}

fn settings_pane(_appearance: Appearance) -> Box<dyn View> {
    record(Seen::SettingsBuilt);
    Box::new(SettingsPane)
}

struct Example {
    refused: bool,
}

fn open(cx: &mut Cx<Example>) {
    let window = cx
        .open(Kind::Main, "Example", (200, 120), |_| Pane { id: None })
        .expect("window");
    if let Some((_, pane)) = cx.window(window) {
        pane.id = Some(window);
    }
    record(Seen::Opened(window));
}

impl Program for Example {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, Error> {
        open(cx);
        open(cx);
        Ok(Self { refused: false })
    }

    fn menus(&self, _cx: &Cx<Self>) -> Menus {
        Menus {
            menus: vec![
                Entry::submenu(
                    "File",
                    vec![
                        Entry::item("New", NEW).shortcut("Ctrl+N"),
                        Entry::item("Copy All", COPY_ALL).shortcut("Ctrl+C"),
                    ],
                ),
                edit_submenu(Vec::new()),
            ],
            window: Vec::new(),
        }
    }

    fn command(&mut self, _cx: &mut Cx<Self>, window: Option<WindowId>, command: &str) {
        record(Seen::Command(window, command.to_owned()));
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        open(cx);
    }

    fn open(&mut self, _cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        record(Seen::Asked(arguments, directory.to_path_buf()));
    }

    fn should_close(&mut self, _cx: &mut Cx<Self>, window: WindowId) -> bool {
        record(Seen::AskedToClose(window));
        std::mem::replace(&mut self.refused, true)
    }
}

static INFO: Info = Info {
    id: "org.bande.Example",
    package: "bande-example",
    version: "1.2.3",
    name: || "Example".to_owned(),
    keys: Keys::Standard,
    settings: Some(Settings {
        title: || "Example Settings".to_owned(),
        size: (160, 100),
        view: settings_pane,
    }),
};

struct Portal;

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Portal {
    fn read_all(&self, _namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        HashMap::new()
    }
}

type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

fn menu_item(node: &Layout, wanted: &str) -> Option<i32> {
    let (id, properties, children) = node;
    let label = properties
        .get("label")
        .and_then(|label| String::try_from(label.try_clone().ok()?).ok());
    if label.as_deref() == Some(wanted) {
        return Some(*id);
    }
    children.iter().find_map(|child| {
        let child: Layout = child.try_clone().ok()?.try_into().ok()?;
        menu_item(&child, wanted)
    })
}

fn click_menu_item(client: &zbus::blocking::Connection, label: &str) {
    let (_, root): (u32, Layout) = client
        .call_method(
            Some(INFO.id),
            MENU_PATH,
            Some("com.canonical.dbusmenu"),
            "GetLayout",
            &(0, -1, Vec::<String>::new()),
        )
        .expect("GetLayout")
        .body()
        .deserialize()
        .expect("layout");
    let id = menu_item(&root, label).expect("the menu item");
    client
        .call_method(
            Some(INFO.id),
            MENU_PATH,
            Some("com.canonical.dbusmenu"),
            "Event",
            &(id, "clicked", Value::from(""), 0u32),
        )
        .expect("Event");
}

struct Desktop {
    server: Compositor,
    renderer: Renderer,
    scheduler: FrameScheduler,
    start: Instant,
    seen: Vec<Seen>,
    records: Receiver<Seen>,
}

impl Desktop {
    fn pump(&mut self) {
        self.server.dispatch().expect("server dispatch");
        let start = self.start;
        self.scheduler
            .poll(|| start.elapsed(), &mut self.server, &mut self.renderer)
            .expect("compositor frame");
        self.seen.extend(self.records.try_iter());
        thread::sleep(Duration::from_millis(1));
    }

    fn wait_for(&mut self, seen: &Seen) {
        self.wait_for_after(0, seen);
    }

    fn wait_for_after(&mut self, mark: usize, seen: &Seen) {
        let deadline = Instant::now() + TIMEOUT;
        while !self.seen[mark..].contains(seen) {
            self.pump();
            assert!(
                Instant::now() < deadline,
                "no {seen:?} after {:?}",
                self.seen
            );
        }
    }

    fn settle(&mut self) {
        for _ in 0..100 {
            self.pump();
        }
    }

    fn chord(&mut self, key: u32) {
        self.server.key(KEY_LEFTCTRL, true, 0);
        self.server.key(key, true, 0);
        self.server.key(key, false, 0);
        self.server.key(KEY_LEFTCTRL, false, 0);
    }

    fn since(&mut self, mark: usize) -> Vec<Seen> {
        self.seen.split_off(mark)
    }
}

#[test]
fn windows_share_keys_menus_appearance_closing_and_the_app_runs_on_without_them_until_quit() {
    let bus = SessionBus::start().expect("private session bus");
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
    }
    let portal = bus.connect().expect("portal connection");
    portal
        .object_server()
        .at(PORTAL_PATH, Portal)
        .expect("serve the settings");
    portal
        .request_name("org.freedesktop.portal.Desktop")
        .expect("own the portal name");

    let (sender, records) = mpsc::channel();
    *RECORD.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let app = thread::spawn(move || {
        bande_app::run_on::<Example>(&INFO, Vec::new(), client_end)
            .map_err(|error| error.to_string())
    });
    let mut desktop = Desktop {
        server,
        renderer: Renderer::new().expect("Vulkan renderer"),
        scheduler: FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH)),
        start: Instant::now(),
        seen: Vec::new(),
        records,
    };

    let deadline = Instant::now() + TIMEOUT;
    let (first, second) = loop {
        desktop.pump();
        let opened: Vec<WindowId> = desktop
            .seen
            .iter()
            .filter_map(|seen| match seen {
                Seen::Opened(window) => Some(*window),
                _ => None,
            })
            .collect();
        if let [first, second] = opened[..] {
            break (first, second);
        }
        assert!(Instant::now() < deadline, "the windows did not open");
    };
    desktop.wait_for(&Seen::Active(second));
    desktop.settle();

    portal
        .emit_signal(
            None::<()>,
            PORTAL_PATH,
            SETTINGS,
            "SettingChanged",
            &(APPEARANCE, "color-scheme", Value::from(1u32)),
        )
        .expect("emit SettingChanged");
    desktop.wait_for(&Seen::Dark(first));
    desktop.wait_for(&Seen::Dark(second));

    let (unused, _) = UnixStream::pair().expect("socket pair");
    let second_launch = thread::spawn(move || {
        bande_app::run_on::<Example>(&INFO, vec!["second".to_owned()], unused)
            .map_err(|error| error.to_string())
    });
    desktop.wait_for(&Seen::Asked(
        vec!["second".to_owned()],
        std::env::current_dir().expect("directory"),
    ));
    assert_eq!(second_launch.join().expect("second launch"), Ok(()));

    let mark = desktop.seen.len();
    desktop.chord(KEY_C);
    desktop.chord(KEY_N);
    desktop.wait_for(&Seen::Command(Some(second), NEW.to_owned()));
    assert_eq!(
        desktop.since(mark),
        [
            Seen::Copied(second),
            Seen::Command(Some(second), NEW.to_owned())
        ]
    );

    let mark = desktop.seen.len();
    let menu_bar = bus.connect().expect("menu bar connection");
    click_menu_item(&menu_bar, "Copy");
    desktop.wait_for_after(mark, &Seen::Copied(second));

    desktop.chord(KEY_COMMA);
    desktop.wait_for(&Seen::SettingsActive);
    let mark = desktop.seen.len();
    desktop.chord(KEY_COMMA);
    desktop.settle();
    desktop.chord(KEY_W);
    desktop.wait_for_after(mark, &Seen::Active(second));
    assert_eq!(
        desktop
            .seen
            .iter()
            .filter(|seen| **seen == Seen::SettingsBuilt)
            .count(),
        1
    );

    desktop.chord(KEY_W);
    desktop.wait_for(&Seen::AskedToClose(second));
    desktop.settle();
    let mark = desktop.seen.len();
    desktop.chord(KEY_W);
    desktop.wait_for_after(mark, &Seen::Active(first));
    desktop.chord(KEY_W);
    desktop.wait_for(&Seen::AskedToClose(first));
    desktop.settle();
    let running_without_windows = !app.is_finished();

    desktop.chord(KEY_N);
    desktop.wait_for(&Seen::Command(None, NEW.to_owned()));
    let (unused, _) = UnixStream::pair().expect("socket pair");
    let launched_again = thread::spawn(move || {
        bande_app::run_on::<Example>(&INFO, Vec::new(), unused).map_err(|error| error.to_string())
    });
    let deadline = Instant::now() + TIMEOUT;
    let reopened = loop {
        desktop.pump();
        let opened = desktop.seen.iter().rev().find_map(|seen| match seen {
            Seen::Opened(window) if ![first, second].contains(window) => Some(*window),
            _ => None,
        });
        if let Some(reopened) = opened {
            break reopened;
        }
        assert!(
            Instant::now() < deadline,
            "launching again opened no window"
        );
    };
    assert_eq!(launched_again.join().expect("second launch"), Ok(()));
    desktop.wait_for(&Seen::Active(reopened));
    desktop.chord(KEY_W);
    desktop.wait_for(&Seen::AskedToClose(reopened));
    desktop.settle();

    desktop.chord(KEY_Q);
    let deadline = Instant::now() + TIMEOUT;
    while !app.is_finished() {
        desktop.pump();
        assert!(
            Instant::now() < deadline,
            "Ctrl+Q without a window did not quit"
        );
    }
    assert!(running_without_windows);
    assert_eq!(app.join().expect("app thread"), Ok(()));
    assert_eq!(
        desktop
            .seen
            .iter()
            .filter(|seen| matches!(seen, Seen::AskedToClose(_)))
            .collect::<Vec<_>>(),
        [
            &Seen::AskedToClose(second),
            &Seen::AskedToClose(second),
            &Seen::AskedToClose(first),
            &Seen::AskedToClose(reopened),
        ]
    );
}
