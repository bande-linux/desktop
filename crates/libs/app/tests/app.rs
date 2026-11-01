#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bande_app::{Cx, Error, Info, Keys, Kind, Program, View, WindowId};
use bande_compositor::{Compositor, FrameClock, FrameScheduler, REFRESH, Renderer};
use bande_test_bus::SessionBus;
use bande_toolkit::{Appearance, Scene, Window, WindowEvent};
use smithay::utils::Size;

const KEY_LEFTCTRL: u32 = 29;
const KEY_Q: u32 = 16;
const KEY_W: u32 = 17;

static RECORD: Mutex<Option<Sender<Seen>>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq)]
enum Seen {
    Opened(WindowId),
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
    record(Seen::Opened(window));
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

fn launch(arguments: &[&str], display: UnixStream) -> JoinHandle<Result<(), String>> {
    let arguments = arguments
        .iter()
        .map(|argument| argument.to_string())
        .collect();
    thread::spawn(move || {
        bande_app::run_on::<Example>(&INFO, arguments, display).map_err(|error| error.to_string())
    })
}

#[test]
fn a_second_launch_reaches_the_running_app_which_keeps_running_without_windows_until_quit() {
    let bus = SessionBus::start().expect("private session bus");
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
    }
    let (sender, records) = mpsc::channel();
    *RECORD.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
    let mut server = Compositor::new(Size::from((640, 480))).expect("headless compositor");
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let app = launch(&[], client_end);
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let start = Instant::now();
    let mut seen = Vec::new();
    let mut wait_for = |server: &mut Compositor, done: &dyn Fn(&[Seen]) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            server.dispatch().expect("server dispatch");
            scheduler
                .poll(|| start.elapsed(), server, &mut renderer)
                .expect("compositor frame");
            seen.extend(records.try_iter());
            if done(&seen) {
                break;
            }
            assert!(Instant::now() < deadline, "waited in vain after {seen:?}");
            thread::sleep(Duration::from_millis(1));
        }
    };
    let ctrl = |server: &mut Compositor, key: u32| {
        server.key(KEY_LEFTCTRL, true, 0);
        server.key(key, true, 0);
        server.key(key, false, 0);
        server.key(KEY_LEFTCTRL, false, 0);
    };
    let opened = |seen: &[Seen]| {
        seen.iter()
            .filter_map(|seen| match seen {
                Seen::Opened(window) => Some(*window),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    wait_for(&mut server, &|seen| {
        opened(seen)
            .first()
            .is_some_and(|first| seen.contains(&Seen::Active(*first)))
    });
    let (unused, _) = UnixStream::pair().expect("socket pair");
    let second_launch = launch(&["notes.txt"], unused);
    let asked = Seen::Asked(
        vec!["notes.txt".to_owned()],
        std::env::current_dir().expect("directory"),
    );
    wait_for(&mut server, &|seen| seen.contains(&asked));
    assert_eq!(second_launch.join().expect("second launch"), Ok(()));

    ctrl(&mut server, KEY_W);
    let windowless = Instant::now() + Duration::from_millis(500);
    wait_for(&mut server, &|_| Instant::now() > windowless);
    assert!(!app.is_finished());

    let (unused, _) = UnixStream::pair().expect("socket pair");
    let launched_again = launch(&[], unused);
    wait_for(&mut server, &|seen| {
        opened(seen)
            .get(1)
            .is_some_and(|reopened| seen.contains(&Seen::Active(*reopened)))
    });
    assert_eq!(launched_again.join().expect("launched again"), Ok(()));

    ctrl(&mut server, KEY_Q);
    let deadline = Instant::now() + Duration::from_secs(60);
    while !app.is_finished() {
        wait_for(&mut server, &|_| true);
        assert!(Instant::now() < deadline, "Ctrl+Q did not quit");
    }
    assert_eq!(app.join().expect("app thread"), Ok(()));
}
