#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bande_app::{Cx, Error, Info, Keys, Kind, Program, View, WindowId};
use bande_compositor::Compositor;
use bande_test_bus::SessionBus;
use bande_test_wayland::{Frames, chord, headless};
use bande_toolkit::{Appearance, Scene, Window, WindowEvent};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;

const KEY_LEFTCTRL: u32 = 29;
const KEY_Q: u32 = 16;
const KEY_W: u32 = 17;
const KEY_F11: u32 = 87;

static RECORD: Mutex<Option<Sender<Seen>>> = Mutex::new(None);
static ONE_BUS_AT_A_TIME: Mutex<()> = Mutex::new(());

fn private_bus() -> SessionBus {
    let bus = SessionBus::start().expect("private session bus");
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
    }
    bus
}

#[derive(Debug, Clone, PartialEq)]
enum Seen {
    Active(WindowId),
    Asked(Vec<String>, PathBuf),
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

struct Pane(Option<WindowId>);

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

    fn set_active(&mut self, active: bool) {
        if let (true, Some(id)) = (active, self.0) {
            record(Seen::Active(id));
        }
    }
}

struct Example;

fn open(cx: &mut Cx<Example>) {
    let window = cx
        .open(Kind::Main, "Example", (200, 120), |_| Pane(None))
        .expect("window");
    if let Some((_, pane)) = cx.window(window) {
        pane.0 = Some(window);
    }
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

    fn open(&mut self, _cx: &mut Cx<Self>, arguments: Vec<String>, directory: &Path) {
        record(Seen::Asked(arguments, directory.to_path_buf()));
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

fn launch_as<P: Program>(
    info: &'static Info,
    arguments: &[&str],
    display: UnixStream,
) -> JoinHandle<Result<(), String>> {
    let arguments = arguments
        .iter()
        .map(|argument| argument.to_string())
        .collect();
    thread::spawn(move || {
        bande_app::run_on::<P>(info, arguments, display).map_err(|error| error.to_string())
    })
}

fn launch(arguments: &[&str], display: UnixStream) -> JoinHandle<Result<(), String>> {
    launch_as::<Example>(&INFO, arguments, display)
}

struct Agent;

impl Program for Agent {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, Error> {
        if !cx.launched_in_background() {
            Agent.reopen(cx);
        }
        Ok(Self)
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        cx.open(Kind::Main, "Agent", (200, 120), |_| Pane(None))
            .expect("window");
    }

    fn open(&mut self, cx: &mut Cx<Self>, _arguments: Vec<String>, _directory: &Path) {
        cx.quit();
    }

    fn runs_in_background() -> bool {
        true
    }
}

static AGENT: Info = Info {
    id: "org.bande.Agent",
    package: "bande-agent",
    version: "1.0.0",
    name: || "Agent".to_owned(),
    keys: Keys::Standard,
    settings: None,
};

#[test]
fn an_app_started_in_the_background_runs_unlisted_until_its_window_opens_and_quits_back_there() {
    let _serial = ONE_BUS_AT_A_TIME
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let _bus = private_bus();
    let mut server = headless(640, 480);
    let running = std::env::temp_dir().join(format!("bande-app-running-{}", std::process::id()));
    let _ = std::fs::remove_file(&running);
    server.note_running_apps_in(running.clone());
    let listed = || {
        std::fs::read_to_string(&running)
            .unwrap_or_default()
            .lines()
            .any(|app| app == AGENT.id)
    };
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let app = launch_as::<Agent>(&AGENT, &["--background"], client_end);
    let mut frames = Frames::start();
    let settled = Instant::now() + Duration::from_millis(1500);
    frames.run_until(&mut server, || Instant::now() > settled);
    assert!(!app.is_finished());
    assert!(!listed());
    let (unused, _) = UnixStream::pair().expect("socket pair");
    let opened = launch_as::<Agent>(&AGENT, &[], unused);
    frames.run_until(&mut server, listed);
    assert_eq!(opened.join().expect("launch"), Ok(()));
    let (unused, _) = UnixStream::pair().expect("socket pair");
    let again = launch_as::<Agent>(&AGENT, &["--background"], unused);
    assert_eq!(again.join().expect("background launch"), Ok(()));

    let (unused, _) = UnixStream::pair().expect("socket pair");
    let quit = launch_as::<Agent>(&AGENT, &["quit"], unused);
    frames.run_until(&mut server, || !listed());
    assert_eq!(quit.join().expect("quit"), Ok(()));
    assert!(!app.is_finished());
    let _ = std::fs::remove_file(&running);
}

#[test]
fn f11_fills_the_screen_and_a_second_launch_reaches_the_running_app_until_quit() {
    let _serial = ONE_BUS_AT_A_TIME
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let _bus = private_bus();
    let (sender, records) = mpsc::channel();
    *RECORD.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
    let mut server = headless(640, 480);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let app = launch(&[], client_end);
    let mut frames = Frames::start();
    let mut seen = Vec::new();
    let mut wait_for = |server: &mut Compositor, done: &dyn Fn(&[Seen]) -> bool| {
        frames.run_until(server, || {
            seen.extend(records.try_iter());
            done(&seen)
        });
    };
    let active_windows = |seen: &[Seen]| {
        let mut active: Vec<_> = seen
            .iter()
            .filter_map(|seen| match seen {
                Seen::Active(window) => Some(*window),
                _ => None,
            })
            .collect();
        active.dedup();
        active.len()
    };

    wait_for(&mut server, &|seen| active_windows(seen) == 1);
    let (unused, _) = UnixStream::pair().expect("socket pair");
    let second_launch = launch(&["notes.txt"], unused);
    let asked = Seen::Asked(
        vec!["notes.txt".to_owned()],
        std::env::current_dir().expect("directory"),
    );
    wait_for(&mut server, &|seen| seen.contains(&asked));
    assert_eq!(second_launch.join().expect("second launch"), Ok(()));

    chord(&mut server, &[KEY_LEFTCTRL, KEY_W]);
    let windowless = Instant::now() + Duration::from_millis(500);
    wait_for(&mut server, &|_| Instant::now() > windowless);
    assert!(!app.is_finished());

    let (unused, _) = UnixStream::pair().expect("socket pair");
    let launched_again = launch(&[], unused);
    wait_for(&mut server, &|seen| active_windows(seen) == 2);
    assert_eq!(launched_again.join().expect("launched again"), Ok(()));
    let full_screen = |server: &Compositor| {
        server
            .windows()
            .iter()
            .any(|window| window.has_state(xdg_toplevel::State::Fullscreen))
    };
    for wanted in [true, false] {
        chord(&mut server, &[KEY_F11]);
        let deadline = Instant::now() + Duration::from_secs(5);
        while full_screen(&server) != wanted {
            assert!(Instant::now() < deadline, "F11 did not toggle full screen");
            let mut stepped = false;
            frames.run_until(&mut server, || std::mem::replace(&mut stepped, true));
        }
    }

    chord(&mut server, &[KEY_LEFTCTRL, KEY_Q]);
    frames.run_until(&mut server, || app.is_finished());
    assert_eq!(app.join().expect("app thread"), Ok(()));
}
