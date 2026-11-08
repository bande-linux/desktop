#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bande_app::{Cx, Error, Info, Keys, Kind, Program, Saved, View, WindowId};
use bande_compositor::Compositor;
use bande_test_bus::SessionBus;
use bande_test_wayland::{Frames, chord, headless};
use bande_toolkit::{Appearance, Scene, Window, WindowEvent};

const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTMETA: u32 = 125;
const KEY_LEFT: u32 = 105;
const KEY_Q: u32 = 16;

static RESTORED: Mutex<Option<Sender<String>>> = Mutex::new(None);

struct Note(String);

impl View for Note {
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

struct Notes;

fn open(cx: &mut Cx<Notes>, text: &str) {
    let text = text.to_owned();
    cx.open(Kind::Main, "Note", (300, 200), |_| Note(text))
        .expect("window");
}

impl Program for Notes {
    type View = Note;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, Error> {
        for text in arguments {
            open(cx, &text);
        }
        Ok(Self)
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        open(cx, "fresh");
    }

    fn save_window(&self, cx: &Cx<Self>, window: WindowId) -> Option<Saved> {
        Saved::of(&cx.view(window)?.0)
    }

    fn restore_window(&mut self, cx: &mut Cx<Self>, saved: Saved) {
        let text: String = saved.read().expect("saved text");
        if let Some(restored) = RESTORED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            let _ = restored.send(text.clone());
        }
        open(cx, &text);
    }
}

static INFO: Info = Info {
    id: "org.bande.Notes",
    package: "bande-notes",
    version: "1.0.0",
    name: || "Notes".to_owned(),
    keys: Keys::Standard,
    settings: None,
};

fn launch(server: &mut Compositor, arguments: &[&str]) -> JoinHandle<Result<(), String>> {
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let arguments = arguments.iter().map(ToString::to_string).collect();
    thread::spawn(move || {
        bande_app::run_on::<Notes>(&INFO, arguments, client_end).map_err(|error| error.to_string())
    })
}

fn wait(frames: &mut Frames, server: &mut Compositor, mut done: impl FnMut(&Compositor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut stepped = false;
        frames.run_until(server, || std::mem::replace(&mut stepped, true));
        if done(server) {
            return;
        }
        assert!(Instant::now() < deadline, "waited in vain");
    }
}

fn restored_texts(restored: &Receiver<String>) -> Vec<String> {
    let mut texts: Vec<String> = restored.try_iter().collect();
    texts.sort();
    texts
}

#[test]
fn windows_come_back_after_a_crash_and_not_after_quit() {
    let bus = SessionBus::start().expect("private session bus");
    let state = std::env::temp_dir().join(format!("bande-resume-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&state);
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
        std::env::set_var("XDG_STATE_HOME", &state);
    }
    let (sender, restored) = mpsc::channel();
    *RESTORED.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
    let places: PathBuf = state.join("window-sessions");
    let saved = state.join("bande/resume").join(INFO.id);

    let mut server = headless(1000, 700);
    server.remember_sessions_in(places.clone());
    let mut frames = Frames::start();
    let app = launch(&mut server, &["first", "second"]);
    wait(&mut frames, &mut server, |server| {
        server
            .windows()
            .iter()
            .filter(|w| w.geometry().is_some())
            .count()
            == 2
    });
    chord(&mut server, &[KEY_LEFTMETA, KEY_LEFT]);
    wait(&mut frames, &mut server, |server| {
        server
            .windows()
            .iter()
            .any(|window| window.tile().is_some())
    });
    let tiled_before = server.windows().iter().find_map(|window| window.tile());
    wait(&mut frames, &mut server, |_| {
        let notes = std::fs::read_to_string(&saved).unwrap_or_default();
        let placed = std::fs::read_to_string(&places).unwrap_or_default();
        notes.contains("first") && notes.contains("second") && placed.contains("\"tile\":\"Left\"")
    });

    drop(frames);
    drop(server);
    assert!(
        app.join().expect("app thread").is_err(),
        "the app lost its display"
    );
    assert!(saved.exists(), "a lost display keeps the windows");

    let mut server = headless(1000, 700);
    server.remember_sessions_in(places.clone());
    let mut frames = Frames::start();
    let app = launch(&mut server, &[]);
    wait(&mut frames, &mut server, |server| {
        server
            .windows()
            .iter()
            .filter(|w| w.geometry().is_some())
            .count()
            == 2
    });
    assert_eq!(restored_texts(&restored), ["first", "second"]);
    let tiled_after = server.windows().iter().find_map(|window| window.tile());
    assert_eq!(tiled_after, tiled_before);

    chord(&mut server, &[KEY_LEFTCTRL, KEY_Q]);
    frames.run_until(&mut server, || app.is_finished());
    assert_eq!(app.join().expect("app thread"), Ok(()));
    assert!(!saved.exists(), "Quit is a decision to close the windows");

    let app = launch(&mut server, &[]);
    wait(&mut frames, &mut server, |server| {
        server
            .windows()
            .iter()
            .filter(|w| w.geometry().is_some())
            .count()
            == 1
    });
    assert!(restored.try_recv().is_err());
    chord(&mut server, &[KEY_LEFTCTRL, KEY_Q]);
    frames.run_until(&mut server, || app.is_finished());
    assert_eq!(app.join().expect("app thread"), Ok(()));
    let _ = std::fs::remove_dir_all(&state);
}
