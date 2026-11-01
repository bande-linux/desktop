#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use bande_compositor::{Compositor, FrameClock, FrameScheduler, REFRESH, Renderer};
use bande_test_bus::SessionBus;
use bande_toolkit::ui::{Button, Context, Ui};
use bande_toolkit::{App, Palette, Role, Scene, Scheme, TextSystem, Window, WindowEvent};
use smithay::utils::Size;
use zbus::blocking::{Connection, Proxy, connection, proxy};
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

const OUTPUT: (i32, i32) = (320, 240);
const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT_PATH: &str = "/org/a11y/atspi/accessible/root";
const TIMEOUT: Duration = Duration::from_secs(180);
const CALL_TIMEOUT: Duration = Duration::from_secs(2);
const ATSPI_ROLE_BUTTON: u32 = 43;

type ObjectRef = (String, OwnedObjectPath);

struct Status;

#[zbus::interface(name = "org.a11y.Status")]
impl Status {
    #[zbus(property)]
    fn is_enabled(&self) -> bool {
        true
    }
}

struct Registry {
    embedded: mpsc::Sender<String>,
    unique_name: String,
}

#[zbus::interface(name = "org.a11y.atspi.Socket")]
impl Registry {
    fn embed(&self, plug: ObjectRef) -> ObjectRef {
        let _ = self.embedded.send(plug.0);
        (
            self.unique_name.clone(),
            OwnedObjectPath::try_from(ROOT_PATH).expect("registry root path"),
        )
    }
}

fn serve_atspi(bus: &SessionBus) -> (Connection, Connection, mpsc::Receiver<String>) {
    let status = bus.connect().expect("status connection");
    status
        .object_server()
        .at("/org/a11y/bus", Status)
        .expect("serve the AT-SPI status");
    status
        .request_name("org.a11y.Bus")
        .expect("own the AT-SPI bus name");
    let (embedded, plugs) = mpsc::channel();
    let registry = bus.connect().expect("registry connection");
    registry
        .object_server()
        .at(
            ROOT_PATH,
            Registry {
                embedded,
                unique_name: registry
                    .unique_name()
                    .expect("registry unique name")
                    .to_string(),
            },
        )
        .expect("serve the registry socket");
    registry
        .request_name(REGISTRY)
        .expect("own the registry name");
    (status, registry, plugs)
}

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

fn accessible<'a>(atspi: &'a Connection, object: &'a ObjectRef, interface: &'a str) -> Proxy<'a> {
    proxy::Builder::new(atspi)
        .destination(object.0.as_str())
        .expect("destination")
        .path(object.1.as_str())
        .expect("path")
        .interface(interface)
        .expect("interface")
        .cache_properties(CacheProperties::No)
        .build()
        .expect("proxy")
}

fn find_named(atspi: &Connection, object: &ObjectRef, name: &str) -> Option<ObjectRef> {
    let proxy = accessible(atspi, object, "org.a11y.atspi.Accessible");
    if proxy.get_property::<String>("Name").ok()? == name {
        return Some(object.clone());
    }
    let children: Vec<ObjectRef> = proxy.call("GetChildren", &()).ok()?;
    children
        .iter()
        .find_map(|child| find_named(atspi, child, name))
}

#[test]
fn a_screen_reader_finds_a_button_over_atspi_and_presses_it() {
    let bus = SessionBus::start().expect("private session and AT-SPI bus");
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
        std::env::set_var("AT_SPI_BUS_ADDRESS", bus.address());
    }
    let (_status, _registry, plugs) = serve_atspi(&bus);

    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
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
        .method_timeout(CALL_TIMEOUT)
        .build()
        .expect("screen reader connection");
    let screen_reader = thread::spawn(move || {
        let app_name = plugs
            .recv_timeout(TIMEOUT)
            .expect("the window embeds itself in the registry");
        let root = (
            app_name,
            OwnedObjectPath::try_from(ROOT_PATH).expect("root path"),
        );
        let started = Instant::now();
        loop {
            let pressed = find_named(&atspi, &root, "Greet").and_then(|button| {
                let role: u32 = accessible(&atspi, &button, "org.a11y.atspi.Accessible")
                    .call("GetRole", &())
                    .ok()?;
                let pressed: bool = accessible(&atspi, &button, "org.a11y.atspi.Action")
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

    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let start = Instant::now();
    while !app.is_finished() {
        assert!(start.elapsed() < TIMEOUT, "the window was never pressed");
        server.dispatch().expect("server dispatch");
        scheduler
            .poll(|| start.elapsed(), &mut server, &mut renderer)
            .expect("compositor frame");
        thread::sleep(Duration::from_millis(1));
    }

    let (role, pressed) = screen_reader.join().expect("screen reader thread");
    assert_eq!(role, ATSPI_ROLE_BUTTON);
    assert!(pressed, "AT-SPI reports the press as performed");
    assert!(
        app.join().expect("app thread"),
        "the press reached the button's action"
    );
}
