#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{Compositor, FrameClock, FrameScheduler, REFRESH, Renderer};
use bande_graphics::{Palette, Point, Scene, Scheme, TextSystem};
use bande_test_bus::SessionBus;
use bande_toolkit_platform::{App, MenuPlacement, Role, Window, WindowEvent};
use bande_widgets::{
    Axis, Button, Cell, Column, Context, Menu, MenuItem, PopUpButton, Row, Stack, Table, TextField,
    Ui,
};
use serde::de::DeserializeOwned;
use smithay::utils::Size;
use zbus::blocking::Connection;
use zbus::zvariant::{DynamicType, OwnedObjectPath, OwnedValue, Value};

const BUS_LAUNCHER: &str = "/usr/libexec/at-spi-bus-launcher";
const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT: &str = "/org/a11y/atspi/accessible/root";
const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const ACTION: &str = "org.a11y.atspi.Action";
const OUTPUT: (i32, i32) = (800, 600);
const WINDOW: (u32, u32) = (420, 360);
const PATIENCE: Duration = Duration::from_secs(30);

struct Launcher {
    process: Child,
    runtime: PathBuf,
}

impl Drop for Launcher {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .arg(self.process.id().to_string())
            .status();
        let _ = self.process.wait();
        let _ = std::fs::remove_dir_all(&self.runtime);
    }
}

fn start_registry(session: &SessionBus) -> (Launcher, String) {
    let runtime = std::env::temp_dir().join(format!("bande-atspi-{}", std::process::id()));
    std::fs::create_dir_all(&runtime).expect("runtime directory");
    let launcher = Launcher {
        process: Command::new(BUS_LAUNCHER)
            .arg("--launch-immediately")
            .env("DBUS_SESSION_BUS_ADDRESS", session.address())
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("GSETTINGS_BACKEND", "memory")
            .spawn()
            .expect("at-spi-bus-launcher from at-spi2-core"),
        runtime,
    };
    let connection = session.connect().expect("session connection");
    let deadline = Instant::now() + PATIENCE;
    let address = loop {
        let reply = connection.call_method(
            Some("org.a11y.Bus"),
            "/org/a11y/bus",
            Some("org.a11y.Bus"),
            "GetAddress",
            &(),
        );
        if let Ok(reply) = reply {
            break reply.body().deserialize::<String>().expect("address");
        }
        assert!(Instant::now() < deadline, "the AT-SPI bus did not start");
        thread::sleep(Duration::from_millis(50));
    };
    connection
        .call_method(
            Some("org.a11y.Bus"),
            "/org/a11y/bus",
            Some("org.freedesktop.DBus.Properties"),
            "Set",
            &("org.a11y.Status", "IsEnabled", Value::from(true)),
        )
        .expect("turn accessibility on");
    (launcher, address)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    bus: String,
    path: OwnedObjectPath,
}

#[derive(Debug, Clone)]
struct Found {
    node: Node,
    role: String,
    name: String,
}

struct Reader {
    bus: Connection,
}

impl Reader {
    fn call<R: DeserializeOwned + zbus::zvariant::Type>(
        &self,
        node: &Node,
        interface: &str,
        method: &str,
        body: &(impl serde::Serialize + DynamicType),
    ) -> zbus::Result<R> {
        let reply = self.bus.call_method(
            Some(node.bus.as_str()),
            node.path.as_str(),
            Some(interface),
            method,
            body,
        )?;
        reply.body().deserialize()
    }

    fn children(&self, node: &Node) -> Vec<Node> {
        self.call::<Vec<(String, OwnedObjectPath)>>(node, ACCESSIBLE, "GetChildren", &())
            .unwrap_or_default()
            .into_iter()
            .map(|(bus, path)| Node { bus, path })
            .collect()
    }

    fn found(&self, node: Node) -> Option<Found> {
        let role = self
            .call::<atspi_common::Role>(&node, ACCESSIBLE, "GetRole", &())
            .ok()?
            .name()
            .to_owned();
        let name = self
            .call::<OwnedValue>(
                &node,
                "org.freedesktop.DBus.Properties",
                "Get",
                &(ACCESSIBLE, "Name"),
            )
            .ok()
            .and_then(|value| String::try_from(value).ok())
            .unwrap_or_default();
        Some(Found { node, role, name })
    }

