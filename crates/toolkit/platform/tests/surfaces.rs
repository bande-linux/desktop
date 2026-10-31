#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use bande_compositor::{Compositor, FrameClock, FrameScheduler, REFRESH, Renderer, SessionRole};
use bande_graphics::{Color, Event, Scene};
use bande_toolkit_platform::{
    Edges, KeyboardFocus, LayerKind, LayerOptions, Role, Surfaces, Window, WindowEvent,
};
use smithay::utils::{Point, Size};

const OUTPUT: (i32, i32) = (320, 240);
const STRIP: u32 = 40;
const KEY_A: u32 = 30;
const BTN_LEFT: u32 = 0x110;

fn strip(anchor: Edges, namespace: &str) -> Role {
    Role::Layer(LayerOptions {
        layer: LayerKind::Top,
        anchor,
        exclusive_zone: 0,
        keyboard: KeyboardFocus::OnDemand,
        namespace: namespace.to_owned(),
    })
}

struct Pair {
    windows: Vec<Window>,
    events: Sender<(usize, WindowEvent)>,
}

impl Surfaces for Pair {
    fn windows(&self) -> &[Window] {
        &self.windows
    }

    fn windows_mut(&mut self) -> &mut [Window] {
        &mut self.windows
    }

    fn scene(&mut self, index: usize) -> Scene {
        let (width, height) = self.windows[index].size();
        Scene {
            width,
            height,
            background: Color::hex(0x3070D0),
            items: Vec::new(),
        }
    }

    fn event(&mut self, index: usize, event: WindowEvent) {
        let _ = self.events.send((index, event));
    }
}

fn open_pair(server: &mut Compositor) -> Receiver<(usize, WindowEvent)> {
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server
        .insert_session_client(server_end, SessionRole::Shell)
        .expect("insert shell client");
    let (events, received) = mpsc::channel();
    thread::spawn(move || {
        let left = Edges {
            left: true,
            top: true,
            bottom: true,
            right: false,
        };
        let top_right = Edges {
            left: false,
            top: true,
            bottom: false,
            right: true,
        };
        let first = Window::open_on(client_end, strip(left, "first"), (STRIP, 0)).expect("first");
        let second = first
            .open_beside(strip(top_right, "second"), (STRIP, STRIP))
            .expect("second");
        let mut pair = Pair {
            windows: vec![first, second],
            events,
        };
        bande_toolkit_platform::run_surfaces(&mut pair).expect("run loop");
    });
    received
}

struct Urgent {
    windows: Vec<Window>,
    socket: TcpStream,
    ready: Sender<(usize, WindowEvent)>,
}

impl Surfaces for Urgent {
    fn windows(&self) -> &[Window] {
        &self.windows
    }

    fn windows_mut(&mut self) -> &mut [Window] {
        &mut self.windows
    }

    fn scene(&mut self, index: usize) -> Scene {
        let (width, height) = self.windows[index].size();
        Scene {
            width,
            height,
            background: Color::hex(0x3070D0),
            items: Vec::new(),
        }
    }

    fn event(&mut self, _index: usize, _event: WindowEvent) {}

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.socket.as_fd()]
    }

    fn source_ready(&mut self, index: usize) {
        let _ = self.ready.send((index, WindowEvent::CloseRequested));
        self.windows[0].close();
    }
}

struct Desktop {
    server: Compositor,
    renderer: Renderer,
    scheduler: FrameScheduler,
    start: Instant,
    seen: Vec<(usize, WindowEvent)>,
}

impl Desktop {
    fn pump(&mut self, events: &Receiver<(usize, WindowEvent)>) {
        self.server.dispatch().expect("server dispatch");
        let start = self.start;
        self.scheduler
            .poll(|| start.elapsed(), &mut self.server, &mut self.renderer)
            .expect("compositor frame");
        self.seen.extend(events.try_iter());
        thread::sleep(Duration::from_micros(200));
    }

    fn wait_for(
        &mut self,
        events: &Receiver<(usize, WindowEvent)>,
        mut until: impl FnMut(&[(usize, WindowEvent)]) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !until(&self.seen) {
            self.pump(events);
            assert!(Instant::now() < deadline, "timed out after {:?}", self.seen);
        }
    }
}

