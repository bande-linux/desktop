#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread::{self, JoinHandle};

use bande_app::{Cx, Error, Info, Keys, Kind, Program, View};
use bande_test_bus::SessionBus;
use bande_test_wayland::keys::{KEY_LEFTCTRL, KEY_W};
use bande_test_wayland::{Frames, chord, headless};
use bande_toolkit::{Appearance, Scene, Window, WindowEvent};

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

struct Service;

impl Program for Service {
    type View = Pane;

    fn start(_cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, Error> {
        Ok(Self)
    }

    fn reopen(&mut self, _cx: &mut Cx<Self>) {}

    fn open(&mut self, cx: &mut Cx<Self>, _arguments: Vec<String>, _directory: &Path) {
        cx.open(Kind::Popover("word"), "word", (200, 120), |_| Pane)
            .expect("window");
    }

    fn waits_for_activation() -> bool {
        true
    }
}

static INFO: Info = Info {
    id: "org.bande.PopoverExample",
    package: "bande-popover-example",
    version: "1.2.3",
    name: || "Example".to_owned(),
    keys: Keys::Standard,
    settings: None,
};

fn launch(arguments: &[&str], display: UnixStream) -> JoinHandle<Result<(), String>> {
    let arguments = arguments.iter().map(ToString::to_string).collect();
    thread::spawn(move || {
        bande_app::run_on::<Service>(&INFO, arguments, display).map_err(|error| error.to_string())
    })
}

#[test]
fn an_activated_service_waits_shows_a_popover_at_the_pointer_and_exits_when_it_closes() {
    let bus = SessionBus::start().expect("private session bus");
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
        std::env::set_var(
            "XDG_STATE_HOME",
            std::env::temp_dir().join("bande-popover-state"),
        );
    }
    let mut server = headless(640, 480);
    server.move_pointer_to((300.0, 200.0).into(), 0);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let service = launch(&[], client_end);
    let mut frames = Frames::start();
    let mut turns = 0;
    frames.run_until(&mut server, || {
        turns += 1;
        turns > 30
    });
    assert!(!service.is_finished());
    assert!(server.windows().is_empty());

    let (unused, _) = UnixStream::pair().expect("socket pair");
    assert_eq!(launch(&["word"], unused).join().expect("request"), Ok(()));
    let mapped = |server: &bande_compositor::Compositor| {
        server
            .windows()
            .iter()
            .find_map(|window| window.geometry())
            .is_some()
    };
    while !mapped(&server) {
        let mut stepped = false;
        frames.run_until(&mut server, || std::mem::replace(&mut stepped, true));
    }
    let at = server.windows()[0].geometry().expect("mapped").loc;
    assert_eq!((at.x, at.y), (300, 200));

    chord(&mut server, &[KEY_LEFTCTRL, KEY_W]);
    frames.run_until(&mut server, || service.is_finished());
    assert_eq!(service.join().expect("service"), Ok(()));
}
