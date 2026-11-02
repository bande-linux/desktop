#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use bande_test_bus::SessionBus;
use bande_test_bus::atspi::{FakeRegistry, ROLE_BUTTON, accessible, find_named};
use bande_test_wayland::{Frames, headless};
use bande_toolkit::ui::{Button, Context, Ui};
use bande_toolkit::{App, Palette, Role, Scene, Scheme, TextSystem, Window, WindowEvent};
use zbus::blocking::connection;

const TIMEOUT: Duration = Duration::from_secs(180);

struct Greeting {
    ui: Ui,
    greeted: Rc<Cell<bool>>,
}

impl App for Greeting {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        if let WindowEvent::Accessibility(request) = event
            && self.ui.perform(&request)
        {
            window.request_redraw();
        }
    }

    fn finished(&self) -> bool {
        self.greeted.get()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

#[test]
fn a_screen_reader_finds_a_button_over_atspi_and_presses_it() {
    let bus = SessionBus::start().expect("private session and AT-SPI bus");
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
        std::env::set_var("AT_SPI_BUS_ADDRESS", bus.address());
    }
    let registry = FakeRegistry::start(&bus).expect("AT-SPI registry");
    let mut server = headless(320, 240);
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let app = thread::spawn(move || {
        let mut window = Window::open_on(
            client_end,
            Role::toplevel("Greeting", "bande-toolkit-test"),
            (200, 120),
        )
        .expect("window");
        let greeted = Rc::new(Cell::new(false));
        let pressed = greeted.clone();
        let mut greeting = Greeting {
            ui: Ui::new(
                Box::new(Button::new("Greet", move || pressed.set(true))),
                Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
            ),
            greeted,
        };
        bande_toolkit::run(&mut window, &mut greeting).expect("run the window");
        greeting.greeted.get()
    });

    let atspi = connection::Builder::address(bus.address())
        .expect("address")
        .method_timeout(Duration::from_secs(2))
        .build()
        .expect("screen reader connection");
    let screen_reader = thread::spawn(move || {
        let root = registry
            .embedded_root(TIMEOUT)
            .expect("the window embeds itself in the registry");
        let started = Instant::now();
        loop {
            let pressed = find_named(&atspi, &root, "Greet").and_then(|button| {
                let role: u32 = accessible(&atspi, &button, "org.a11y.atspi.Accessible")
                    .ok()?
                    .call("GetRole", &())
                    .ok()?;
                let pressed: bool = accessible(&atspi, &button, "org.a11y.atspi.Action")
                    .ok()?
                    .call("DoAction", &(0i32))
                    .ok()?;
                Some((role, pressed))
            });
            if let Some(pressed) = pressed {
                break pressed;
            }
            assert!(started.elapsed() < TIMEOUT, "no Greet button over AT-SPI");
            thread::sleep(Duration::from_millis(50));
        }
    });

    Frames::start().run_until(&mut server, || app.is_finished());
    let (role, pressed) = screen_reader.join().expect("screen reader thread");
    assert_eq!(role, ROLE_BUTTON);
    assert!(pressed, "AT-SPI reports the press as performed");
    assert!(
        app.join().expect("app thread"),
        "the press reached the button's action"
    );
}
