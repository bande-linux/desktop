use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use smithay::utils::Size;
use unbound_compositor::{Compositor, FrameClock, FrameScheduler, REFRESH, Renderer, SessionRole};
use unbound_toolkit_platform::{App, Role, Window, WindowEvent};

pub struct Client {
    pub events: Receiver<WindowEvent>,
    pub commands: UnixStream,
    pub thread: Option<JoinHandle<()>>,
    pub seen: Vec<WindowEvent>,
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.commands.shutdown(std::net::Shutdown::Both);
    }
}

pub struct Desktop {
    pub server: Compositor,
    pub renderer: Renderer,
    pub scheduler: FrameScheduler,
    pub start: Instant,
}

impl Desktop {
    pub fn sized(output: (i32, i32)) -> Self {
        Self {
            server: Compositor::new(Size::from(output)).expect("headless compositor"),
            renderer: Renderer::new().expect("Vulkan renderer"),
            scheduler: FrameScheduler::new(FrameClock::new(Duration::ZERO, REFRESH)),
            start: Instant::now(),
        }
    }

    pub fn open_app<A: App>(
        &mut self,
        role: Role,
        as_shell: bool,
        size: (u32, u32),
        app: impl FnOnce(Sender<WindowEvent>, UnixStream) -> A + Send + 'static,
    ) -> Client {
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
            let mut window = Window::open_on(client_end, role, size).expect("window");
            let mut app = app(events, app_commands);
            unbound_toolkit_platform::run(&mut window, &mut app).expect("run loop");
        });
        Client {
            events: received,
            commands,
            thread: Some(thread),
            seen: Vec::new(),
        }
    }

    pub fn pump(&mut self) {
        self.server.dispatch().expect("server dispatch");
        let start = self.start;
        self.scheduler
            .poll(|| start.elapsed(), &mut self.server, &mut self.renderer)
            .expect("compositor frame");
        thread::sleep(Duration::from_micros(200));
    }

    pub fn wait_for(
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
            assert!(
                Instant::now() < deadline,
                "timed out with {} popups after {:?}",
                self.server.popup_count(),
                clients
                    .iter()
                    .map(|client| &client.seen)
                    .collect::<Vec<_>>()
            );
        }
    }

    pub fn settle(&mut self, clients: &mut [&mut Client]) {
        let until = Instant::now() + Duration::from_millis(100);
        while Instant::now() < until {
            self.pump();
            for client in clients.iter_mut() {
                client.seen.extend(client.events.try_iter());
            }
        }
    }

    pub fn mapped(&self) -> bool {
        let windows = self.server.windows();
        !windows.is_empty() && windows.iter().all(|window| window.size().is_some())
    }
}