    fn tree(&self) -> Vec<Found> {
        let root = Node {
            bus: REGISTRY.to_owned(),
            path: OwnedObjectPath::try_from(ROOT).expect("root path"),
        };
        let mut found = Vec::new();
        let mut pending = self.children(&root);
        while let Some(node) = pending.pop() {
            if let Some(entry) = self.found(node.clone()) {
                pending.extend(self.children(&node));
                found.push(entry);
            }
        }
        found
    }

    fn wait_for(&self, role: &str, name: &str) -> Found {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let tree = self.tree();
            if let Some(found) = tree
                .iter()
                .find(|found| found.role == role && found.name == name)
            {
                return found.clone();
            }
            assert!(
                Instant::now() < deadline,
                "no {role} named {name:?} under {:?} in {:?}",
                self.call::<Vec<(String, OwnedObjectPath)>>(
                    &Node {
                        bus: REGISTRY.to_owned(),
                        path: OwnedObjectPath::try_from(ROOT).expect("root path"),
                    },
                    ACCESSIBLE,
                    "GetChildren",
                    &()
                ),
                tree.iter()
                    .map(|found| (&found.role, &found.name))
                    .collect::<Vec<_>>()
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn actions(&self, node: &Node) -> Vec<String> {
        self.call::<Vec<(String, String, String)>>(node, ACTION, "GetActions", &())
            .unwrap_or_default()
            .into_iter()
            .map(|(name, _, _)| name)
            .collect()
    }

    fn act(&self, node: &Node, action: &str) {
        let index = self
            .actions(node)
            .iter()
            .position(|name| name == action)
            .unwrap_or_else(|| panic!("no {action} action on {node:?}"));
        let done: bool = self
            .call(node, ACTION, "DoAction", &(index as i32))
            .expect("do the action");
        assert!(done, "{action} was refused");
    }

    fn wait_for_text(&self, node: &Node, wanted: &str) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let text: String = self
                .call(node, "org.a11y.atspi.Text", "GetText", &(0i32, -1i32))
                .expect("read the text");
            if text == wanted {
                return;
            }
            assert!(Instant::now() < deadline, "the text stayed {text:?}");
            thread::sleep(Duration::from_millis(50));
        }
    }
}

#[derive(Debug, PartialEq)]
enum Heard {
    Saved,
    Typed(String),
    Chose(String),
}

struct Form {
    ui: Ui,
    heard: Sender<Heard>,
    commands: UnixStream,
    finished: bool,
}

fn form(heard: &Sender<Heard>) -> Ui {
    let saved = heard.clone();
    let typed = heard.clone();
    let mut table = Table::new(vec![
        Column::new("Name", 160.0).flexible(),
        Column::new("Size", 80.0).numeric(),
    ]);
    table.set_rows(vec![
        Row::new("alpha", vec![Cell::new("Alpha.txt"), Cell::new("4 KB")]),
        Row::new("beta", vec![Cell::new("Beta.txt"), Cell::new("9 KB")]),
    ]);
    let root = Stack::new(
        Axis::Vertical,
        8.0,
        12.0,
        vec![
            Box::new(Button::new("Save", move || {
                let _ = saved.send(Heard::Saved);
            })),
            Box::new(TextField::new("File name", "", move |text| {
                let _ = typed.send(Heard::Typed(text.to_owned()));
            })),
            Box::new(PopUpButton::new("Format", &["Text", "Markdown"], 0, |_| {})),
            Box::new(table),
        ],
    );
    Ui::new(
        Box::new(root),
        Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light)),
    )
}

