use std::io::Read;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use smithay::utils::{Point, Size};
use unbound_compositor::{FrameClock, FrameScheduler, Headless, REFRESH, Renderer, SessionRole};
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{
    App, Color, Edges, KeyboardFocus, LayerKind, LayerOptions, Role, Scene, Window, WindowEvent,
};

const OUTPUT: (i32, i32) = (320, 240);
const WINDOW: (u32, u32) = (120, 80);
const KEY_A: u32 = 30;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTCTRL: u32 = 29;
const KEY_C: u32 = 46;
const KEY_LEFTMETA: u32 = 125;
const KEY_SPACE: u32 = 57;
const BTN_LEFT: u32 = 0x110;
const BUSY: Duration = Duration::from_millis(900);

struct Forwarder {
    events: Sender<WindowEvent>,
    commands: UnixStream,
    finished: bool,
}

impl App for Forwarder {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        Scene {
            width,
            height,
            background: Color::hex(0x3070D0),
            items: Vec::new(),
        }
    }

    fn event(&mut self, _window: &mut Window, event: WindowEvent) {
        let _ = self.events.send(event);
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
        match command[0] {
            b'c' => window.copy("copied from the toolkit"),
            b'p' => window.paste().expect("paste"),
            b'h' => window.hide().expect("hide"),
            b's' => window.show().expect("show"),
            b'b' => thread::sleep(BUSY),
            _ => self.finished = true,
        }
    }

    fn finished(&self) -> bool {
        self.finished
    }
}

struct Client {
    events: Receiver<WindowEvent>,
    commands: UnixStream,
    thread: Option<JoinHandle<()>>,
    seen: Vec<WindowEvent>,
}

impl Client {
    fn send(&mut self, command: u8) {
        std::io::Write::write_all(&mut self.commands, &[command]).expect("command");
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.commands.shutdown(std::net::Shutdown::Both);
    }
}

struct Desktop {
    server: Headless,
    renderer: Renderer,
    scheduler: FrameScheduler,
    start: Instant,
}

impl Desktop {
    fn new() -> Self {
        Self {
            server: Headless::new(Size::from(OUTPUT)).expect("headless compositor"),
            renderer: Renderer::new().expect("Vulkan renderer"),
            scheduler: FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH)),
            start: Instant::now(),
        }
    }

    fn open(&mut self, role: Role, as_shell: bool) -> Client {
        let (client_end, server_end) = UnixStream::pair().expect("socket pair");
        if as_shell {
            self.server
                .insert_session_client(server_end, SessionRole::Shell)
                .expect("insert shell client");
        } else {
            self.server
                .insert_client(server_end)
                .expect("insert client");
        }
        let (events, received) = mpsc::channel();
        let (commands, app_commands) = UnixStream::pair().expect("command pipe");
        let thread = thread::spawn(move || {
            let mut window = Window::open_on(client_end, role, WINDOW).expect("window");
            let mut app = Forwarder {
                events,
                commands: app_commands,
                finished: false,
            };
            unbound_toolkit::run(&mut window, &mut app).expect("run loop");
        });
        Client {
            events: received,
            commands,
            thread: Some(thread),
            seen: Vec::new(),
        }
    }

    fn pump(&mut self) {
        self.server.dispatch().expect("server dispatch");
        let start = self.start;
        self.scheduler
            .poll(|| start.elapsed(), &mut self.server, &mut self.renderer)
            .expect("compositor frame");
        thread::sleep(Duration::from_micros(200));
    }

    fn wait_for(
        &mut self,
        clients: &mut [&mut Client],
        mut until_event: impl FnMut(&[WindowEvent]) -> bool,
        mut until_desktop: impl FnMut(&Self) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            self.pump();
            for client in clients.iter_mut() {
                client.seen.extend(client.events.try_iter());
                assert!(
                    client
                        .thread
                        .as_ref()
                        .is_some_and(|thread| !thread.is_finished()),
                    "the client stopped"
                );
            }
            let events_done = clients.iter().any(|client| until_event(&client.seen));
            if events_done && until_desktop(self) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out");
        }
    }

    fn settle(&mut self, clients: &mut [&mut Client]) {
        let until = Instant::now() + Duration::from_millis(100);
        while Instant::now() < until {
            self.pump();
            for client in clients.iter_mut() {
                client.seen.extend(client.events.try_iter());
            }
        }
    }

    fn mapped(&self) -> bool {
        let windows = self.server.windows();
        !windows.is_empty() && windows.iter().all(|window| window.size().is_some())
    }

    fn type_keys(&mut self, keys: &[u32]) {
        for key in keys {
            self.server.key(*key, true, 0);
        }
        for key in keys.iter().rev() {
            self.server.key(*key, false, 0);
        }
    }
}

fn typed(events: &[WindowEvent]) -> Vec<(Key, Modifiers)> {
    events
        .iter()
        .filter_map(|event| match event {
            WindowEvent::Input(Event::Key { key, modifiers }) => Some((*key, *modifiers)),
            _ => None,
        })
        .collect()
}

#[test]
fn typed_keys_arrive_as_characters_with_their_modifiers() {
    let mut desktop = Desktop::new();
    let mut client = desktop.open(Role::toplevel("keys", "test.keys"), false);
    desktop.wait_for(
        &mut [&mut client],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );

    desktop.type_keys(&[KEY_A]);
    desktop.type_keys(&[KEY_LEFTSHIFT, KEY_A]);
    desktop.type_keys(&[KEY_LEFTCTRL, KEY_C]);
    desktop.wait_for(
        &mut [&mut client],
        |events| typed(events).len() >= 3,
        |_| true,
    );

    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    let control = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    assert_eq!(
        typed(&client.seen),
        [
            (Key::Character('a'), Modifiers::default()),
            (Key::Character('A'), shift),
            (Key::Character('c'), control),
        ]
    );
}