fn pointer_events(seen: &[(usize, WindowEvent)]) -> Vec<(usize, &'static str)> {
    seen.iter()
        .filter_map(|(index, event)| match event {
            WindowEvent::Input(Event::PointerEnter(_)) => Some((*index, "enter")),
            WindowEvent::Input(Event::PointerLeave) => Some((*index, "leave")),
            WindowEvent::Input(Event::PointerDown(_)) => Some((*index, "down")),
            _ => None,
        })
        .collect()
}

#[test]
fn sibling_surfaces_on_one_connection_each_get_only_their_own_input() {
    let mut desktop = Desktop {
        server: Compositor::new(Size::from(OUTPUT)).expect("headless compositor"),
        renderer: Renderer::new().expect("Vulkan renderer"),
        scheduler: FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH)),
        start: Instant::now(),
        seen: Vec::new(),
    };
    let events = open_pair(&mut desktop.server);
    desktop.wait_for(&events, |seen| {
        [0, 1].iter().all(|window| {
            seen.iter().any(|(index, event)| {
                index == window && matches!(event, WindowEvent::AppearanceChanged(_))
            })
        })
    });
    for _ in 0..200 {
        desktop.pump(&events);
    }

    desktop
        .server
        .move_pointer_to(Point::from((10.0, f64::from(OUTPUT.1) / 2.0)), 0);
    desktop.server.press_button(BTN_LEFT, true, 0);
    desktop.server.press_button(BTN_LEFT, false, 0);
    desktop.wait_for(&events, |seen| pointer_events(seen).contains(&(0, "down")));

    desktop.server.move_pointer_to(
        Point::from((f64::from(OUTPUT.0) - 10.0, f64::from(STRIP) / 2.0)),
        0,
    );
    desktop.server.press_button(BTN_LEFT, true, 0);
    desktop.server.press_button(BTN_LEFT, false, 0);
    desktop.wait_for(&events, |seen| pointer_events(seen).contains(&(1, "down")));
    desktop.server.key(KEY_A, true, 0);
    desktop.server.key(KEY_A, false, 0);
    desktop.wait_for(&events, |seen| {
        seen.iter()
            .any(|(_, event)| matches!(event, WindowEvent::Input(Event::Key { .. })))
    });
    for _ in 0..50 {
        desktop.pump(&events);
    }

    assert_eq!(
        pointer_events(&desktop.seen),
        [
            (0, "enter"),
            (0, "down"),
            (0, "leave"),
            (1, "enter"),
            (1, "down")
        ]
    );
    let typed_in: Vec<usize> = desktop
        .seen
        .iter()
        .filter(|(_, event)| matches!(event, WindowEvent::Input(Event::Key { .. })))
        .map(|(index, _)| *index)
        .collect();
    assert_eq!(typed_in, [1]);
}

#[test]
fn a_source_that_signals_only_priority_data_wakes_the_loop() {
    let mut desktop = Desktop {
        server: Compositor::new(Size::from(OUTPUT)).expect("headless compositor"),
        renderer: Renderer::new().expect("Vulkan renderer"),
        scheduler: FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH)),
        start: Instant::now(),
        seen: Vec::new(),
    };
    let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
    let sender = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
    let (socket, _) = listener.accept().expect("accept");
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    desktop
        .server
        .insert_session_client(server_end, SessionRole::Shell)
        .expect("insert shell client");
    let (ready, events) = mpsc::channel();
    thread::spawn(move || {
        let window =
            Window::open_on(client_end, strip(Edges::ALL, "urgent"), (0, 0)).expect("window");
        let mut urgent = Urgent {
            windows: vec![window],
            socket,
            ready,
        };
        bande_toolkit_platform::run_surfaces(&mut urgent).expect("run loop");
    });
    for _ in 0..200 {
        desktop.pump(&events);
    }
    assert!(desktop.seen.is_empty());

    rustix::net::send(&sender, b"u", rustix::net::SendFlags::OOB).expect("urgent byte");
    desktop.wait_for(&events, |seen| !seen.is_empty());

    assert_eq!(desktop.seen, [(0, WindowEvent::CloseRequested)]);
}
