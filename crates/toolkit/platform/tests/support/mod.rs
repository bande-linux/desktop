#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use smithay::utils::{Point, Size};
use unbound_compositor::{FrameClock, FrameScheduler, Headless, REFRESH, Renderer};
use unbound_graphics::Scene;
use unbound_toolkit_platform::{App, Role, Window, WindowEvent, run};
use unbound_widgets::Ui;

const MIN_SSIM: f64 = 0.99;
const OUTPUT: (i32, i32) = (420, 300);

fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    let actual = image::RgbaImage::from_raw(size.0, size.1, rgba).expect("frame");
    let actual = image::DynamicImage::ImageRgba8(actual).to_rgb8();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
    if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().expect("golden directory")).expect("directory");
        actual.save(&path).expect("write golden");
    }
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    actual.save(&target).expect("save the frame");
    let expected = image::open(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .to_rgb8();
    let similarity = image_compare::rgb_hybrid_compare(&expected, &actual).expect("same size");
    assert!(
        similarity.score >= MIN_SSIM,
        "{name} differs from its golden: {:.4} (frame in {})",
        similarity.score,
        target.display()
    );
}

struct Still {
    done: mpsc::Receiver<()>,
    ui: Option<Ui>,
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
        self.at_rest.store(
            !self.ui.as_ref().is_some_and(Ui::is_animating),
            Ordering::SeqCst,
        );
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        if let Some(ui) = &mut self.ui {
            window.drive(ui, event).expect("drive the ui");
        }
    }

    fn animating(&self) -> bool {
        self.ui.as_ref().is_some_and(Ui::is_animating)
    }

    fn deadline(&self) -> Option<Instant> {
        Some(Instant::now() + Duration::from_millis(20))
    }

    fn finished(&self) -> bool {
        self.done.try_recv().is_ok()
    }
}

pub struct Shot {
    pub scale: f64,
    pub dark: bool,
    pub pointer: (i32, i32),
}

pub fn shot(name: &str, prepare: fn(&mut Window) -> Option<Ui>, shot: Shot) {
    let output = (
        (f64::from(OUTPUT.0) * shot.scale) as i32,
        (f64::from(OUTPUT.1) * shot.scale) as i32,
    );
    let mut server = Headless::new(Size::from(output)).expect("headless compositor");
    server.set_output_scale(shot.scale);
    server.set_dark_appearance(shot.dark);
    let pointer = shot.pointer;
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let (drawn, on_screen) = mpsc::channel();
    let (checked, check_done) = mpsc::channel::<()>();
    let at_rest = Arc::new(AtomicBool::new(false));
    let client_at_rest = Arc::clone(&at_rest);
    let client = thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel("Documents", "unbound-toolkit-test"),
            (300, 160),
        )
        .expect("window");
        let mut still = Still {
            done: check_done,
            ui: None,
            at_rest: client_at_rest,
        };
        window.wait_for_gpu();
        let scene = still.scene(&mut window);
        window.draw(&scene).expect("first frame");
        still.ui = prepare(&mut window);
        window.request_redraw();
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
    while Instant::now() < settled || !at_rest.load(Ordering::SeqCst) || server.is_animating() {
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