impl App for Form {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        if let Some(WindowEvent::MenuAction(action)) =
            window.drive(&mut self.ui, event).expect("drive the ui")
        {
            let _ = self.heard.send(Heard::Chose(action));
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.commands.as_fd()]
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        let mut command = [0u8; 1];
        if self.commands.read(&mut command).unwrap_or(0) == 0 {
            self.finished = true;
            return;
        }
        let menu = Menu::new(vec![
            MenuItem::new("Open", "open").shortcut("Ctrl+O"),
            MenuItem::new("Rename", "rename"),
        ]);
        window
            .show_menu(menu, MenuPlacement::AtPoint(Point::new(40.0, 40.0)))
            .expect("show the menu");
    }

    fn finished(&self) -> bool {
        self.finished
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

fn heard(heard: &Receiver<Heard>, wanted: &Heard) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if heard
            .recv_timeout(Duration::from_millis(100))
            .is_ok_and(|event| event == *wanted)
        {
            return;
        }
    }
    panic!("the app never heard {wanted:?}");
}

#[test]
fn a_screen_reader_reads_and_operates_a_toolkit_window_through_atspi() {
    let session = SessionBus::start().expect("private session bus");
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", session.address());
    }
    let (_launcher, address) = start_registry(&session);
    let mut server = Compositor::new(Size::from(OUTPUT)).expect("headless compositor");
    let mut renderer = Renderer::new().expect("Vulkan renderer");
    let mut scheduler = FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH));
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let (commands, app_commands) = UnixStream::pair().expect("command pipe");
    let (heard_sender, heard_events) = mpsc::channel();
    let app = thread::spawn(move || {
        let role = Role::toplevel("Export", "org.bande.ScreenReaderTest");
        let mut window = Window::open_on(client_end, role, WINDOW).expect("window");
        let mut form = Form {
            ui: form(&heard_sender),
            heard: heard_sender,
            commands: app_commands,
            finished: false,
        };
        bande_toolkit_platform::run(&mut window, &mut form).expect("run loop");
    });
    let reader = thread::spawn(move || {
        let reader = Reader {
            bus: zbus::blocking::connection::Builder::address(address.as_str())
                .expect("accessibility bus address")
                .build()
                .expect("accessibility bus"),
        };
        reader.wait_for("frame", "Export");
        let save = reader.wait_for("button", "Save");
        assert_eq!(reader.actions(&save.node), ["click"]);
        reader.act(&save.node, "click");
        heard(&heard_events, &Heard::Saved);

        let field = reader.wait_for("entry", "");
        let attributes: std::collections::HashMap<String, String> = reader
            .call(&field.node, ACCESSIBLE, "GetAttributes", &())
            .expect("attributes");
        assert_eq!(
            attributes.get("placeholder-text").map(String::as_str),
            Some("File name")
        );
        let written: bool = reader
            .call(
                &field.node,
                "org.a11y.atspi.EditableText",
                "SetTextContents",
                &("notes",),
            )
            .expect("write into the field");
        assert!(written);
        heard(&heard_events, &Heard::Typed("notes".to_owned()));
        reader.wait_for_text(&field.node, "notes");

        reader.wait_for("combo box", "Format");
        let table = reader.wait_for("table", "");
        let tree = reader.tree();
        for cell in ["Alpha.txt", "4 KB", "Beta.txt", "9 KB"] {
            assert!(
                tree.iter()
                    .any(|found| found.role == "table cell" && found.name == cell),
                "no cell {cell:?} under {table:?}"
            );
        }
        for header in ["Name", "Size"] {
            assert!(
                tree.iter()
                    .any(|found| found.role == "column header" && found.name == header),
                "no column header {header:?}"
            );
        }

        let mut commands = commands;
        commands.write_all(b"m").expect("ask for the menu");
        reader.wait_for("menu", "");
        let open = reader.wait_for("menu item", "Open");
        reader.wait_for("menu item", "Rename");
        reader.act(&open.node, "click");
        heard(&heard_events, &Heard::Chose("open".to_owned()));
    });
    let start = Instant::now();
    while !reader.is_finished() {
        server.dispatch().expect("server dispatch");
        scheduler
            .poll(|| start.elapsed(), &mut server, &mut renderer)
            .expect("compositor frame");
        assert!(!app.is_finished(), "the app stopped");
        thread::sleep(Duration::from_millis(2));
    }
    if let Err(panic) = reader.join() {
        std::panic::resume_unwind(panic);
    }
}
