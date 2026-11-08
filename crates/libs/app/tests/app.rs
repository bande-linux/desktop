#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bande_app::{Cx, Error, Info, Keys, Kind, Program, View};
use bande_compositor::Compositor;
use bande_test_bus::SessionBus;
use bande_test_wayland::keys::{KEY_LEFTCTRL, KEY_Q, KEY_W};
use bande_test_wayland::{Frames, chord, headless};
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
fn a_second_launch_reaches_the_running_app_which_outlives_its_windows_until_quit() {
    let bus = SessionBus::start().expect("private session bus");
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

    chord(&mut server, &[KEY_LEFTCTRL, KEY_Q]);
    frames.run_until(&mut server, || app.is_finished());
    assert_eq!(app.join().expect("app thread"), Ok(()));
}
