use std::os::fd::BorrowedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bande_bar::surfaces::{self, Display, Services};
use bande_compositor::{Compositor, FrameClock, FrameScheduler, REFRESH, Renderer, SessionRole};
use bande_config::Config;
use bande_notifications::Notifications;
use bande_test_wayland::{RED, ShmImage, TestClient};
use bande_toolkit::{App, Scene, Surfaces, TextSystem, Window, WindowEvent};
use smithay::utils::Size;
use wayland_client::protocol::wl_shm::Format;

const OUTPUT: (i32, i32) = (1024, 720);
const WINDOW: i32 = 64;

pub fn install_apps(name: &str) -> PathBuf {
    let home = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&home);
    let applications = home.join("apps/applications");
    std::fs::create_dir_all(&applications).expect("applications folder");
    unsafe {
        std::env::set_var("XDG_DATA_HOME", home.join("home"));
        std::env::set_var("XDG_CONFIG_HOME", home.join("config"));
        std::env::set_var("XDG_DATA_DIRS", home.join("apps"));
    }
    applications
}

pub fn write_entry(applications: &Path, app_id: &str, name: &str, exec: &str) {
    std::fs::write(
        applications.join(format!("{app_id}.desktop")),
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\n"),
    )
    .expect("desktop entry");
}

struct Shell {
    windows: Vec<Window>,
    apps: Vec<Box<dyn App>>,
    labels: Sender<Vec<String>>,
    _text: TextSystem,
}

impl Surfaces for Shell {
    fn windows(&self) -> &[Window] {
        &self.windows
    }

    fn windows_mut(&mut self) -> &mut [Window] {
        &mut self.windows
    }

    fn scene(&mut self, index: usize) -> Scene {
        let scene = self.apps[index].scene(&mut self.windows[index]);
        if let Some(tree) = self.apps[index].accessibility().filter(|_| index == 1) {
            let labels = tree
                .nodes
                .iter()
                .filter_map(|(_, node)| node.label().map(str::to_owned))
                .collect();
            let _ = self.labels.send(labels);
        }
        scene
    }

    fn event(&mut self, index: usize, event: WindowEvent) {
        self.apps[index].event(&mut self.windows[index], event);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.apps.iter().flat_map(|app| app.sources()).collect()
    }

    fn source_ready(&mut self, mut index: usize) {
        for (part, app) in self.apps.iter_mut().enumerate() {
            let count = app.sources().len();
            if index < count {
                app.source_ready(&mut self.windows[part], index);
                return;
            }
            index -= count;
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.apps.iter().filter_map(|app| app.deadline()).min()
    }

    fn tick(&mut self, now: Instant) {
        for (app, window) in self.apps.iter_mut().zip(&mut self.windows) {
            if app.deadline().is_some_and(|deadline| deadline <= now) {
                app.tick(window, now);
            }
        }
    }

    fn animating(&self, index: usize) -> bool {
        self.apps[index].animating()
    }
}

fn shell(stream: UnixStream, labels: Sender<Vec<String>>) {
    let text = TextSystem::new();
    let mut root =
        Window::open_on(stream, bande_notifications::role(), (0, 0)).expect("notifications");
    root.share_fonts(&text);
    let notifications =
        Notifications::new(&mut root, text.sharing_fonts(), None).expect("notifications");
    let services = Services {
        text: &text,
        session: None,
        system: None,
        config_home: None,
        home: None,
        notification_center: Rc::new(notifications.toggle()),
    };
    let display = Display {
        output: "HEADLESS-1".to_owned(),
        main: true,
    };
    let mut config = Config::default();
    config.bar.apps = Vec::new();
    let bar = surfaces::open(&root, &display, &config, &services).expect("bar");
    let mut windows = vec![root];
    let mut apps: Vec<Box<dyn App>> = vec![Box::new(notifications)];
    for (window, app) in bar.surfaces {
        windows.push(window);
        apps.push(app);
    }
    let mut shell = Shell {
        windows,
        apps,
        labels,
        _text: text,
    };
    bande_toolkit::run_surfaces(&mut shell).expect("run loop");
}

pub struct Desktop {
    server: Compositor,
    renderer: Renderer,
    scheduler: FrameScheduler,
    start: Instant,
    label_updates: Receiver<Vec<String>>,
    labels: Vec<String>,
    shell: JoinHandle<()>,
    client: TestClient,
    windows: usize,
}

impl Desktop {
    pub fn new() -> Self {
        let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
        let (client_end, server_end) = UnixStream::pair().expect("socket pair");
        server
            .insert_session_client(server_end, SessionRole::Shell)
            .expect("insert the shell");
        let (labels, label_updates) = mpsc::channel();
        let shell = thread::spawn(move || shell(client_end, labels));
        let mut client = TestClient::connect(&mut server);
        client.use_seat(&mut server);
        Self {
            server,
            renderer: Renderer::new().expect("renderer"),
            scheduler: FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH)),
            start: Instant::now(),
            label_updates,
            labels: Vec::new(),
            shell,
            client,
            windows: 0,
        }
    }

    pub fn open_window(&mut self, app_id: &str) {
        let pixels = RED.repeat((WINDOW * WINDOW) as usize);
        self.client.show_window(
            &mut self.server,
            &ShmImage {
                width: WINDOW,
                height: WINDOW,
                stride: WINDOW * 4,
                format: Format::Xrgb8888,
                bytes: &pixels,
            },
        );
        self.client
            .set_app_id(&mut self.server, self.windows, app_id);
        self.windows += 1;
    }

    fn pump(&mut self) {
        self.server.dispatch().expect("server dispatch");
        let start = self.start;
        self.scheduler
            .poll(|| start.elapsed(), &mut self.server, &mut self.renderer)
            .expect("compositor frame");
        self.client.exchange(&mut self.server);
        if let Some(labels) = self.label_updates.try_iter().last() {
            self.labels = labels;
        }
        assert!(!self.shell.is_finished(), "the shell stopped");
        thread::sleep(Duration::from_micros(300));
    }

    pub fn wait_for_label(&mut self, label: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.labels.iter().any(|known| known == label) {
            self.pump();
            assert!(
                Instant::now() < deadline,
                "no {label:?} in {:?}",
                self.labels
            );
        }
    }
}
