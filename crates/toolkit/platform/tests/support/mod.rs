#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use smithay::utils::{Point, Size};
use unbound_compositor::{FrameClock, FrameScheduler, Headless, REFRESH, Renderer};
use unbound_graphics::Scene;
use unbound_test_bus::SessionBus;
use unbound_toolkit_platform::{App, RENDERER_VARIABLE, Role, Window, WindowEvent, run};
use unbound_widgets::Ui;
use zbus::zvariant::{OwnedValue, Value};

const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const APPEARANCE: &str = "org.freedesktop.appearance";

static DARK: AtomicBool = AtomicBool::new(false);

struct Settings;

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Settings {
    fn read_all(&self, _namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let scheme = if DARK.load(Ordering::SeqCst) { 1u32 } else { 2 };
        let appearance = HashMap::from([(
            "color-scheme".to_owned(),
            OwnedValue::try_from(Value::from(scheme)).expect("owned value"),
        )]);
        HashMap::from([(APPEARANCE.to_owned(), appearance)])
    }
}

pub struct Portal {
    _bus: SessionBus,
    _connection: zbus::blocking::Connection,
}

pub fn appearance_portal() -> Portal {
    let bus = SessionBus::start().expect("private session bus");
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
    }
    let connection = bus.connect().expect("portal connection");
    connection
        .object_server()
        .at(PORTAL_PATH, Settings)
        .expect("serve the settings");
    connection
        .request_name("org.freedesktop.portal.Desktop")
        .expect("own the portal name");
    Portal {
        _bus: bus,
        _connection: connection,
    }
}

const OUTPUT: (i32, i32) = (480, 340);
const SHOWN_AT_REST: u32 = 3;

fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    unbound_golden::assert_golden!(name, &unbound_golden::from_rgba(size.0, size.1, rgba));
}

struct Still {
    done: mpsc::Receiver<()>,
    ui: Option<Ui>,
    frames_at_rest: u32,
    at_rest: Arc<AtomicBool>,
}

impl App for Still {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let scene = match &mut self.ui {
            Some(ui) => ui.scene(width, height),
            None => Scene {
                width,
                height,
                background: window.appearance().palette().sheet,
                items: Vec::new(),
            },
        };
        self.frames_at_rest = if self.ui.as_ref().is_some_and(Ui::is_animating) {
            0
        } else {
            self.frames_at_rest + 1
        };
        self.at_rest
            .store(self.frames_at_rest > SHOWN_AT_REST, Ordering::SeqCst);
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        if let Some(ui) = &mut self.ui {
            window.drive(ui, event).expect("drive the ui");
        }
    }

    fn animating(&self) -> bool {
        self.frames_at_rest <= SHOWN_AT_REST
    }

    fn deadline(&self) -> Option<Instant> {
        Some(Instant::now() + Duration::from_millis(20))
    }

    fn finished(&self) -> bool {
        self.done.try_recv().is_ok()
    }
}

pub struct Shot {
    pub title: &'static str,
    pub companion: Option<&'static str>,
    pub dark: bool,
    pub pointer: (i32, i32),
    pub popups: usize,
}

fn draw_on_the_gpu() {
    static FORCED: Once = Once::new();
    FORCED.call_once(|| unsafe { std::env::set_var(RENDERER_VARIABLE, "gpu") });
}

pub fn shot(name: &str, prepare: fn(&mut Window) -> Option<Ui>, shot: Shot) {
    draw_on_the_gpu();
    let mut server = Headless::new(Size::from(OUTPUT)).expect("headless compositor");
    server.set_dark_appearance(shot.dark);
    DARK.store(shot.dark, Ordering::SeqCst);
    let pointer = shot.pointer;
    let title = shot.title;
    let companion = shot.companion;
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let (drawn, on_screen) = mpsc::channel();
    let (checked, check_done) = mpsc::channel::<()>();
    let at_rest = Arc::new(AtomicBool::new(false));
    let client_at_rest = Arc::clone(&at_rest);
    let client = thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel(title, "unbound-toolkit-test"),
            (360, 200),
        )
        .expect("window");
        let mut still = Still {
            done: check_done,
            ui: None,
            frames_at_rest: 0,
            at_rest: client_at_rest,
        };
        window.wait_for_gpu();
        let scene = still.scene(&mut window);
        window.draw(scene).expect("first frame");
        still.ui = prepare(&mut window);
        still.frames_at_rest = 0;
        still.at_rest.store(false, Ordering::SeqCst);
        window.request_redraw();
        let _companion = companion.map(|title| {
            let mut beside = window
                .open_beside(
                    Role::toplevel(title, "unbound-toolkit-companion"),
                    (240, 120),
                )
                .expect("second window");
            beside.wait_for_gpu();
            let scene = Scene {
                width: 240,
                height: 120,
                background: beside.appearance().palette().sheet,
                items: Vec::new(),
            };
            for _ in 0..3 {
                beside.draw(scene.clone()).expect("second window's frame");
                beside.wait_for_frame().expect("second window shown");
            }
            beside
        });
        drawn.send(()).expect("report the frame");
        run(&mut window, &mut still).expect("run loop");
    });

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let start = Instant::now();
    let pump = |server: &mut Headless, scheduler: &mut FrameScheduler, renderer: &mut Renderer| {
        server.dispatch().expect("server dispatch");
        scheduler
            .poll(|| start.elapsed(), server, renderer)
            .expect("compositor frame");
        thread::sleep(Duration::from_millis(2));
    };
    while on_screen.try_recv().is_err() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the client stalled"
        );
        pump(&mut server, &mut scheduler, &mut renderer);
    }
    let mut pointed = None;
    let settled = Instant::now() + Duration::from_millis(700);
    while Instant::now() < settled
        || !at_rest.load(Ordering::SeqCst)
        || server.is_animating()
        || server.popup_count() != shot.popups
    {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the scene never came to rest"
        );
        let at = server.windows()[0].location();
        if pointed != Some(at) {
            pointed = Some(at);
            for step in [1, 0] {
                server.move_pointer_to(
                    Point::from((
                        f64::from(at.x + pointer.0 - step),
                        f64::from(at.y + pointer.1 - step),
                    )),
                    0,
                );
                pump(&mut server, &mut scheduler, &mut renderer);
            }
        }
        pump(&mut server, &mut scheduler, &mut renderer);
    }
    let presented = Instant::now() + Duration::from_millis(100);
    while Instant::now() < presented {
        pump(&mut server, &mut scheduler, &mut renderer);
    }
    let snapshot = renderer.snapshot(&server).expect("read back");
    checked.send(()).expect("release the client");
    while !client.is_finished() {
        pump(&mut server, &mut scheduler, &mut renderer);
    }
    client.join().expect("client thread");
    check_golden(
        name,
        (snapshot.size.w as u32, snapshot.size.h as u32),
        snapshot.rgba,
    );
}
