#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{self, PipeWriter, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::thread;

use bande_test_bus::SessionBus;
use bande_test_wayland::{Frames, headless};
use bande_toolkit::{App, Role, Scene, Window, WindowEvent};

struct Source {
    name: &'static str,
    reader: OwnedFd,
    writer: PipeWriter,
}

impl Source {
    fn new(name: &'static str) -> Self {
        let (reader, writer) = io::pipe().expect("pipe");
        Self {
            name,
            reader: reader.into(),
            writer,
        }
    }

    fn ready(name: &'static str) -> Self {
        let mut source = Self::new(name);
        source.writer.write_all(&[1]).expect("make it ready");
        source
    }
}

struct Changing {
    sources: Vec<Source>,
    handled: Vec<&'static str>,
}

impl App for Changing {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        Scene {
            width,
            height,
            background: window.appearance().palette().sheet,
            items: Vec::new(),
        }
    }

    fn event(&mut self, _window: &mut Window, _event: WindowEvent) {}

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.sources
            .iter()
            .map(|source| source.reader.as_fd())
            .collect()
    }

    fn source_ready(&mut self, _window: &mut Window, index: usize) {
        let source = self.sources.remove(index);
        self.handled.push(source.name);
        if source.name == "first" {
            let quiet = Source::new("replacement");
            let replaced = &mut self.sources[0];
            rustix::io::dup2(&quiet.reader, &mut replaced.reader).expect("use its number again");
            replaced.writer = quiet.writer;
            replaced.name = quiet.name;
            self.sources.insert(0, Source::new("added"));
            self.sources.insert(0, Source::new("added too"));
        }
    }

    fn finished(&self) -> bool {
        self.handled.contains(&"last")
    }

    fn animating(&self) -> bool {
        false
    }
}

#[test]
fn only_what_was_ready_is_handled_after_the_sources_change() {
    let bus = SessionBus::start().expect("private session bus");
    // SAFETY: this binary's only test sets this before it starts the window or the compositor, so
    // no other thread reads the environment yet.
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
    }
    let mut server = headless(640, 480);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let app = thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel("Sources", "bande-toolkit-test"),
            (240, 120),
        )
        .expect("window");
        let mut changing = Changing {
            sources: ["first", "replaced", "last"].map(Source::ready).into(),
            handled: Vec::new(),
        };
        bande_toolkit::run(&mut window, &mut changing).expect("run the window");
        changing.handled
    });
    Frames::start().run_until(&mut server, || app.is_finished());
    assert_eq!(app.join().expect("app thread"), ["first", "last"]);
}