#[test]
fn a_held_key_repeats_after_the_compositors_delay() {
    let mut desktop = Desktop::new();
    let mut client = desktop.open(Role::toplevel("repeat", "test.repeat"), false);
    desktop.wait_for(
        &mut [&mut client],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );

    let pressed = Instant::now();
    desktop.server.key(KEY_A, true, 0);
    desktop.wait_for(
        &mut [&mut client],
        |events| typed(events).len() >= 3,
        |_| true,
    );
    let held = pressed.elapsed();
    desktop.server.key(KEY_A, false, 0);

    assert!(
        held >= Duration::from_millis(600),
        "repeated after {held:?}"
    );
    assert!(
        typed(&client.seen)
            .iter()
            .all(|(key, _)| *key == Key::Character('a'))
    );
}

#[test]
fn a_key_released_while_the_app_is_busy_does_not_repeat() {
    let mut desktop = Desktop::new();
    let mut client = desktop.open(Role::toplevel("busy", "test.busy"), false);
    desktop.wait_for(
        &mut [&mut client],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );

    desktop.server.key(KEY_A, true, 0);
    desktop.wait_for(
        &mut [&mut client],
        |events| !typed(events).is_empty(),
        |_| true,
    );
    client.send(b'b');
    desktop.settle(&mut [&mut client]);
    desktop.server.key(KEY_A, false, 0);
    let until = Instant::now() + BUSY + Duration::from_millis(600);
    while Instant::now() < until {
        desktop.settle(&mut [&mut client]);
    }

    assert_eq!(
        typed(&client.seen),
        [(Key::Character('a'), Modifiers::default())]
    );
}

#[test]
fn clicks_arrive_at_surface_coordinates() {
    let mut desktop = Desktop::new();
    let mut client = desktop.open(Role::toplevel("pointer", "test.pointer"), false);
    desktop.wait_for(&mut [&mut client], |_| true, Desktop::mapped);
    let origin = desktop.server.windows()[0].location();

    desktop.server.move_pointer_to(
        Point::from((f64::from(origin.x + 30), f64::from(origin.y + 20))),
        0,
    );
    desktop.server.press_button(BTN_LEFT, true, 0);
    desktop.server.press_button(BTN_LEFT, false, 0);
    desktop.wait_for(
        &mut [&mut client],
        |events| {
            events
                .iter()
                .any(|event| matches!(event, WindowEvent::Input(Event::PointerUp(_))))
        },
        |_| true,
    );

    let clicks = client
        .seen
        .iter()
        .filter_map(|event| match event {
            WindowEvent::Input(Event::PointerDown(at) | Event::PointerUp(at)) => Some((at.x, at.y)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(clicks, [(30.0, 20.0), (30.0, 20.0)]);
}

#[test]
fn text_copied_in_one_window_is_pasted_in_another() {
    let mut desktop = Desktop::new();
    let mut source = desktop.open(Role::toplevel("source", "test.source"), false);
    desktop.wait_for(
        &mut [&mut source],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );
    desktop.type_keys(&[KEY_A]);
    desktop.wait_for(
        &mut [&mut source],
        |events| !typed(events).is_empty(),
        |_| true,
    );
    source.send(b'c');
    desktop.settle(&mut [&mut source]);

    let mut target = desktop.open(Role::toplevel("target", "test.target"), false);
    desktop.wait_for(
        &mut [&mut target],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );
    target.send(b'p');
    desktop.wait_for(
        &mut [&mut source, &mut target],
        |events| {
            events
                .iter()
                .any(|event| matches!(event, WindowEvent::Pasted(_)))
        },
        |_| true,
    );

    assert!(
        target
            .seen
            .contains(&WindowEvent::Pasted("copied from the toolkit".to_owned()))
    );
}

#[test]
fn a_launcher_overlay_takes_the_keyboard_while_shown_and_returns_it_when_hidden() {
    let mut desktop = Desktop::new();
    let mut app = desktop.open(Role::toplevel("app", "test.app"), false);
    desktop.wait_for(
        &mut [&mut app],
        |events| events.contains(&WindowEvent::Focus(true)),
        Desktop::mapped,
    );
    let mut launcher = desktop.open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: 0,
            keyboard: KeyboardFocus::Exclusive,
            namespace: "launcher".to_owned(),
        }),
        true,
    );
    desktop.wait_for(
        &mut [&mut launcher],
        |events| events.contains(&WindowEvent::Focus(true)),
        |_| true,
    );
    desktop.type_keys(&[KEY_A]);
    desktop.wait_for(
        &mut [&mut launcher],
        |events| !typed(events).is_empty(),
        |_| true,
    );

    launcher.send(b'h');
    desktop.wait_for(
        &mut [&mut app],
        |events| {
            events
                .iter()
                .filter(|event| **event == WindowEvent::Focus(true))
                .count()
                == 2
        },
        |_| true,
    );
    desktop.type_keys(&[KEY_LEFTMETA, KEY_SPACE]);
    desktop.wait_for(
        &mut [&mut launcher],
        |events| events.contains(&WindowEvent::ShowLauncher),
        |_| true,
    );

    assert!(typed(&app.seen).is_empty());

    launcher.send(b's');
    let focused_before = launcher
        .seen
        .iter()
        .filter(|event| **event == WindowEvent::Focus(true))
        .count();
    desktop.wait_for(
        &mut [&mut launcher],
        |events| {
            events
                .iter()
                .filter(|event| **event == WindowEvent::Focus(true))
                .count()
                > focused_before
        },
        |_| true,
    );
}
