use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use smithay::utils::Size;
use unbound_compositor::{
    FrameClock, FrameScheduler, Headless, OutputConfig, REFRESH, Renderer, SessionRole,
};

use super::*;

const KEY_LEFTMETA: u32 = 125;
const KEY_SPACE: u32 = 57;
const TIMEOUT: Duration = Duration::from_secs(30);

struct Session {
    server: Headless,
    renderer: Renderer,
    scheduler: FrameScheduler,
    start: Instant,
    _config: tempfile::TempDir,
}

impl Session {
    fn start(displays: &[OutputConfig]) -> (Self, Receiver<()>) {
        let config = tempfile::tempdir().unwrap();
        let home = config.path().to_path_buf();
        let mut server = Headless::new(Size::from((1280, 800))).unwrap();
        for display in displays {
            server.add_output(display.clone());
        }
        let (client, server_end) = UnixStream::pair().unwrap();
        server
            .insert_session_client(server_end, SessionRole::Shell)
            .unwrap();
        let (rooted, root_opened) = mpsc::channel();
        thread::spawn(move || {
            let root = Window::open_on(client, root_role(), (0, 0)).unwrap();
            let _ = rooted.send(());
            let context = Context {
                text: TextSystem::new(),
                session: None,
                system: None,
                config_home: Some(home.clone()),
                home: Some(home),
                notification_center: Rc::new(RefCell::new(Box::new(|| {}))),
            };
            let mut shell = Shell::start(context, root).unwrap();
            let _ = unbound_toolkit::run_surfaces(&mut shell);
        });
        let session = Session {
            server,
            renderer: Renderer::new().unwrap(),
            scheduler: FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH)),
            start: Instant::now(),
            _config: config,
        };
        (session, root_opened)
    }

    fn pump(&mut self) {
        self.server.dispatch().unwrap();
        let start = self.start;
        self.scheduler
            .poll(|| start.elapsed(), &mut self.server, &mut self.renderer)
            .unwrap();
        thread::sleep(Duration::from_millis(1));
    }

    fn pump_until(&mut self, what: &str, mut done: impl FnMut(&Headless) -> bool) {
        while !done(&self.server) {
            assert!(
                self.start.elapsed() < TIMEOUT,
                "timed out waiting for {what}"
            );
            self.pump();
        }
    }
}

#[test]
fn startup_maps_every_shell_surface_once_on_every_display() {
    let side = OutputConfig::virtual_display("SIDE-1", Size::from((800, 600)));
    let (mut session, _) = Session::start(&[side]);
    let mut seen: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    let mut record = |server: &Headless| {
        for (namespace, id) in server.mapped_layers() {
            seen.entry(namespace).or_default().insert(id);
        }
    };
    let on_every_display = |server: &Headless| {
        let layers = server.mapped_layers();
        ["desktop", "bar"].iter().all(|wanted| {
            layers
                .iter()
                .filter(|(namespace, _)| namespace == wanted)
                .count()
                == 2
        })
    };
    while !on_every_display(&session.server) {
        assert!(session.start.elapsed() < TIMEOUT, "the shell never showed");
        session.pump();
        record(&session.server);
    }
    let settled = Instant::now();
    while settled.elapsed() < Duration::from_secs(2) {
        session.pump();
        record(&session.server);
    }

    let mut shown: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for (namespace, id) in session.server.mapped_layers() {
        shown.entry(namespace).or_default().insert(id);
    }
    assert_eq!(seen, shown);
}

#[test]
fn super_space_pressed_while_the_shell_starts_opens_the_launcher() {
    let (mut session, root_opened) = Session::start(&[]);
    session.pump_until("the first shell surface", |_| {
        root_opened.try_recv().is_ok()
    });
    for (key, pressed) in [
        (KEY_LEFTMETA, true),
        (KEY_SPACE, true),
        (KEY_SPACE, false),
        (KEY_LEFTMETA, false),
    ] {
        session.server.key(key, pressed, 0);
    }

    session.pump_until("the launcher", |server| {
        server
            .mapped_layers()
            .iter()
            .any(|(namespace, _)| namespace == "launcher")
    });
}
