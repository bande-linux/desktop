use std::io;
use std::os::fd::BorrowedFd;
use std::time::Instant;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use wayland_client::backend::{ReadEventsGuard, WaylandError};

use crate::scene::Scene;
use crate::window::{Window, WindowError, WindowEvent};

pub trait App {
    fn scene(&mut self, window: &mut Window) -> Scene;

    fn event(&mut self, window: &mut Window, event: WindowEvent);

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        Vec::new()
    }

    fn source_ready(&mut self, _window: &mut Window, _index: usize) {}

    fn deadline(&self) -> Option<Instant> {
        None
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) {}

    fn finished(&self) -> bool {
        false
    }
}

fn earliest(deadlines: impl IntoIterator<Item = Option<Instant>>) -> Option<Instant> {
    deadlines.into_iter().flatten().min()
}

fn read_events(guard: ReadEventsGuard) -> Result<(), WindowError> {
    match guard.read() {
        Ok(_) => Ok(()),
        Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn run(window: &mut Window, app: &mut impl App) -> Result<(), WindowError> {
    loop {
        let guard = loop {
            match window.queue.prepare_read() {
                Some(guard) => break guard,
                None => window.dispatch_pending()?,
            }
        };
        read_events(guard)?;
        window.dispatch_pending()?;
        window.fire_repeats(Instant::now());
        for event in window.take_events() {
            app.event(window, event);
        }
        if window.is_closed() || app.finished() {
            return Ok(());
        }
        if window.wants_frame() {
            let scene = app.scene(window);
            window.draw(&scene)?;
        }
        window.flush()?;
        let Some(guard) = window.queue.prepare_read() else {
            continue;
        };

        let deadline = earliest([window.state.input.repeat_deadline(), app.deadline()]);
        let (wayland_ready, pastes_ready, sources_ready) = {
            let connection = guard.connection_fd();
            let pastes = window.paste_fds();
            let sources = app.sources();
            let mut fds = Vec::with_capacity(1 + pastes.len() + sources.len());
            fds.push(PollFd::new(&connection, PollFlags::IN));
            for fd in pastes.iter().chain(&sources) {
                fds.push(PollFd::new(fd, PollFlags::IN));
            }
            let timeout = deadline.map(|deadline| {
                let wait = deadline.saturating_duration_since(Instant::now());
                Timespec {
                    tv_sec: i64::try_from(wait.as_secs()).unwrap_or(i64::MAX),
                    tv_nsec: i64::from(wait.subsec_nanos()),
                }
            });
            match poll(&mut fds, timeout.as_ref()) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(io::Error::from(error).into()),
            }
            let ready = |fd: &PollFd| !fd.revents().is_empty();
            let wayland_ready = ready(&fds[0]);
            let pastes_ready = (0..pastes.len())
                .filter(|index| ready(&fds[1 + index]))
                .collect::<Vec<_>>();
            let sources_ready = (0..sources.len())
                .filter(|index| ready(&fds[1 + pastes.len() + index]))
                .collect::<Vec<_>>();
            (wayland_ready, pastes_ready, sources_ready)
        };
        if wayland_ready {
            read_events(guard)?;
        } else {
            drop(guard);
        }
        for index in pastes_ready.into_iter().rev() {
            window.read_paste(index);
        }
        for index in sources_ready {
            app.source_ready(window, index);
        }
        let now = Instant::now();
        if app.deadline().is_some_and(|deadline| deadline <= now) {
            app.tick(window, now);
        }
    }
}
